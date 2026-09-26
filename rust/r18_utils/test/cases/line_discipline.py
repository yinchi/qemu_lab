"""Last updated: Stage 12, Step 13b.

One line discipline for both the shell's prompt and a program's `read(0)`: the ordinary behavior
of typing a line at the prompt (Backspace, a blank line, Ctrl+D doing nothing, a line wider than the
row) and of a program reading typed lines (`cat`, `wc`), plus that output longer than the screen
scrolls. Cursor movement, history and Ctrl+D's partial delivery are in `line_editing.py`.

(Step 4 first checked this session against a transcript recorded from r11, to prove its refactor moved
the code without changing behavior. That safety net was retired once its job was done: a test says what
this stage's behavior is, not what an earlier stage's was.)
"""

from harness import BACKSPACE, CTRL_D, text_bands

# A command line longer than the display is wide (80 cells): the input row's window slides.
LONG = "echo " + "y" * 95


def run(ctx):
    s, check = ctx.s, ctx.check

    check("echo", s.run("echo hello world"), "echo hello world\nhello world\n")

    # Editing at the prompt: Backspace pops, on an empty line it does nothing, Ctrl+D does nothing.
    s.type("echo abx")
    s.keys([BACKSPACE])
    s.type("c\n")
    check("prompt: backspace", s.wait_prompt(), "echo abc\nabc\n")
    s.keys([BACKSPACE, BACKSPACE])
    check("prompt: backspace on an empty line", s.run("echo ok"), "echo ok\nok\n")
    check("prompt: blank line", s.run(""), "\n")
    s.type("ec")
    s.keys([CTRL_D])
    s.type("ho x\n")
    check("prompt: ctrl+d does nothing", s.wait_prompt(), "echo x\nx\n")
    check("prompt: line wider than the row", s.run(LONG), LONG + "\n" + "y" * 95 + "\n")

    check("unknown program", s.run("nosuch"), "nosuch\nnosuch: command not found\n")
    check("exit status", s.run_status("false"), ("false\n", 1))
    check("hello", s.run("hello"), "hello\nhello from userspace\n")

    # A program reading typed lines: the same editing, Ctrl+D on an empty line ends it.
    s.type("cat\n")
    s.wait_until(lambda t: t.endswith("cat\n"), "cat to start")
    s.type("hellp")
    s.keys([BACKSPACE])
    s.type("o\n")
    s.wait_until(lambda t: t.endswith("hello\nhello\n"), "cat to echo the line back")
    s.keys([BACKSPACE])  # nothing typed: no effect
    s.type("abc\n")
    s.wait_until(lambda t: t.endswith("abc\nabc\n"), "cat to echo abc")
    s.keys([CTRL_D])
    check("cat: stdin", s.wait_prompt(), "cat\nhello\nhello\nabc\nabc\n")

    s.type("wc\n")
    s.wait_until(lambda t: t.endswith("wc\n"), "wc to start")
    s.type("one two\nthree\n")
    s.wait_until(lambda t: t.endswith("three\n"), "wc to read")
    s.keys([CTRL_D])
    check("wc: stdin", s.wait_prompt(), "wc\none two\nthree\n2 3 14\n")

    check("shell alive", s.run("echo done"), "echo done\ndone\n")

    # --- T4.6: output longer than the screen scrolls, and the prompt lands on the last row ---
    s.run("cat tests/utf8-boundary.txt")
    bands = text_bands(s.screendump_settled())
    rows = 480 // 16
    check("after scrolling output, the prompt is on the last row", bands[-1][0], rows - 1)
    check("...and output fills the rows above it", bands[-2][0], rows - 2)
    check("shell alive after scrolling", s.run("echo ok"), "echo ok\nok\n")
