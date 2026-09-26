"""Last updated: Stage 12, Step 2.

The console draws Unicode with Unifont -- wide glyphs take two cells, invalid bytes and characters
the font lacks draw U+FFFD, zero-width code points draw nothing, and Backspace moves over a whole
character. Compares what is drawn on the display, cell row against cell row.

The fixtures are UTF-8 files under `/tests/` (`unicode-mix.txt` is deliberately not all valid UTF-8).
"""

from harness import CELL_H, CELL_W, text_bands

COLS = 80  # 640x480 display, 8x16 cells


def cell_blank(band, cell, width=1):
    """Whether `width` cells of a cell row starting at cell `cell` are all background."""
    bg = band[0][0]
    return all(px == bg for line in band for px in line[cell * CELL_W:(cell + width) * CELL_W])


def run(ctx):
    s, check = ctx.s, ctx.check
    s.run("chmod +x tests/probe")
    width, _height, _rows = s.screendump_settled()
    check("display is 80 columns wide (the fixtures assume it)", width // CELL_W, COLS)

    # --- a wide glyph is two cells: 日本語 fills exactly six ---
    check("cat cjk.txt: serial carries the UTF-8", s.run("cat tests/cjk.txt"), "cat tests/cjk.txt\n日本語\n")
    band = text_bands(s.screendump_settled())[-2][1]
    check("cjk: six cells drawn", [cell_blank(band, c) for c in range(8)],
          [False] * 6 + [True, True])

    # --- a wide glyph that would start in the last column wraps whole ---
    s.run("cat tests/wide-wrap.txt")
    bands = text_bands(s.screendump_settled())
    check("wide-wrap: the last cell of the first row is left blank", cell_blank(bands[-3][1], COLS - 1), True)
    check("wide-wrap: the glyph starts the next row", [cell_blank(bands[-2][1], c) for c in range(3)],
          [False, False, True])

    # --- what draws what ---
    s.run("cat tests/unicode-mix.txt")
    lines = [band for _row, band in text_bands(s.screendump_settled())[-8:-1]]
    e_acute, fffd, emoji, bad_byte, control, with_zwj, plain = lines
    check("a real character draws its own glyph", e_acute != fffd, True)
    check("emoji (no glyph) draws U+FFFD", emoji, fffd)
    check("an invalid byte draws U+FFFD", bad_byte, fffd)
    check("an uninterpreted control character draws U+FFFD", control, fffd)
    check("a zero-width joiner draws nothing", with_zwj, plain)

    # --- Backspace moves back a whole character: X lands on the wide glyph's left cell ---
    check("probe bs-wide output", s.run("tests/probe bs-wide"), "tests/probe bs-wide\n日\bX\n")
    after_bs = text_bands(s.screendump_settled())[-2][1]
    s.run("echo X")
    plain_x = text_bands(s.screendump_settled())[-2][1]
    check("backspace over a wide glyph, then X: same as X alone", after_bs, plain_x)

    # --- wrapping follows xterm: a full row waits for the next glyph, so a newline after it leaves no blank row ---
    s.run("cat tests/full-row.txt")
    bands = text_bands(s.screendump_settled())
    rows = [row for row, _band in bands[-3:]]
    check("full row then newline: the next line is on the very next row", rows[1] - rows[0], 1)
    check("...and the prompt on the row after that", rows[2] - rows[1], 1)
    check("the full row really is full", cell_blank(bands[-3][1], COLS - 1), False)

    # A space is printable like any glyph: it goes in column 0 of the next row, as in every terminal.
    s.run("cat tests/full-row-space.txt")
    bands = text_bands(s.screendump_settled())
    after = bands[-2][1]
    check("a space after a full row lands in column 0 of the next row",
          [cell_blank(after, c) for c in range(3)], [True, False, False])
    check("...on the row right below", bands[-2][0] - bands[-3][0], 1)

    # A typed line that fills the row exactly (the prompt is two cells): no blank row after Enter either.
    filler = "x" * (COLS - 2 - len("echo "))
    check("a full-width command line runs", s.run("echo " + filler), f"echo {filler}\n{filler}\n")
    bands = text_bands(s.screendump_settled())
    rows = [row for row, _band in bands[-3:]]
    check("full-width command: output on the next row, prompt on the one after",
          (rows[1] - rows[0], rows[2] - rows[1]), (1, 1))
