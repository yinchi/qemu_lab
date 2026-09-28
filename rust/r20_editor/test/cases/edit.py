"""Last updated: Stage 20, Step 8.

`edit FILE`: typing and deleting, `^S` save and `^O` save-as, the status bar, `^X` exit (asking first
if modified), `^G`'s help screen, `^C`'s cursor-position message, `.editrc`, line cut/copy/paste
(`^K`/`Alt+6`/`^U`), forward search (`^W`, `Alt+W` for find next), go to line (`^_`/`Alt+G`), the mark
(`Alt+A`; with one set, `^K`/`Alt+6`/`^U` act on the region instead of the whole line), the
line-number gutter (`Alt+N`) and the auto-indent toggle (`Alt+I`). Since Step 6 the text area is only
`rows - 2` rows: the last two are reserved for the inverse-video status bar (`STATUS_ROW`) and, below
it, the help footer / a transient one-frame message / the one-line prompt widget (`FOOTER_ROW`),
whichever is current.

`edit` takes over the console entirely (no shell prompt while it runs), so a case types the launch
command, then sends further keys directly with `s.keys(...)` -- the same pattern the `CONSOLE_READ_KEY`
cases in `console.py` use -- until `^X` returns the shell's own prompt.

Two different kinds of check are used, for two different kinds of thing:
- **What ends up on disk** (typed text, Backspace/Delete, Enter/split, Tab, save, save-as) is checked
  through the serial log, by `cat`-ing the file after `edit` exits -- exact and simple, unlike
  matching pixels.
- **Screen-only state** (the status bar's own content, the help screen, the save-confirmation prompt,
  a one-frame message) is checked structurally: whether the status bar's row is entirely inverse (it
  always is, outside `^G`'s help screen, which draws no inverse cell at all and so is easy to tell
  apart from every other mode this way), whether a row's pixels changed at all between two frames, and
  -- for a few plain, punctuation-free strings worth pinning down exactly -- by comparing a row's
  pixels against a fresh reference of the same text printed the ordinary way (`echo`), the same
  technique `console.py` uses for its crash message.

The cursor is a single inverse-video cell (`ATTR_INVERSE`): a normal cell is mostly the background
colour with sparse foreground ink, an inverted one mostly foreground. `is_inverse` below tells them
apart by which colour is the majority of the cell's 8x16 pixels, not by matching a specific glyph --
so it works regardless of which character is under the cursor.

Fixtures: `tests/hello.txt` (twelve short lines, "one".."twelve", already used elsewhere) fits the
whole text area at once; `tests/edit-lines.txt` (fifty lines, "L01".."L50") does not, for the
scrolling checks. Everything Step 6 needs to read back (typed text, saved files) is created by `edit`
itself, on file names that don't otherwise exist.
"""

from harness import CELL_H, CELL_W, KEY_NAMES, text_bands

FG = (0x55, 0xFF, 0x55)
BG = (0x00, 0x00, 0x00)
DIM_FG = (0x2A, 0x7F, 0x2A)

ROWS, COLS = 30, 80
STATUS_ROW = ROWS - 2  # 28: the inverse-video status bar
FOOTER_ROW = ROWS - 1  # 29: help footer / message / prompt


def is_inverse(dump, row, col):
    """Whether the cell at (row, col) is drawn inverse (mostly foreground-coloured)."""
    _width, _height, pixels = dump
    cell = [pixels[row * CELL_H + dy][col * CELL_W + dx] for dy in range(CELL_H) for dx in range(CELL_W)]
    return cell.count(FG) > cell.count(BG)


def is_cursor(dump, row, col):
    """Whether (row, col) is drawn as *a* cursor: plain inverse (`ATTR_INVERSE`), or, while a mark
    is active, inverse *and* dim (`ATTR_INVERSE | ATTR_DIM` -- `syscall/fd.rs`'s own attribute
    table draws that combination as black on `DIM_FG`, not black on `FG`). The editor uses the dim
    variant so the cursor still looks distinct from the plain-inverse selected region it can be
    sitting right next to -- see Step 8's `Stage20.md` note on why the plain cursor color there
    would otherwise read as "one more selected character."."""
    _width, _height, pixels = dump
    cell = [pixels[row * CELL_H + dy][col * CELL_W + dx] for dy in range(CELL_H) for dx in range(CELL_W)]
    return cell.count(FG) > cell.count(BG) or cell.count(DIM_FG) > cell.count(BG)


def find_cursor(dump, rows=ROWS, cols=COLS):
    """The (row, col) of the one cursor cell (`is_cursor`), or None if there isn't one. Skips
    `STATUS_ROW`: it is entirely inverse by design (see `status_bar_showing`) and the cursor itself
    never lands there, so including it would make every call here find its column 0 instead."""
    for row in range(rows):
        if row == STATUS_ROW:
            continue
        for col in range(cols):
            if is_cursor(dump, row, col):
                return (row, col)
    return None


