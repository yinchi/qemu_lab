#!/usr/bin/env python3
"""Boots r12_shell headless and runs every test case module in `cases/` against it, then checks what
ended up on the disk once QEMU is gone: each module's `verify_disk`, and finally `fsck.fat -n` on the
image.

Usage: run_tests.py <kernel.elf> <disk.img>   (normally via `just test-qemu`)

Modules run in `GROUPS` -- each its own QEMU session and its own private copy of the disk image. Every
module is fully self-sufficient given nothing but a fresh boot (chmods whatever exec-bit fixtures it
uses itself, never assumes the working directory left by some other module), so each currently gets a
group of its own: one module, one QEMU instance. `run_group` still takes a *list* of modules, not one
module, so a future module that does need to depend on an earlier one (a chmod'd exec bit, a working
directory) can share a group with it -- add it to that module's list, in order, rather than adding a
new one.

Groups run at up to `worker_count()` at a time -- the host's CPU count minus one, so twelve groups on a
two-core box queue in batches instead of trying to run all twelve at once and starving the machine, and
one core stays free for everything else running on it. `ThreadPoolExecutor` queues the rest itself and
starts each one the moment a slot frees up, so this needs no manual batching (a fixed "7 then 5" split
would leave workers idle near the end of the first batch, waiting on its one slowest group, instead of
already starting from the queue). Each group's PASS/FAIL lines are collected into a list rather than
printed as they happen (`run_group`'s `results`), so that a slower group finishing later -- whether from
being genuinely slower or just queued behind another -- can't interleave its output with a faster one's:
`main` prints every group's list only once every group has finished, in `GROUPS`' own order, so the
transcript always reads the same regardless of which group actually finished first or how many ran at once.

A case module exposes `run(ctx)` (drives the shell) and optionally `verify_disk(ctx)` (runs after QEMU
has exited). Later Steps of `Stage12.md` add modules here, one per area -- each in its own new group,
unless it genuinely needs an earlier module's leftover state, per the paragraph above.
"""

import concurrent.futures
import os
import shutil
import subprocess
import sys
import tempfile

from cases import core_utils, launch, console, unicode, stack, line_discipline, wrapped_input, token_queue, cwd, syntax, redirection, scripts
from harness import Context, Session

GROUPS = [
    [core_utils],
    [launch],
    [console],
    [unicode],
    [stack],
    [line_discipline],
    [wrapped_input],
    [token_queue],
    [cwd],
    [syntax],
    [redirection],
    [scripts],
]


def worker_count():
    """How many groups to run at once: the CPUs actually available to this process, minus one left
    free for the rest of the system -- `sched_getaffinity` (Linux) counts only those, respecting a
    container/cgroup CPU limit, not just the physical core count `cpu_count` reports; anywhere that
    call doesn't exist, `cpu_count` (or a conservative default, if even that is unavailable) stands in.
    Never less than 1, so this still runs somewhere with a single CPU."""
    try:
        cpus = len(os.sched_getaffinity(0))
    except AttributeError:
        cpus = os.cpu_count() or 4
    return max(1, cpus - 1)


def run_group(elf, orig_img, disk_dir, modules):
    """Runs one group's modules against their own private copy of the disk image, in one QEMU
    session. Returns the list of `(name, got, want)` triples `check` collected, in the order they
    happened -- not printed here, so `main` can print every group in a fixed order once all of them
    are done, regardless of which one finishes first."""
    label = "+".join(m.__name__.rsplit(".", 1)[-1] for m in modules)
    workdir = tempfile.mkdtemp(prefix=f"r12-{label}-")
    img = os.path.join(workdir, "disk.img")
    subprocess.run(["cp", "--sparse=always", orig_img, img], check=True)

    results = []

    def check(name, got, want):
        results.append((name, got, want))

    s = Session(elf, img, workdir)
    ctx = Context(s, check, disk_dir, img, workdir)
    try:
        for case in modules:
            case.run(ctx)
    finally:
        s.close()

    # QEMU is gone, so this group's image is quiescent.
    for case in modules:
        if hasattr(case, "verify_disk"):
            case.verify_disk(ctx)
    fsck = subprocess.run(["fsck.fat", "-n", img], capture_output=True, text=True)
    check(
        f"disk ({label}): fsck.fat -n is clean",
        (fsck.returncode, fsck.stdout + fsck.stderr) if fsck.returncode else 0,
        0,
    )

    shutil.rmtree(workdir, ignore_errors=True)
    return results


def main():
    elf, orig_img = os.path.abspath(sys.argv[1]), os.path.abspath(sys.argv[2])
    here = os.path.dirname(os.path.abspath(__file__))
    disk_dir = os.path.join(os.path.dirname(here), "disk")

    workers = min(len(GROUPS), worker_count())
    print(f"running {len(GROUPS)} groups, {workers} at a time")
    with concurrent.futures.ThreadPoolExecutor(max_workers=workers) as pool:
        futures = [pool.submit(run_group, elf, orig_img, disk_dir, modules) for modules in GROUPS]
        # `.result()` on each future in turn: this blocks on group 0 even if group 1 finishes first,
        # so the two groups' results are gathered -- and about to be printed -- in `GROUPS`' own
        # order every time, never in whichever order the sessions happened to finish.
        all_results = [f.result() for f in futures]

    failures = []
    for results in all_results:
        for name, got, want in results:
            if got == want:
                print(f"PASS  {name}")
            else:
                print(f"FAIL  {name}\n  want: {want!r}\n  got:  {got!r}")
                failures.append(name)

    if failures:
        print(f"\n{len(failures)} failed: {', '.join(failures)}")
        return 1
    print("\nall passed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
