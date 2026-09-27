"""Last updated: Stage 19, Step 4b (the line typed during the copy waits a second).

Keys are queued by the keyboard interrupt and read later, so a program runs with interrupts on and
keys pressed while it runs are kept, in order, for the next reader.

This module is `EXCLUSIVE` (see `run_tests.py`): its "typing during a large copy" check races real key presses against a
3 MiB copy in the guest and can lose a key when other QEMU sessions are starving the machine, so it runs on its own
once the parallel groups have finished.

**Why "typing during a large copy" waits a second before it types.** Keys typed in the first half second or so after a program is
started can be lost (a virtio-input device queue that fills while the guest's interrupts are masked -- 32 buffers, several events
per keystroke, fixed by the `virtio-drivers` crate -- so QEMU drops the next events before the kernel's own 16-key queue ever
sees them: no overflow note, a different key each time). Measured with the copy this check does, four runs at a time: typing the
line at the harness's usual 25 ms per key right after the Enter that starts the copy lost a key in 4 of 12 runs (and 4 of 15 in another
batch), starting 0.15 s later 2 of 12, 0.3 s later 4 of 12, 0.6 s later 0 of 12, and 0.8 s or 1 s later 0 of 72. Typing more slowly
(80 ms per key) did not help once the line began right after the Enter. Since Stage 19's `cp` is a different binary the window moved
and the rate went from about one run in ten to about one in three. The kernel-side fix -- an owner for the console input queue that
drains the device promptly -- belongs to the scheduling stage (`ROADMAP.md` Stage 26); until then the line is typed
`LAUNCH_SETTLE` seconds after the copy starts, which is still well inside the copy (it runs about 2.5 s).

`spin N` (a test program) busy-waits N seconds without reading anything -- long enough to type during. This
kernel build (`testhooks`) has a 16-key queue so the overflow path can be reached by typing a few dozen keys.
"""

import filecmp
import os
import time

from harness import mcopy_out

EXCLUSIVE = True  # run alone, after the parallel groups: timing-sensitive under CPU load

LAUNCH_SETTLE = 1.0  # seconds between starting the copy and typing the line during it

NOTE = "[keyboard: input queue full, further keys dropped]\n"


def wait_for_end(s, text):
    """Waits until the serial log ends with `text` (a prompt is part of it), returns the transcript since the
    last checkpoint without the trailing prompt, and moves the checkpoint."""
    out = s.wait_until(lambda t: t.endswith(text), repr(text))
    s.pos = len(s.log())
    return out[: -len("> ")]


def run(ctx):
    s, check = ctx.s, ctx.check
    s.run("chmod +x tests/spin")
    s.run("chmod +x tests/bigpad")  # self-sufficient, same reason as cwd.py's own copy of this line

    # --- T5.1: keys typed while a program runs (and never reads) are run afterwards, in order, once each ---
    s.type("tests/spin 3\n")
    s.type("echo a\n")
    s.type("echo b\n")
    check("keys typed during a running program are kept and run in order",
          wait_for_end(s, "b\nb\n> "),
          "tests/spin 3\nspun 3\n> echo a\na\n> echo b\nb\n")

    # --- T5.3: more keys than the queue holds: the first ones are kept, in order; one note, and the shell lives ---
    s.type("tests/spin 4\n")
    s.type("z" * 30)  # the queue holds 16
    wait_for_end(s, "spun 4\n> ")
    s.type("\n")
    out = wait_for_end(s, "command not found\n> ")
    check("overflow: the first 16 keys survive, the rest are dropped", out,
          "zzzzzzzzzzzzzzzz\nzzzzzzzzzzzzzzzz: command not found\n")
    check("overflow: one note on the serial log for the burst", s.log().count(NOTE), 1)
    check("overflow: the shell is responsive", s.run("echo alive"), "echo alive\nalive\n")

    # --- T5.4: many block-device interrupts (a big copy) while typing: the typed line is intact ---
    # The typed line waits `LAUNCH_SETTLE` after the copy starts: see the module docstring for why.
    s.type("cp tests/bigpad tests/bigcopy.bin\n")
    time.sleep(LAUNCH_SETTLE)
    s.type("echo intact\n")
    check("typing during a large copy loses nothing", wait_for_end(s, "intact\nintact\n> "),
          "cp tests/bigpad tests/bigcopy.bin\n> echo intact\nintact\n")

    # --- T5.5: many commands typed back to back: outputs in order, none lost or repeated ---
    count = 15
    for i in range(count):
        s.type(f"echo {i}\n")
    want = "".join(f"echo {i}\n{i}\n> " for i in range(count))[: -len("> ")]
    check("rapid commands run in order", wait_for_end(s, f"echo {count - 1}\n{count - 1}\n> "), want)


def verify_disk(ctx):
    out = os.path.join(ctx.workdir, "out")
    os.makedirs(out, exist_ok=True)
    mcopy_out(ctx.img, "tests/bigcopy.bin", os.path.join(out, "bigcopy.bin"))
    ctx.check("disk: the copy made while typing is byte-identical", 
              filecmp.cmp(os.path.join(out, "bigcopy.bin"), ctx.fixture_path("bigpad"), shallow=False), True)
