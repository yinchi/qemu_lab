"""Last updated: Stage 12, Step 4.

One line discipline for both the shell's prompt and a program's `read(0)`.

`golden_session` is one scripted session of prompt editing and stdin reading whose serial transcript
was captured from r11 (`golden/step04_r11.json`, made by `mkgolden.py`). Step 4 moves code without
changing behavior, so r12 must produce the same transcript. The session uses only what r11 and r12 both
have -- programs from `bin/`, no fixtures -- so it can run on either.
"""

import json
import os

from harness import BACKSPACE, CTRL_D, text_bands

GOLDEN = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "golden", "step04_r11.json")

# A command line longer than the display is wide (80 cells): the input row's window slides.
LONG = "echo " + "y" * 95


def golden_session(s):
    """Runs the scripted session on `s` and returns `[[label, transcript], ...]`."""
    out = []

    def rec(label, transcript):
        out.append([label, transcript])

    rec("echo", s.run("echo hello world"))

    # Editing at the prompt: Backspace pops, on an empty line it does nothing, Ctrl+D does nothing.
    s.type("echo abx")
    s.keys([BACKSPACE])
    s.type("c\n")
    rec("prompt: backspace", s.wait_prompt())
    s.keys([BACKSPACE, BACKSPACE])
    rec("prompt: backspace on an empty line", s.run("echo ok"))
    rec("prompt: blank line", s.run(""))
    s.type("ec")
    s.keys([CTRL_D])
    s.type("ho x\n")
    rec("prompt: ctrl+d does nothing", s.wait_prompt())
    rec("prompt: line wider than the row", s.run(LONG))

    rec("unknown program", s.run("nosuch"))
    rec("exit status", s.run("false"))
    rec("hello", s.run("hello"))

    # A program reading typed lines: the same editing, Ctrl+D on an empty line ends it.
    s.type("cat\n")
    s.wait_until(lambda t: t.endswith("cat\n"), "cat to start")
    s.type("hellp")
    s.keys([BACKSPACE])
    s.type("o\n")
    s.wait_until(lambda t: t.endswith("hello\nhello\n"), "cat to echo the line back")
    s.keys([BACKSPACE])  # nothing typed: no effect
    # Not Ctrl+D-on-a-non-empty-line here: r11 ignored it, r12 doesn't (Step 12's POSIX partial-delivery
    # refinement, tested on its own in `line_editing.py`) -- typing the line whole keeps this shared
    # script's transcript identical on both, matching the r11-captured golden data below.
    s.type("abc\n")
    s.wait_until(lambda t: t.endswith("abc\nabc\n"), "cat to echo abc")
    s.keys([CTRL_D])
    rec("cat: stdin", s.wait_prompt())

    s.type("wc\n")
    s.wait_until(lambda t: t.endswith("wc\n"), "wc to start")
    s.type("one two\nthree\n")
    s.wait_until(lambda t: t.endswith("three\n"), "wc to read")
    s.keys([CTRL_D])
    rec("wc: stdin", s.wait_prompt())

    rec("shell alive", s.run("echo done"))
    return out


def run(ctx):
    s, check = ctx.s, ctx.check
    with open(GOLDEN) as f:
        golden = json.load(f)
    for (label, got), (want_label, want) in zip(golden_session(s), golden):
        assert label == want_label, (label, want_label)
        # The one deliberate difference from r11: Step 7 switched the launcher's messages to bash's wording.
        want = want.replace(": not found\n", ": command not found\n")
        check(f"golden (r11): {label}", got, want)

    # --- T4.6: output longer than the screen scrolls, and the prompt lands on the last row ---
    s.run("cat tests/utf8-boundary.txt")
    bands = text_bands(s.screendump_settled())
    rows = 480 // 16
    check("after scrolling output, the prompt is on the last row", bands[-1][0], rows - 1)
    check("...and output fills the rows above it", bands[-2][0], rows - 2)
    check("shell alive after scrolling", s.run("echo ok"), "echo ok\nok\n")
