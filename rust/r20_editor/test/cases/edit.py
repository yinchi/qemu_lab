"""Last updated: Stage 20, Step 5.

`edit FILE`: opening a file (or starting a new buffer for a missing one), viewing it full-screen,
scrolling and moving the cursor, and quitting with `^X`. No typing, no save, no status bar yet
(Step 6) -- the whole screen is the text area, and `^X` always exits at once.

`edit` takes over the console entirely (no shell prompt while it runs), so a case types the launch
command, then sends further keys directly with `s.keys(...)` -- the same pattern the `CONSOLE_READ_KEY`
cases in `console.py` use -- until `^X` returns the shell's own prompt.

The cursor is a single inverse-video cell (`ATTR_INVERSE`): a normal cell is mostly the background
colour with sparse foreground ink, an inverted one mostly foreground. `is_inverse` below tells them
apart by which colour is the majority of the cell's 8x16 pixels, not by matching a specific glyph --
so it works regardless of which character is under the cursor.

Fixtures: `tests/hello.txt` (twelve short lines, "one".."twelve", already used elsewhere) fits the
whole 30-row screen at once; `tests/edit-lines.txt` (fifty lines, "L01".."L50", made for this stage)
does not, for the scrolling checks.
"""

from harness import CELL_H, CELL_W, text_bands

FG = (0x55, 0xFF, 0x55)
BG = (0x00, 0x00, 0x00)


def is_inverse(dump, row, col):
    """Whether the cell at (row, col) is drawn inverse (mostly foreground-coloured)."""
    _width, _height, pixels = dump
    cell = [pixels[row * CELL_H + dy][col * CELL_W + dx] for dy in range(CELL_H) for dx in range(CELL_W)]
    return cell.count(FG) > cell.count(BG)


def find_cursor(dump, rows=30, cols=80):
    """The (row, col) of the one inverse cell on screen, or None if there isn't one."""
    for row in range(rows):
        for col in range(cols):
            if is_inverse(dump, row, col):
                return (row, col)
    return None


def open_editor(s, path):
    """Launches `edit PATH` and returns its first settled frame. There is no serial-side signal for
    "the first frame is drawn" (unlike `s.run`, which waits for the shell's own prompt): `edit`
    writes nothing on a successful launch, so the only signal available is the display itself. A
    longer-than-default stability window makes catching a stray stable moment of the *shell's* own
    display, before the program has actually produced its first frame, much less likely under load
    -- without eliminating it; the same class of timing gap `token_queue`'s already-documented flake
    comes from (no scheduler yet to synchronize on), just in a new place."""
    s.type(f"edit {path}\n")
    return s.screendump_settled(stable_for=0.5)