def status_bar_showing(dump):
    """Whether `STATUS_ROW` is the usual, entirely-inverse status bar -- true in every mode except
    `^G`'s help screen, which replaces the whole display with plain text and draws no inverse cell
    anywhere (Step 3 already found that `CONSOLE_DRAW`'s `cursor_row`/`cursor_col` header fields draw
    nothing visual by themselves)."""
    return all(is_inverse(dump, STATUS_ROW, col) for col in range(COLS))


def band_of(dump, row):
    """One text row's raw pixels (`CELL_H` rows), whatever they are -- unlike `text_bands`, not
    filtered to only the non-blank ones, since a specific row (the status bar, the footer) is wanted
    here regardless of its content."""
    _width, _height, pixels = dump
    return pixels[row * CELL_H:(row + 1) * CELL_H]


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


def keys_slowly(s, names):
    """Like `Session.keys`, but waits for the display to settle -- with a longer-than-default
    window, for the same reason `open_editor` uses one -- after each key. Unlike the shell's line
    editor, `edit` redraws the *whole* screen on every keystroke, so this class of test is more
    exposed than most to the input-timing gap `token_queue`'s own cases already accept (no scheduler
    yet to synchronize on: see the project's `project_input_flake_deferred` note) -- a burst sent at
    `Session.keys`' normal pace can outrun the guest's own redraw, and a short settle window can
    latch onto a stale frame before that redraw has actually happened. One settle per key, generously
    long, costs real time but is the most reliable way to be sure every key -- including, worst case,
    the very last one, an Enter meant to confirm a prompt -- actually lands before the next is sent."""
    for name in names:
        s.keys([name])
        s.screendump_settled(stable_for=0.5)


def type_slowly(s, text):
    """Like `Session.type`, but one character at a time with a settle between -- see `keys_slowly`.
    Reuses `Session.type`'s own name for each character (`KEY_NAMES`, or Shift for an uppercase
    letter, or the character itself) rather than a second copy of that mapping."""
    keys_slowly(s, (KEY_NAMES.get(c) or (f"shift-{c.lower()}" if c.isupper() else c) for c in text))


