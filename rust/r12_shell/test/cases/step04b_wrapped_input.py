"""Step 4b of `Stage12.md`: a typed line longer than the row wraps onto the rows below instead of scrolling
sideways, at the prompt and in a program's `read(0)`, and the screen scrolls with it at the bottom.

The display is 80x30 cells. Lines are typed without Enter first so the screen can be inspected while the
line is still being edited.
"""

from harness import BACKSPACE, CTRL_D, CELL_W, text_bands

COLS, ROWS = 80, 30


def cell_blank(band, cell):
    bg = band[0][0]
    return all(line[cell * CELL_W + x] == bg for line in band for x in range(CELL_W))


def run(ctx):
    s, check = ctx.s, ctx.check

    # Put the prompt on the bottom row, so the long line below has to scroll the screen.
    s.run("cat tests/utf8-boundary.txt")
    bands = text_bands(s.screendump())
    check("the prompt starts on the last row", bands[-1][0], ROWS - 1)

    # --- a 202-cell line (prompt + `echo ` + 195 characters): 80 + 80 + 42 cells, three rows ---
    typed = "z" * 195
    s.type("echo " + typed)
    bands = text_bands(s.screendump())
    rows = [row for row, _ in bands[-3:]]
    check("a long line wraps onto three consecutive rows, scrolling the screen", rows, [ROWS - 3, ROWS - 2, ROWS - 1])
    first, second, third = (band for _, band in bands[-3:])
    check("the start of the line (the prompt) is still visible", cell_blank(first, 0), False)
    check("the first two rows are full", (cell_blank(first, COLS - 1), cell_blank(second, COLS - 1)), (False, False))
    check("the last row holds the remaining 42 cells",
          (cell_blank(third, 41), cell_blank(third, 42)), (False, True))

    # --- Backspace back across the row boundary: the rows the line no longer needs are cleared ---
    s.keys([BACKSPACE] * 125)  # 202 - 125 = 77 cells: one row
    bands = text_bands(s.screendump())
    check("a shortened line leaves no stale rows behind", bands[-1][0], ROWS - 3)
    check("...and is one row", cell_blank(bands[-1][1], 76), False)

    # --- the line still runs, and its transcript is just the finished line ---
    kept = "z" * 70
    s.type("\n")
    check("the wrapped-then-shortened line runs", s.wait_prompt(), f"echo {kept}\n{kept}\n")

    # --- a program reading a line: canonical mode wraps the same way (no prefix: 100 cells = 80 + 20) ---
    s.type("cat\n")
    s.wait_until(lambda t: t.endswith("cat\n"), "cat to start")
    line = "q" * 100
    s.type(line)
    bands = text_bands(s.screendump())
    rows = [row for row, _ in bands[-2:]]
    check("read(0): a 100-character line takes two consecutive rows", rows[1] - rows[0], 1)
    check("read(0): the first row is full and starts with the first character",
          (cell_blank(bands[-2][1], 0), cell_blank(bands[-2][1], COLS - 1)), (False, False))
    s.type("\n")
    s.wait_until(lambda t: t.endswith(f"{line}\n{line}\n"), "cat to echo the line back")
    s.keys([CTRL_D])
    check("read(0): the line reaches the program intact", s.wait_prompt(), f"cat\n{line}\n{line}\n")
    check("shell alive after all that", s.run("echo ok"), "echo ok\nok\n")