def run(ctx):
    s, check = ctx.s, ctx.check

    # --- opening an existing file shows it, cursor at the very start ---
    dump = open_editor(s, "tests/hello.txt")
    check("edit: the cursor starts at (0, 0)", is_inverse(dump, 0, 0), True)
    check("edit: the cell right of it is not inverted", is_inverse(dump, 0, 1), False)

    # Move off row 0 (its cursor was inverting the cell that would otherwise be checked) and
    # confirm "one" is actually drawn there, not left blank.
    s.keys(["down"])
    rows_shown = [row for row, _band in text_bands(s.screendump_settled())]
    check("edit: all twelve lines of hello.txt are drawn", rows_shown, list(range(12)))
    s.keys(["ctrl-x"])
    s.wait_prompt()

    # --- Right moves the cursor; Home/End go to the logical line's start/end ---
    open_editor(s, "tests/hello.txt")
    s.keys(["right", "right"])
    dump = s.screendump_settled()
    check("edit: Right moved the cursor off (0,0)", is_inverse(dump, 0, 0), False)
    check("edit: ...to the new column (2)", is_inverse(dump, 0, 2), True)
    s.keys(["end"])
    dump = s.screendump_settled()
    check("edit: End reaches the end of 'one' (column 3)", is_inverse(dump, 0, 3), True)
    s.keys(["home"])
    dump = s.screendump_settled()
    check("edit: Home returns to column 0", is_inverse(dump, 0, 0), True)
    s.keys(["ctrl-x"])
    s.wait_prompt()

    # --- Down/Up move by line, remembering the display column across a same-length line ---
    open_editor(s, "tests/hello.txt")
    s.keys(["right", "right", "right", "down"])  # "one"'s column 3 (its own end) -> "two" (also ends at 3)
    dump = s.screendump_settled()
    check("edit: Down onto an equally short line keeps the column", is_inverse(dump, 1, 3), True)
    s.keys(["down"])  # "three": long enough that column 3 is a real, mid-line position
    dump = s.screendump_settled()
    check("edit: the remembered column survives a longer line too", is_inverse(dump, 2, 3), True)
    s.keys(["up", "up"])
    dump = s.screendump_settled()
    check("edit: Up returns to the same remembered column", is_inverse(dump, 0, 3), True)
    s.keys(["ctrl-x"])
    s.wait_prompt()

    # --- a missing file starts a new, empty buffer -- Step 5 has no save, so it stays missing ---
    check("edit: the file doesn't exist yet", s.run("cat tests/brand-new.txt"),
          "cat tests/brand-new.txt\ncat: tests/brand-new.txt: No such file or directory\n")
    dump = open_editor(s, "tests/brand-new.txt")
    check("edit: a missing file is a new, empty buffer -- cursor at (0,0)", is_inverse(dump, 0, 0), True)
    check("edit: nothing else on screen is inverted", is_inverse(dump, 1, 0), False)
    s.keys(["ctrl-x"])
    s.wait_prompt()
    check("edit: still doesn't exist -- Step 5 has no save", s.run("cat tests/brand-new.txt"),
          "cat tests/brand-new.txt\ncat: tests/brand-new.txt: No such file or directory\n")

    # --- scrolling: a file taller than the screen (edit-lines.txt, 50 lines "L01".."L50") ---
    dump = open_editor(s, "tests/edit-lines.txt")
    check("edit: starts scrolled to the top", is_inverse(dump, 0, 0), True)

    s.keys(["alt-slash"])  # Alt+/: last line
    dump = s.screendump_settled()
    check("edit: Alt+/ moves the cursor off the top row", find_cursor(dump)[0], 29)

    s.keys(["alt-backslash"])  # Alt+\: first line
    dump = s.screendump_settled()
    check("edit: Alt+\\ returns to the very first line", is_inverse(dump, 0, 0), True)

    s.keys(["pgdn"])
    dump = s.screendump_settled()
    check("edit: PgDn moves the cursor off row 0", find_cursor(dump)[0] != 0, True)
    s.keys(["pgup"])
    dump = s.screendump_settled()
    check("edit: PgUp returns to the very first line", is_inverse(dump, 0, 0), True)

    s.keys(["ctrl-x"])
    s.wait_prompt()

    check("shell alive after edit", s.run("echo alive"), "echo alive\nalive\n")

    # --- Stage 20, Step 5b: the console's own "alternate screen" -- quitting restores the screen ---
    # `edit` draws a full screen of its own (twelve lines of hello.txt); once it exits, none of that
    # should still be visible. Checked by *row count*, not exact row positions or pixels: by this
    # point in the session the console has already scrolled some number of times, so which absolute
    # rows are in use depends on exactly when a screendump happens to be taken, but the count is
    # simple arithmetic either way. Right before typing the launch command, the console shows some
    # number of non-blank rows; typing it adds exactly one more (the echoed command line itself,
    # counted the moment before `edit` draws its own first frame -- the very thing its snapshot
    # captures and its exit restores). If restoring genuinely brings that back, and nothing of
    # `edit`'s own drawing survives, the count after quitting is that same "one more," not twelve-plus
    # more for hello.txt's own lines.
    before_count = len(text_bands(s.screendump_settled()))
    s.type("edit tests/hello.txt\n")
    s.screendump_settled()  # let it actually draw its first frame before doing anything else
    s.keys(["down", "right", "right"])  # move around; Step 5b restores whatever state edit ends in
    s.keys(["ctrl-x"])
    s.wait_prompt()
    after_count = len(text_bands(s.screendump_settled()))
    check("edit: quitting restores exactly one more row than before launching it",
          after_count, before_count + 1)
