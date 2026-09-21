#!/usr/bin/env python3
"""Boots r12_shell headless and runs every test case module in `cases/` against it, in order, in one
QEMU session (see `harness.py`), then checks what ended up on the disk once QEMU is gone: each
module's `verify_disk`, and finally `fsck.fat -n` on the image.

Usage: run_tests.py <kernel.elf> <disk.img>   (normally via `just test-qemu`)

The disk image is copied first (sparsely -- it is 64 MiB), so the built one is never modified.

A case module exposes `run(ctx)` (drives the shell) and optionally `verify_disk(ctx)` (runs after QEMU
has exited). Later Steps of `Stage12.md` add modules here, one per area.
"""

import os
import shutil
import subprocess
import sys
import tempfile

from cases import core_utils, step01_launch, step02_console, step02b_unicode, step03_stack, step04_line_discipline, step04b_wrapped_input
from harness import Context, Session

CASES = [core_utils, step01_launch, step02_console, step02b_unicode, step03_stack, step04_line_discipline, step04b_wrapped_input]


def main():
    elf, orig_img = os.path.abspath(sys.argv[1]), os.path.abspath(sys.argv[2])
    here = os.path.dirname(os.path.abspath(__file__))
    disk_dir = os.path.join(os.path.dirname(here), "disk")

    workdir = tempfile.mkdtemp(prefix="r12-")
    img = os.path.join(workdir, "disk.img")
    subprocess.run(["cp", "--sparse=always", orig_img, img], check=True)

    failures = []

    def check(name, got, want):
        if got == want:
            print(f"PASS  {name}")
        else:
            print(f"FAIL  {name}\n  want: {want!r}\n  got:  {got!r}")
            failures.append(name)

    s = Session(elf, img, workdir)
    ctx = Context(s, check, disk_dir, img, workdir)
    try:
        for case in CASES:
            case.run(ctx)
    finally:
        s.close()

    # QEMU is gone, so the image is quiescent.
    for case in CASES:
        if hasattr(case, "verify_disk"):
            case.verify_disk(ctx)
    fsck = subprocess.run(["fsck.fat", "-n", img], capture_output=True, text=True)
    check("disk: fsck.fat -n is clean", (fsck.returncode, fsck.stdout + fsck.stderr) if fsck.returncode else 0, 0)

    shutil.rmtree(workdir, ignore_errors=True)
    if failures:
        print(f"\n{len(failures)} failed: {', '.join(failures)}")
        return 1
    print("\nall passed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
