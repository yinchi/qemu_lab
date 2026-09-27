"""Last updated: Stage 20, Step 1.

The console write path -- UTF-8 that is split across `write` calls or invalid is decoded, not
blanked; stdout is buffered so a `write!` costs one display flush, not one per fragment; and a
segmentation fault is shown on the display, not only on the serial log. Since Stage 20, a program can
also ask how big the console is (`TIOCGWINSZ`, through `probe winsize`).

Uses the kernel built with `testhooks` (see the justfile's `build-test`), which reports how many display
flushes each program's console writes caused; `Session.flush_counts` reads them.
"""

from harness import text_bands

DIGITS = "".join(str(i % 10) for i in range(200))


def run(ctx):
    s, check = ctx.s, ctx.check
    s.run("chmod +x tests/probe")

    # --- T2.1: every byte value reaches the serial log untouched (only \n becomes \r\n, as always) ---
    # binary256's last byte (0xff) is not a newline, so the shell's own "start the next prompt on a
    # fresh row" behavior (`uart_ensure_newline`, in `start_prompt`) adds one real `\r\n` of its own
    # right before the prompt -- same as it would for any command whose last output doesn't end in one.
    every_byte = bytes(range(256))
    want = every_byte.replace(b"\n", b"\r\n") + b"\r\n"
    check("cat binary256: serial carries every byte", s.run_raw("cat tests/binary256"), want)
    check("shell alive after binary output", s.run("echo alive"), "echo alive\nalive\n")

    # --- T2.2: a multibyte character split across cat's 4096-byte chunks draws like an unsplit one ---
    text = ctx.fixture("utf8-boundary.txt")
    check("cat utf8-boundary: serial transcript", s.run("cat tests/utf8-boundary.txt"),
          "cat tests/utf8-boundary.txt\n" + text)
    split = text_bands(s.screendump_settled())
    s.run("cat tests/utf8-line.txt")
    whole = text_bands(s.screendump_settled())
    # The last band is the prompt; the one above it is the fixture's last line.
    check("split character is drawn like the unsplit one", split[-2][1] == whole[-2][1], True)
    check("...and is not the '<invalid utf8>' placeholder", split[-2][1] == split[-3][1], False)

    # --- T2.5: formatted output to stdout costs one display flush per line, not per fragment ---
    check("probe frag output", s.run("tests/probe frag"), "tests/probe frag\n" + DIGITS + "\n")
    check("frag: one console flush for 200 fragments", s.flush_counts()[-1], 1)
    check("probe frag-raw output", s.run("tests/probe frag-raw"), "tests/probe frag-raw\n" + DIGITS + "\n")
    check("frag-raw: one flush per raw write (the counter works)", s.flush_counts()[-1], 201)

    # --- T2.6: stdout's pending text goes out before stderr's, so the order is the program's ---
    check("interleave: OUT, ERR, newline stay in order", s.run("tests/probe interleave"),
          "tests/probe interleave\nOUTERR\n")

    # --- the segmentation fault message is on the display as well as the serial log ---
    s.run("echo Segmentation")
    reference = text_bands(s.screendump_settled())[-2][1]
    crash = s.run("crash")
    check("crash: serial message unchanged", "Segmentation fault (address 0xffff800000000000" in crash, True)
    columns = len("Segmentation") * 8
    shown = any(
        [line[:columns] for line in band] == [line[:columns] for line in reference]
        for _row, band in text_bands(s.screendump_settled())
    )
    check("crash: 'Segmentation' is on the display", shown, True)

    # --- Stage 20, Step 1: TIOCGWINSZ -- the console's size in cells, from any of the three standard fds ---
    # 640x480 pixels of 8x16-pixel cells: 30 rows, 80 columns; the pixel fields are left 0.
    check("winsize: stdout is the console, 30 rows x 80 columns", s.run("tests/probe winsize 1"),
          "tests/probe winsize 1\nwinsize(1): 30 80 0 0\n")
    check("winsize: stdin (the keyboard) answers too", s.run("tests/probe winsize 0"),
          "tests/probe winsize 0\nwinsize(0): 30 80 0 0\n")
    check("winsize: stderr answers", s.run("tests/probe winsize 2"),
          "tests/probe winsize 2\nwinsize(2): 30 80 0 0\n")
    check("winsize: an fd that is not open is EBADF", s.run("tests/probe winsize 3"),
          "tests/probe winsize 3\nwinsize(3): -9\n")
    # Redirected, an fd is no longer the console: ENOTTY (how a program learns it is not on a terminal) -- while
    # the standard fds that were not redirected still answer.
    s.run("tests/probe winsize 1 > /tmp/ws")
    check("winsize: redirected stdout is ENOTTY", s.run("cat /tmp/ws"), "cat /tmp/ws\nwinsize(1): -25\n")
    s.run("tests/probe winsize 2 > /tmp/ws")
    check("winsize: stderr still answers when only stdout is redirected", s.run("cat /tmp/ws"),
          "cat /tmp/ws\nwinsize(2): 30 80 0 0\n")
    check("winsize: redirected stdin is ENOTTY", s.run("tests/probe winsize 0 < /tmp/ws"),
          "tests/probe winsize 0 < /tmp/ws\nwinsize(0): -25\n")
    s.run("rm /tmp/ws")
    # A bad output pointer is refused, never a kernel fault: null, below the window, read-only code (the program's
    # first page), eight bytes that start inside the window but end past its top, the top itself, and a wrapping one.
    for addr in (0, 1, 0x4400_0000, 0x4600_0000 - 4, 0x4600_0000, 2**64 - 4):
        check(f"winsize: bad output pointer {addr:#x} is EFAULT", s.run(f"tests/probe winsize-ptr 1 {addr}"),
              f"tests/probe winsize-ptr 1 {addr}\nwinsize-ptr(1, {addr}): -14\n")
    check("winsize: shell alive after the bad pointers", s.run("echo alive"), "echo alive\nalive\n")

    # --- Stage 20, Step 2: CONSOLE_READ_KEY -- one key event per call, bypassing the line discipline ---
    # NumLock starts on, so every event's mods include 16 (MOD_NUM) even for a non-keypad key (`Token.num`
    # is the lock state at the time, not something the keypad alone carries).
    s.type("tests/probe read-key 0\n")
    s.keys(["a"])
    check("read-key: a plain key (code 30 'a', mods 16 NUM, ch 97 'a')", s.wait_prompt(),
          "tests/probe read-key 0\nread-key(0): 30 16 97\n")

    s.type("tests/probe read-key 0\n")
    s.keys(["shift-a"])
    check("read-key: Shift sets bit 1 and capitalizes ch ('A' = 65)", s.wait_prompt(),
          "tests/probe read-key 0\nread-key(0): 30 17 65\n")

    s.type("tests/probe read-key 0\n")
    s.keys(["ctrl-left"])
    check("read-key: Ctrl+Left is code 105 (KEY_LEFT), mods 18 (NUM+CTRL), no character",
          s.wait_prompt(), "tests/probe read-key 0\nread-key(0): 105 18 0\n")

    s.type("tests/probe read-key 0\n")
    s.keys(["ret"])
    check("read-key: Enter is code 28, no character of its own", s.wait_prompt(),
          "tests/probe read-key 0\nread-key(0): 28 16 0\n")

    check("read-key: an fd that is not open is EBADF", s.run("tests/probe read-key 3"),
          "tests/probe read-key 3\nread-key(3): -9\n")
    check("read-key: stdout is not the keyboard, ENOTTY", s.run("tests/probe read-key 1"),
          "tests/probe read-key 1\nread-key(1): -25\n")
    s.run("tests/probe winsize 1 > /tmp/rk")  # anything, just to have a file to redirect from
    check("read-key: redirected stdin is ENOTTY, same as TIOCGWINSZ", s.run("tests/probe read-key 0 < /tmp/rk"),
          "tests/probe read-key 0 < /tmp/rk\nread-key(0): -25\n")
    s.run("rm /tmp/rk")

    # A bad output pointer is refused *before* blocking for a key -- never a kernel fault, and the shell is
    # never left waiting for a keypress that could never be delivered anywhere.
    for addr in (0, 1, 0x4400_0000, 0x4600_0000 - 4, 0x4600_0000, 2**64 - 4):
        check(f"read-key: bad output pointer {addr:#x} is EFAULT, without blocking",
              s.run(f"tests/probe read-key-ptr 0 {addr}"),
              f"tests/probe read-key-ptr 0 {addr}\nread-key-ptr(0, {addr}): -14\n")

    # --- read(0) is untouched by CONSOLE_READ_KEY: the next program's line-based read still works ---
    check("read(0) still works normally after CONSOLE_READ_KEY was used", s.run("echo alive"),
          "echo alive\nalive\n")
