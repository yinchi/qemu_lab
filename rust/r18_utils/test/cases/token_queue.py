"""Last updated: Stage 12, Step 5.

Keys are queued by the keyboard interrupt and read later, so a program runs with interrupts on and
keys pressed while it runs are kept, in order, for the next reader.

`spin N` (a test program) busy-waits N seconds without reading anything -- long enough to type during. This
kernel build (`testhooks`) has a 16-key queue so the overflow path can be reached by typing a few dozen keys.
"""

import filecmp
import os

from harness import mcopy_out

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
    s.type("cp tests/bigpad tests/bigcopy.bin\n")
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