def run(ctx):
    s, check = ctx.s, ctx.check

    # --- opening an existing file shows it, cursor at the very start ---
    dump = open_editor(s, "tests/hello.txt")
    check("edit: the cursor starts at (0, 0)", is_inverse(dump, 0, 0), True)
    check("edit: the cell right of it is not inverted", is_inverse(dump, 0, 1), False)

    # Move off row 0 (its cursor was inverting the cell that would otherwise be checked) and
    # confirm "one" is actually drawn there, not left blank. Since Step 6, the status bar and
    # footer rows are always drawn too, so they are non-blank bands as well.
    s.keys(["down"])
    rows_shown = [row for row, _band in text_bands(s.screendump_settled())]
    check("edit: all twelve lines of hello.txt are drawn, plus the status bar and footer",
          rows_shown, list(range(12)) + [STATUS_ROW, FOOTER_ROW])
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

    # --- a missing file starts a new, empty buffer ---
    check("edit: the file doesn't exist yet", s.run("cat tests/brand-new.txt"),
          "cat tests/brand-new.txt\ncat: tests/brand-new.txt: No such file or directory\n")
    dump = open_editor(s, "tests/brand-new.txt")
    check("edit: a missing file is a new, empty buffer -- cursor at (0,0)", is_inverse(dump, 0, 0), True)
    check("edit: nothing else in the text area is inverted", is_inverse(dump, 1, 0), False)
    s.keys(["ctrl-x"])
    s.wait_prompt()
    check("edit: still doesn't exist -- unmodified, so ^X didn't ask and didn't save",
          s.run("cat tests/brand-new.txt"), "cat tests/brand-new.txt\ncat: tests/brand-new.txt: No such file or directory\n")

    # --- scrolling: a file taller than the text area (edit-lines.txt, 50 lines "L01".."L50") ---
    dump = open_editor(s, "tests/edit-lines.txt")
    check("edit: starts scrolled to the top", is_inverse(dump, 0, 0), True)

    s.keys(["alt-slash"])  # Alt+/: last line
    dump = s.screendump_settled()
    check("edit: Alt+/ moves the cursor to the last row of the (28-row) text area", find_cursor(dump)[0], ROWS - 3)

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
    # `edit` draws a full screen of its own (twelve lines of hello.txt, plus its own status bar and
    # footer); once it exits, none of that should still be visible. Checked by *row count*, not exact
    # row positions or pixels: by this point in the session the console has already scrolled some
    # number of times, so which absolute rows are in use depends on exactly when a screendump happens
    # to be taken, but the count is simple arithmetic either way. Right before typing the launch
    # command, the console shows some number of non-blank rows; typing it adds exactly one more (the
    # echoed command line itself, counted the moment before `edit` draws its own first frame -- the
    # very thing its snapshot captures and its exit restores). If restoring genuinely brings that back,
    # and nothing of `edit`'s own drawing survives, the count after quitting is that same "one more,"
    # not many more for hello.txt's own lines plus the status bar and footer.
    before_count = len(text_bands(s.screendump_settled()))
    s.type("edit tests/hello.txt\n")
    s.screendump_settled()  # let it actually draw its first frame before doing anything else
    s.keys(["down", "right", "right"])  # move around; Step 5b restores whatever state edit ends in
    s.keys(["ctrl-x"])
    s.wait_prompt()
    after_count = len(text_bands(s.screendump_settled()))
    check("edit: quitting restores exactly one more row than before launching it",
          after_count, before_count + 1)

    # ================================================================================================
    # Stage 20, Step 6
    # ================================================================================================

    # --- typing marks the buffer modified (the status bar changes); `^S` saves and clears it ---
    check("edit: tests/edit-new.txt doesn't exist yet", s.run("cat tests/edit-new.txt"),
          "cat tests/edit-new.txt\ncat: tests/edit-new.txt: No such file or directory\n")
    dump = open_editor(s, "tests/edit-new.txt")
    clean_status, default_footer = band_of(dump, STATUS_ROW), band_of(dump, FOOTER_ROW)
    check("edit: the status bar is the usual inverse bar", status_bar_showing(dump), True)

    s.type("ab")
    dump = s.screendump_settled()
    check("edit: typing moved the cursor to column 2", is_inverse(dump, 0, 2), True)
    modified_status = band_of(dump, STATUS_ROW)
    check("edit: typing changed the status bar ([Modified], and the column)", modified_status != clean_status, True)

    s.keys(["ctrl-s"])
    dump = s.screendump_settled()
    check("edit: saving changes the status bar again (modified cleared)",
          band_of(dump, STATUS_ROW) not in (clean_status, modified_status), True)
    check("edit: saving shows a message, not the usual footer", band_of(dump, FOOTER_ROW) != default_footer, True)

    s.keys(["right"])  # any harmless key: the message is shown for exactly one frame, then cleared
    dump = s.screendump_settled()
    check("edit: the next key clears the message -- the footer is back to normal",
          band_of(dump, FOOTER_ROW), default_footer)

    s.keys(["ctrl-x"])
    s.wait_prompt()
    check("edit: 'ab' was saved, LF-terminated", s.run("cat tests/edit-new.txt"), "cat tests/edit-new.txt\nab\n")

    # --- Backspace and Delete edit around the cursor ---
    dump = open_editor(s, "tests/edit-new.txt")  # "ab\n" from above
    check("edit: reopens the saved content -- cursor at (0,0)", is_inverse(dump, 0, 0), True)
    s.keys(["end"])
    s.keys(["backspace"])
    dump = s.screendump_settled()
    check("edit: Backspace removed the 'b' -- cursor now after 'a'", is_inverse(dump, 0, 1), True)
    s.keys(["ctrl-s"])
    s.keys(["ctrl-x"])
    s.wait_prompt()
    check("edit: 'a' was saved after Backspace", s.run("cat tests/edit-new.txt"), "cat tests/edit-new.txt\na\n")

    # --- Enter splits a line; Delete at the end of a line joins the next one back on ---
    dump = open_editor(s, "tests/edit-new.txt")  # "a\n"
    s.keys(["end"])
    s.type("bc")  # "abc", cursor at the end
    s.keys(["home", "right", "right"])  # between 'b' and 'c'
    s.keys(["ret"])
    dump = s.screendump_settled()
    check("edit: Enter split the line -- the cursor starts the new line, column 0", is_inverse(dump, 1, 0), True)
    s.keys(["ctrl-s"])
    s.keys(["ctrl-x"])
    s.wait_prompt()
    check("edit: the split saved as two lines", s.run("cat tests/edit-new.txt"), "cat tests/edit-new.txt\nab\nc\n")

    dump = open_editor(s, "tests/edit-new.txt")  # "ab\nc\n"
    s.keys(["end"])  # end of "ab"
    s.keys(["delete"])
    dump = s.screendump_settled()
    check("edit: Delete at end-of-line joined the next line back on", is_inverse(dump, 0, 2), True)
    s.keys(["ctrl-s"])
    s.keys(["ctrl-x"])
    s.wait_prompt()
    check("edit: the join saved as one line", s.run("cat tests/edit-new.txt"), "cat tests/edit-new.txt\nabc\n")

    # --- auto-indent: off by default, on with `.editrc`'s AUTOINDENT=1 ---
    dump = open_editor(s, "tests/edit-indent.txt")  # missing -- new, empty buffer
    s.type("  x")
    s.keys(["ret"])
    s.type("y")
    s.keys(["ctrl-s"])
    s.keys(["ctrl-x"])
    s.wait_prompt()
    check("edit: no .editrc -- auto-indent is off, Enter copies nothing",
          s.run("cat tests/edit-indent.txt"), "cat tests/edit-indent.txt\n  x\ny\n")
    s.run("rm tests/edit-indent.txt")

    s.run("echo AUTOINDENT=1 > .editrc")
    dump = open_editor(s, "tests/edit-indent.txt")
    s.type("  x")
    s.keys(["ret"])
    s.type("y")
    s.keys(["ctrl-s"])
    s.keys(["ctrl-x"])
    s.wait_prompt()
    check("edit: with AUTOINDENT=1, Enter copies the leading spaces onto the new line",
          s.run("cat tests/edit-indent.txt"), "cat tests/edit-indent.txt\n  x\n  y\n")
    s.run("rm tests/edit-indent.txt .editrc")

    # --- Tab: the default tab stop is 4 columns; `.editrc`'s TABSIZE overrides it ---
    dump = open_editor(s, "tests/edit-tab.txt")
    s.keys(["tab"])
    dump = s.screendump_settled()
    check("edit: a Tab with the default TABSIZE (4) advances the cursor to column 4", is_inverse(dump, 0, 4), True)
    s.keys(["ctrl-s"])
    s.keys(["ctrl-x"])
    s.wait_prompt()
    check("edit: the file holds a literal tab character", s.run("cat tests/edit-tab.txt"), "cat tests/edit-tab.txt\n\t\n")
    s.run("rm tests/edit-tab.txt")

    s.run("echo TABSIZE=2 > .editrc")
    dump = open_editor(s, "tests/edit-tab.txt")
    s.keys(["tab"])
    dump = s.screendump_settled()
    check("edit: with TABSIZE=2, Tab advances the cursor to column 2 instead", is_inverse(dump, 0, 2), True)
    s.keys(["ctrl-x"])  # unsaved and modified: the confirmation dialog appears
    dump = s.screendump_settled()
    check("edit: ^X on a modified buffer asks first (a message replaces the usual footer)",
          band_of(dump, FOOTER_ROW) != default_footer, True)
    s.keys(["n"])  # discard
    s.wait_prompt()
    check("edit: tests/edit-tab.txt still doesn't exist -- discarded, not saved",
          s.run("cat tests/edit-tab.txt"), "cat tests/edit-tab.txt\ncat: tests/edit-tab.txt: No such file or directory\n")
    s.run("rm .editrc")

    # --- a bad `.editrc` value is reported on the message line (not stderr), and falls back to the
    # default for that one setting; every other line's value still applies ---
    s.run("echo TABSIZE=99 > .editrc")
    s.run("echo LINENOS=1 >> .editrc")
    dump = open_editor(s, "tests/edit-tab.txt")
    check("edit: a bad .editrc value shows a message on the very first frame, not the usual footer",
          band_of(dump, FOOTER_ROW) != default_footer, True)
    s.keys(["ctrl-x"])
    s.wait_prompt()
    s.run("rm .editrc")

    # --- Ctrl+Left / Ctrl+Right: word movement, crossing at most one line boundary per call ---
    dump = open_editor(s, "tests/edit-tab.txt")  # missing again (never saved above) -- empty buffer
    s.type("foo bar")
    s.keys(["ret"])
    s.type("baz")
    s.keys(["ctrl-left"])
    dump = s.screendump_settled()
    check("edit: Ctrl+Left from the end of 'baz' reaches its own start", is_inverse(dump, 1, 0), True)
    s.keys(["ctrl-left"])
    dump = s.screendump_settled()
    check("edit: another Ctrl+Left crosses onto the previous line's end", is_inverse(dump, 0, 7), True)
    s.keys(["ctrl-left"])
    dump = s.screendump_settled()
    check("edit: Ctrl+Left again lands on 'bar's own start", is_inverse(dump, 0, 4), True)
    s.keys(["ctrl-right"])
    dump = s.screendump_settled()
    check("edit: Ctrl+Right returns to 'bar's own end", is_inverse(dump, 0, 7), True)
    s.keys(["ctrl-x"])
    dump = s.screendump_settled()
    s.keys(["n"])  # this buffer (never saved) is discarded
    s.wait_prompt()

    # --- `^O`: write out (save as) -- prompts for a name, saves there, and adopts it ---
    # (The status bar itself stays the usual inverse bar throughout every overlay except `^G`'s
    # help screen -- `render_status_bar` runs unconditionally -- so a prompt is told apart by where
    # the *cursor* is instead: its own cell in `FOOTER_ROW`, not the text area.)
    dump = open_editor(s, "tests/sa-src.txt")
    s.type("saveas")
    s.keys(["ctrl-o"])
    dump = s.screendump_settled()
    check("edit: ^O's prompt puts the cursor on the footer row, not the text area",
          find_cursor(dump)[0], FOOTER_ROW)
    # Clearing the prefilled filename and typing the new one is close to 50 keystrokes; `edit`
    # redraws the whole screen on every one of them (unlike the shell's line editor), so this goes
    # through `keys_slowly`/`type_slowly` rather than `Session.keys`/`Session.type`'s normal pace --
    # see their doc comments for why a plain burst here risks silently losing the `ret` below.
    keys_slowly(s, ["backspace"] * len("tests/sa-src.txt"))  # clear the prefilled filename
    type_slowly(s, "tests/sa-dst.txt")
    s.keys(["ret"])
    dump = s.screendump_settled(stable_for=0.5)
    check("edit: after Save As, the cursor is back in the text area", find_cursor(dump)[0], 0)
    check("edit: ...and a message replaces the usual footer", band_of(dump, FOOTER_ROW) != default_footer, True)
    s.keys(["ctrl-x"])  # the buffer is unmodified again (saved) -- exits at once
    s.wait_prompt()
    check("edit: nothing was ever written under the original name",
          s.run("cat tests/sa-src.txt"), "cat tests/sa-src.txt\ncat: tests/sa-src.txt: No such file or directory\n")
    check("edit: the content is under the new name instead", s.run("cat tests/sa-dst.txt"),
          "cat tests/sa-dst.txt\nsaveas\n")

    # Esc cancels a Save As prompt, changing nothing.
    dump = open_editor(s, "tests/sa-dst.txt")  # "saveas\n"
    s.keys(["ctrl-o"])
    s.keys(["x", "y", "z"])  # typed into the prompt, not the buffer
    s.keys(["esc"])
    dump = s.screendump_settled()
    check("edit: Esc cancels the prompt -- the cursor is back in the text area", find_cursor(dump)[0], 0)
    check("edit: nothing in the buffer changed", is_inverse(dump, 0, 0), True)
    s.keys(["ctrl-x"])  # still unmodified -- no confirmation
    s.wait_prompt()
    check("edit: the file is unchanged", s.run("cat tests/sa-dst.txt"),
          "cat tests/sa-dst.txt\nsaveas\n")
    s.run("rm tests/sa-dst.txt")

    # --- `^X` on a modified buffer: Y saves and exits, N discards and exits, Esc returns to editing ---
    dump = open_editor(s, "tests/edit-confirm.txt")
    s.type("keepme")
    s.keys(["ctrl-x"])
    confirm_dump = s.screendump_settled()
    check("edit: ^X on a modified buffer asks first (a message replaces the usual footer)",
          band_of(confirm_dump, FOOTER_ROW) != default_footer, True)
    s.keys(["esc"])
    dump = s.screendump_settled()
    check("edit: Esc returns to editing -- the footer is back to normal", band_of(dump, FOOTER_ROW), default_footer)
    check("edit: the cursor is still where it was", is_inverse(dump, 0, 6), True)
    s.keys(["ctrl-x"])
    s.keys(["y"])
    s.wait_prompt()
    check("edit: 'Y' saved before exiting", s.run("cat tests/edit-confirm.txt"),
          "cat tests/edit-confirm.txt\nkeepme\n")

    # The confirmation's own wording, checked against the same text printed the ordinary way
    # (`echo`, now that the shell prompt is back) -- letters and spaces only, to sidestep quoting
    # the punctuation ("?", "(", ")") would need.
    s.run("echo Save modified buffer")
    reference = text_bands(s.screendump_settled())[-2][1]
    columns = len("Save modified buffer") * CELL_W
    check("edit: the confirmation reads 'Save modified buffer...'",
          [line[:columns] for line in band_of(confirm_dump, FOOTER_ROW)],
          [line[:columns] for line in reference])

    dump = open_editor(s, "tests/edit-confirm.txt")  # "keepme\n"
    s.type("!")
    s.keys(["ctrl-x"])
    s.keys(["n"])
    s.wait_prompt()
    check("edit: 'N' discarded the change", s.run("cat tests/edit-confirm.txt"),
          "cat tests/edit-confirm.txt\nkeepme\n")
    s.run("rm tests/edit-confirm.txt")

    # --- `^G`: the full-screen help; any key returns to editing exactly as it was ---
    dump = open_editor(s, "tests/hello.txt")
    s.keys(["right"])
    before_help = s.screendump_settled()
    s.keys(["ctrl-g"])
    help_dump = s.screendump_settled()
    check("edit: ^G replaces the whole screen -- no inverse cell anywhere", status_bar_showing(help_dump), False)
    check("edit: ...not even the text area", find_cursor(help_dump), None)
    s.keys(["a"])  # any key at all returns to editing -- not inserted, not treated as a command
    dump = s.screendump_settled()
    check("edit: any key returns to editing -- the status bar is back", status_bar_showing(dump), True)
    check("edit: the buffer is exactly as it was before ^G", dump, before_help)
    s.keys(["ctrl-x"])
    s.wait_prompt()

    # The help screen's own title, checked against the same text echoed the ordinary way (now that
    # the shell prompt is back) -- the same `[-2]` technique `console.py`'s crash-message check
    # uses: the last non-blank band is the fresh prompt, the one above it is what `echo` just printed.
    TITLE = "edit -- A simple text editor"
    s.run(f"echo {TITLE}")
    reference = text_bands(s.screendump_settled())[-2][1]
    columns = len(TITLE) * CELL_W
    check("edit: the help screen's title matches the same text echoed elsewhere",
          [line[:columns] for line in band_of(help_dump, 0)], [line[:columns] for line in reference])

    # --- `^C`: the cursor's line and column, shown as a one-frame message ---
    dump = open_editor(s, "tests/hello.txt")
    s.keys(["down", "down", "right", "right"])  # line 3 ("three"), column 2 (1-based: line 3, column 3)
    default_footer_hello = band_of(s.screendump_settled(), FOOTER_ROW)
    s.keys(["ctrl-c"])
    position_dump = s.screendump_settled()
    s.keys(["left"])  # any key: the message is shown for exactly one frame
    dump = s.screendump_settled()
    check("edit: the next key clears ^C's message", band_of(dump, FOOTER_ROW), default_footer_hello)
    s.keys(["ctrl-x"])
    s.wait_prompt()

    # ^C's own wording, checked against the same text echoed the ordinary way (now that the shell
    # prompt is back).
    s.run("echo line 3 of 12, column 3")
    reference = text_bands(s.screendump_settled())[-2][1]
    columns = len("line 3 of 12, column 3") * CELL_W
    check("edit: ^C shows the exact line/total/column",
          [line[:columns] for line in band_of(position_dump, FOOTER_ROW)],
          [line[:columns] for line in reference])

    check("shell alive after Stage 20 Step 6's edit checks", s.run("echo alive"), "echo alive\nalive\n")

    # ================================================================================================
    # Stage 20, Step 7
    # ================================================================================================

    # --- `^K` cuts the whole current line into the cut buffer; `^U` pastes it back above the cursor.
    # The cut buffer is this *program's own* memory, gone the moment `edit` exits (nano's is too), so
    # a cut and its paste have to happen in the same session -- not across a save-quit-reopen. ---
    dump = open_editor(s, "tests/edit-cut.txt")  # missing -- new, empty buffer
    s.type("one")
    s.keys(["ret"])
    s.type("two")
    s.keys(["ret"])
    s.type("three")
    s.keys(["home", "up", "up"])  # back to "one" (^K cuts the *current* line, whatever the column)
    s.keys(["ctrl-k"])
    dump = s.screendump_settled()
    check("edit: ^K's cursor lands on the line that took the cut one's place", is_inverse(dump, 0, 0), True)
    s.keys(["ctrl-u"])  # paste it straight back, same session -- the round trip is the identity
    dump = s.screendump_settled()
    check("edit: ^U pastes it back above the cursor -- back to (0,0) on it again", is_inverse(dump, 0, 0), True)
    s.keys(["ctrl-s"])
    s.keys(["ctrl-x"])
    s.wait_prompt()
    check("edit: cut then paste reproduces the original file exactly", s.run("cat tests/edit-cut.txt"),
          "cat tests/edit-cut.txt\none\ntwo\nthree\n")

    # --- consecutive `^K`s accumulate into one cut buffer, in order; anything else ends the streak ---
    dump = open_editor(s, "tests/edit-cut.txt")  # "one\ntwo\nthree\n"
    s.keys(["ctrl-k", "ctrl-k"])  # cuts "one", then "two" (now the current line after "one" left)
    dump = s.screendump_settled()
    check("edit: two ^K's leave only 'three'", is_inverse(dump, 0, 0), True)
    s.keys(["right"])  # any other key first: ends the streak, so this ^K would start a *new* cut
    s.keys(["home"])
    s.keys(["ctrl-u"])  # paste the accumulated two-line cut back above "three"
    dump = s.screendump_settled()
    check("edit: ^U pastes both cut lines back, in the order they were cut", is_inverse(dump, 0, 0), True)
    s.keys(["ctrl-s"])
    s.keys(["ctrl-x"])
    s.wait_prompt()
    check("edit: the accumulated cut and its paste also reproduce the file exactly",
          s.run("cat tests/edit-cut.txt"), "cat tests/edit-cut.txt\none\ntwo\nthree\n")
    s.run("rm tests/edit-cut.txt")

    # --- `Alt+6` copies the line without removing it (and isn't part of any `^K` streak) ---
    dump = open_editor(s, "tests/edit-copy.txt")
    s.type("keep me")
    s.keys(["alt-6"])
    s.keys(["ret"])
    s.type("second line")
    s.keys(["ctrl-u"])  # pastes the *copy* above the cursor -- "keep me" is still there too
    s.keys(["ctrl-s"])
    s.keys(["ctrl-x"])
    s.wait_prompt()
    check("edit: Alt+6 copies without removing, ^U pastes the copy", s.run("cat tests/edit-copy.txt"),
          "cat tests/edit-copy.txt\nkeep me\nkeep me\nsecond line\n")
    s.run("rm tests/edit-copy.txt")

    # --- `^W` searches forward, wrapping around; `Alt+W` finds the next match ---
    dump = open_editor(s, "tests/edit-search.txt")
    s.type("apple banana")
    s.keys(["ret"])
    s.type("banana cherry")
    s.keys(["home", "up", "home"])  # the very start of the file
    s.keys(["ctrl-w"])
    dump = s.screendump_settled()
    check("edit: ^W opens a prompt (the cursor moves to the footer row)", find_cursor(dump)[0], FOOTER_ROW)
    type_slowly(s, "banana")
    s.keys(["ret"])
    dump = s.screendump_settled(stable_for=0.5)
    check("edit: ^W finds the first 'banana', landing right after it", is_inverse(dump, 0, 12), True)
    s.keys(["alt-w"])  # find next: the second 'banana', on the following line
    dump = s.screendump_settled()
    check("edit: Alt+W finds the next match", is_inverse(dump, 1, 6), True)
    s.keys(["alt-w"])  # only two matches -- wraps back around to the first
    dump = s.screendump_settled()
    check("edit: Alt+W wraps back around once there's no more", is_inverse(dump, 0, 12), True)

    s.keys(["ctrl-w"])  # the prompt is prefilled with the last term ("banana"); clear it first
    keys_slowly(s, ["backspace"] * len("banana"))
    type_slowly(s, "nope")
    s.keys(["ret"])
    dump = s.screendump_settled(stable_for=0.5)
    check("edit: a term that's nowhere in the buffer shows a message",
          band_of(dump, FOOTER_ROW) != default_footer, True)
    s.keys(["ctrl-x"])  # typing "apple banana"/"banana cherry" modified the buffer -- asks first
    s.keys(["n"])  # discarded: this file's saved content was never the point of this check
    s.wait_prompt()

    # --- `^_` and `Alt+G` both open the same go-to-line prompt; `LINE,COL` positions the column too ---
    dump = open_editor(s, "tests/edit-lines.txt")  # fifty lines, "L01".."L50"
    s.keys(["ctrl-shift-minus"])
    dump = s.screendump_settled()
    check("edit: ^_ opens the go-to-line prompt", find_cursor(dump)[0], FOOTER_ROW)
    type_slowly(s, "25")
    s.keys(["ret"])
    dump = s.screendump_settled(stable_for=0.5)
    # `scroll::center_on` (already host-tested) walks back from the target line by about half a
    # screenful: with a 28-row text area and every one of these lines one screen row tall, line 25
    # (0-based 24) lands 14 rows from the top -- an exact position, not just "somewhere on screen".
    check("edit: ^_ 25 jumps there, centering the view on it", find_cursor(dump), (14, 0))

    s.keys(["alt-g"])
    type_slowly(s, "30,2")
    s.keys(["ret"])
    dump = s.screendump_settled(stable_for=0.5)
    check("edit: Alt+G with LINE,COL positions the column too", find_cursor(dump), (14, 1))

    s.keys(["alt-g"])
    type_slowly(s, "not-a-number")
    s.keys(["ret"])
    dump = s.screendump_settled(stable_for=0.5)
    check("edit: an unparseable go-to answer shows a message, not a silent no-op",
          band_of(dump, FOOTER_ROW) != default_footer, True)
    s.keys(["ctrl-x"])
    s.wait_prompt()

    check("shell alive after Stage 20 Step 7's edit checks", s.run("echo alive"), "echo alive\nalive\n")

    # ================================================================================================
    # Stage 20, Step 8
    # ================================================================================================

    # --- `Alt+A` sets/clears the mark; the region between it and the cursor is drawn inverse ---
    dump = open_editor(s, "tests/edit-mark.txt")
    s.type("hello world")
    s.keys(["home"])
    s.keys(["alt-a"])
    dump = s.screendump_settled()
    check("edit: Alt+A sets the mark (a message, not the usual footer)",
          band_of(dump, FOOTER_ROW) != default_footer, True)
    s.keys(["right", "right", "right", "right", "right"])  # mark (0,0), cursor now (0,5): "hello"
    dump = s.screendump_settled()
    check("edit: the region between mark and cursor is drawn inverse",
          all(is_inverse(dump, 0, col) for col in range(5)), True)
    # The cursor sits one past the actual region (`[mark, cursor)`, never including the cursor's own
    # position) but is drawn distinctly (dim as well as inverse -- `is_cursor`) rather than plain
    # inverse, so it doesn't look like one more selected character.
    check("edit: the cursor cell itself is drawn distinctly, not as a plain selected cell",
          (is_cursor(dump, 0, 5), is_inverse(dump, 0, 5)), (True, False))
    check("edit: the cell after that is plain -- outside the region", is_cursor(dump, 0, 6), False)
    s.keys(["alt-a"])  # clear the mark: the highlight goes, the cursor doesn't move
    dump = s.screendump_settled()
    check("edit: Alt+A again clears the mark -- no highlight left before the cursor",
          is_inverse(dump, 0, 0), False)
    check("edit: ...the cursor is still where it was", is_inverse(dump, 0, 5), True)
    s.keys(["ctrl-x"])
    s.keys(["n"])
    s.wait_prompt()

    # --- `^K` with a mark set cuts the *region* (character-granular), not the whole line ---
    dump = open_editor(s, "tests/edit-region.txt")
    s.type("hello world")
    s.keys(["home", "alt-a"])
    s.keys(["right", "right", "right", "right", "right", "right"])  # mark (0,0), cursor (0,6): "hello "
    s.keys(["ctrl-k"])
    dump = s.screendump_settled()
    check("edit: ^K with a mark cuts the region -- cursor lands at its start", is_inverse(dump, 0, 0), True)
    s.keys(["ctrl-u"])  # splice it right back in at the cursor -- character-granular, not a new line
    s.keys(["ctrl-s"])
    s.keys(["ctrl-x"])
    s.wait_prompt()
    check("edit: region cut then paste reproduces the original line exactly",
          s.run("cat tests/edit-region.txt"), "cat tests/edit-region.txt\nhello world\n")

    # --- `Alt+6` with a mark copies the region without removing it ---
    dump = open_editor(s, "tests/edit-region.txt")  # "hello world\n"
    s.keys(["home", "alt-a"])
    s.keys(["right", "right", "right", "right", "right"])  # mark (0,0), cursor (0,5): "hello"
    s.keys(["alt-6"])
    s.keys(["end"])
    s.keys(["ctrl-u"])  # paste the copy at the end of the line
    s.keys(["ctrl-s"])
    s.keys(["ctrl-x"])
    s.wait_prompt()
    check("edit: Alt+6 with a mark copies the region, leaving the original untouched",
          s.run("cat tests/edit-region.txt"), "cat tests/edit-region.txt\nhello worldhello\n")
    s.run("rm tests/edit-region.txt")

    # --- `Alt+N` toggles the line-number gutter; the cursor shifts right by its own width ---
    dump = open_editor(s, "tests/hello.txt")  # twelve lines: gutter width = digits(12) + 1 = 3
    before = find_cursor(dump)
    s.keys(["alt-n"])
    dump = s.screendump_settled()
    check("edit: Alt+N shifts the cursor right by the gutter's width (digits(12)+1 = 3)",
          find_cursor(dump), (before[0], before[1] + 3))
    s.keys(["alt-n"])  # toggle back off
    dump = s.screendump_settled()
    check("edit: Alt+N again returns the cursor to where it was", find_cursor(dump), before)
    s.keys(["ctrl-x"])
    s.wait_prompt()

    # --- `Alt+I` toggles auto-indent at runtime, with no `.editrc` needed ---
    dump = open_editor(s, "tests/edit-indent2.txt")
    s.keys(["alt-i"])
    dump = s.screendump_settled()
    check("edit: Alt+I shows a message (not the usual footer)", band_of(dump, FOOTER_ROW) != default_footer, True)
    s.type("  x")
    s.keys(["ret"])
    s.type("y")
    s.keys(["ctrl-s"])
    s.keys(["ctrl-x"])
    s.wait_prompt()
    check("edit: with auto-indent toggled on, Enter copies the leading spaces",
          s.run("cat tests/edit-indent2.txt"), "cat tests/edit-indent2.txt\n  x\n  y\n")
    s.run("rm tests/edit-indent2.txt")

    check("shell alive after Stage 20 Step 8's edit checks", s.run("echo alive"), "echo alive\nalive\n")
