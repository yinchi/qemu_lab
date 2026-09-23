"""Last updated: Stage 12, Step 12.

Cursor-aware editing (arrows, Home/End/Delete, Ctrl+A/E/U/K) and command history (Up/Down) at the
shell's prompt (`Mode::Prompt`); a program's `read(0)` (`Mode::Canonical`) stays the simpler
cooked-mode behavior it always was, plus the one POSIX refinement Step 12 adds: Ctrl+D on a
non-empty line delivers what's typed so far without a newline, rather than doing nothing.

See `Stage12.md`'s Step 12 "Token handling by mode" table for what every key means in each mode.
"""

from harness import (
    BACKSPACE, CELL_H, CELL_W, CTRL_A, CTRL_D, CTRL_E, CTRL_K, CTRL_U, DELETE, DOWN, END, HOME,
    LEFT, RIGHT, UP, text_bands,
)

COLS = 80


def run(ctx):
    s, check = ctx.s, ctx.check

    # --- T12.1: insert mid-line, Backspace/Delete mid-line ---
    s.type("echo ac")
    s.keys([LEFT])
    s.type("b\n")
    check("insert mid-line: 'echo ac' + Left + 'b' => abc", s.wait_prompt(), "echo abc\nabc\n")

    s.type("echo abXc")
    s.keys([LEFT])
    s.keys([BACKSPACE])
    s.type("\n")
    check("Backspace mid-line removes the character before the cursor", s.wait_prompt(), "echo abc\nabc\n")

    s.type("echo aXbc")
    s.keys([LEFT, LEFT, LEFT])
    s.keys([DELETE])
    s.type("\n")
    check("Delete mid-line removes the character at the cursor", s.wait_prompt(), "echo abc\nabc\n")

    # --- T12.1: Left/Right are no-ops at the boundaries (screendump: nothing moves) ---
    s.type("echo z")
    before = s.screendump_settled()
    s.keys([RIGHT] * 5)  # already at the end
    check("Right past the end of the line is a no-op", s.screendump_settled() == before, True)
    s.keys([HOME])
    at_home = s.screendump_settled()
    s.keys([LEFT] * 5)  # already at column 0
    check("Left before the start of the line is a no-op", s.screendump_settled() == at_home, True)
    s.keys([END])
    s.type("\n")
    s.wait_prompt()

    # --- T12.1/T12.4b: Home/End and Ctrl+A/E land exactly where they say ---
    s.type("abc")
    s.keys([HOME])
    s.type("echo ")
    s.type("\n")
    check("Home moves the cursor to the true start of the line", s.wait_prompt(), "echo abc\nabc\n")

    s.type("echo ab")
    s.keys([HOME, END])
    s.type("c\n")
    check("End returns the cursor to the end of the line", s.wait_prompt(), "echo abc\nabc\n")

    s.type("abc")
    s.keys([CTRL_A])
    s.type("echo ")
    s.keys([CTRL_E])
    s.type("\n")
    check("Ctrl+A/E behave like Home/End", s.wait_prompt(), "echo abc\nabc\n")

    # --- T12.4b: Ctrl+U/K exact results ---
    s.type("XXXXecho abc")
    s.keys([LEFT] * 8)  # right before "echo abc" (8 characters), after the garbage prefix
    s.keys([CTRL_U])
    s.type("\n")
    check("Ctrl+U erases from the cursor to the start of the line", s.wait_prompt(), "echo abc\nabc\n")

    s.type("echo abcXXXX")
    s.keys([LEFT] * 4)  # right after "abc", before the garbage suffix
    s.keys([CTRL_K])
    s.type("\n")
    check("Ctrl+K erases from the cursor to the end of the line", s.wait_prompt(), "echo abc\nabc\n")

    # --- regression: a line whose end exactly fills a row must not crash the kernel (a cursor
    # naively resolved to "the row past the last one drawn" panicked `console::put_char_at`'s bounds
    # assert -- `input_layout.rs`'s `cursor_position` now stays deferred-wrap at the true end
    # instead, matching xterm's own rule) ---
    exact = "x" * (COLS - len("> "))
    s.type(exact)
    s.screendump_settled()  # would have panicked here, pre-fix
    s.keys([BACKSPACE] * len(exact))
    check("shell alive after a line exactly filling a row", s.run("echo ok"), "echo ok\nok\n")

    # --- T12.2: history ---
    s.run("echo one")
    s.run("echo two")
    s.run("echo three")
    s.type("echo pending")
    # Far more Ups than this history could possibly hold at this point in the session -- exercises
    # "capped at the oldest, further Ups are a no-op" without needing to know the exact count; the
    # same number of Downs is then guaranteed to walk all the way back to the pending line.
    ups = 50
    s.keys([UP] * ups)
    s.keys([DOWN] * ups)
    s.type("\n")
    check("Up (capped at the oldest) then the same number of Down returns to the pending line",
          s.wait_prompt(), "echo pending\npending\n")

    s.run("echo dup")
    s.run("echo dup")  # exact duplicate of the last command: not re-recorded
    s.keys([UP])  # "echo dup"
    s.keys([UP])  # the previous *distinct* entry -- proves the second "echo dup" wasn't recorded
    s.type("\n")
    check("a duplicate of the last command is not re-recorded", s.wait_prompt(), "echo pending\npending\n")

    s.run("")  # a blank line: not recorded either
    s.keys([UP])  # the newest real entry -- "echo pending" again, resubmitted (and re-recorded,
    # since it's no longer a *consecutive* duplicate) by the check just above
    s.type("\n")
    check("an empty line is not recorded", s.wait_prompt(), "echo pending\npending\n")

    # --- T12.3: the visible cursor lands on the right cell, including at a wrap boundary ---
    def corner(dump, row, col):
        _w, _h, rows = dump
        return rows[row * CELL_H][col * CELL_W]

    s.type("echo " + "z" * 90)  # prompt(2) + "echo "(5) + 90 z's = 97 cells: wraps onto two rows
    before = s.screendump_settled()
    s.keys([HOME])
    after = s.screendump_settled()
    bands = text_bands(before)
    line_row, wrapped_row = bands[-2][0], bands[-1][0]
    check("Home un-inverts the old cursor cell (end of the wrapped row)",
          corner(before, wrapped_row, 17) != corner(after, wrapped_row, 17), True)
    check("...and draws it at the true start of the line",
          corner(before, line_row, 2) != corner(after, line_row, 2), True)
    s.keys([END])
    s.type("\n")
    s.wait_prompt()

    # --- T12.4: `Mode::Canonical` (a program's read(0)) is unaffected by any of the above ---
    s.type("cat\n")
    s.wait_until(lambda t: t.endswith("cat\n"), "cat to start")
    s.type("abc")
    s.keys([LEFT, LEFT, HOME, CTRL_A, RIGHT, END, UP, DOWN, CTRL_E, CTRL_K])  # all no-ops here
    s.keys([BACKSPACE])  # still works: removes 'c'
    s.type("c\n")
    s.wait_until(lambda t: t.endswith("abc\nabc\n"),
                 "movement keys do nothing in canonical mode; Backspace still works")

    s.type("garbage")
    s.keys([CTRL_U])  # POSIX KILL: discards the whole line, same as at the prompt
    s.type("kept\n")
    s.wait_until(lambda t: t.endswith("kept\nkept\n"), "Ctrl+U discards the line in canonical mode too")

    # --- T12.4: Ctrl+D's POSIX partial-delivery refinement ---
    s.type("abc")
    s.keys([CTRL_D])  # non-empty line: delivers "abc" with no trailing newline
    s.wait_until(lambda t: t.endswith("abc"), "Ctrl+D on a non-empty line delivers it without a newline")
    s.keys([CTRL_D])  # now empty: end-of-file
    # `wait_prompt` returns everything since "cat\n" (the `wait_until` calls above never reset the
    # checkpoint): the line discipline's own echo of each finished line plus cat's own stdout echo
    # of what it read, doubling each one -- except the Ctrl+D delivery, which has no line-discipline
    # echo of its own (`LineOutcome::Partial` doesn't call `uart_write`, unlike `Finished`) and no
    # newline of its own either; the one trailing here is `start_prompt`'s `uart_ensure_newline`,
    # inserted because the transcript wasn't already on a fresh line when the next prompt began.
    check("cat exits once Ctrl+D hits an empty line", s.wait_prompt(), "cat\nabc\nabc\nkept\nkept\nabc\n")

    # --- T12.4b: Ctrl+D still does nothing at the prompt ---
    s.type("echo x")
    s.keys([CTRL_D])
    s.type("y\n")
    check("Ctrl+D at the prompt does nothing", s.wait_prompt(), "echo xy\nxy\n")

    check("shell alive after everything above", s.run("echo ok"), "echo ok\nok\n")
