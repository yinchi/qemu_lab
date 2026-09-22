"""Last updated: Stage 12, Step 8.

The console write path -- UTF-8 that is split across `write` calls or invalid is decoded, not
blanked; stdout is buffered so a `write!` costs one display flush, not one per fragment; and a
segmentation fault is shown on the display, not only on the serial log.

Uses the kernel built with `testhooks` (see the justfile's `build-test`), which reports how many display
flushes each program's console writes caused; `Session.flush_counts` reads them.
"""

from harness import text_bands

DIGITS = "".join(str(i % 10) for i in range(200))


def run(ctx):
    s, check = ctx.s, ctx.check
    s.run("chmod +x tests/probe.exe")

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
    split = text_bands(s.screendump())
    s.run("cat tests/utf8-line.txt")
    whole = text_bands(s.screendump())
    # The last band is the prompt; the one above it is the fixture's last line.
    check("split character is drawn like the unsplit one", split[-2][1] == whole[-2][1], True)
    check("...and is not the '<invalid utf8>' placeholder", split[-2][1] == split[-3][1], False)

    # --- T2.5: formatted output to stdout costs one display flush per line, not per fragment ---
    check("probe frag output", s.run("tests/probe.exe frag"), "tests/probe.exe frag\n" + DIGITS + "\n")
    check("frag: one console flush for 200 fragments", s.flush_counts()[-1], 1)
    check("probe frag-raw output", s.run("tests/probe.exe frag-raw"), "tests/probe.exe frag-raw\n" + DIGITS + "\n")
    check("frag-raw: one flush per raw write (the counter works)", s.flush_counts()[-1], 201)

    # --- T2.6: stdout's pending text goes out before stderr's, so the order is the program's ---
    check("interleave: OUT, ERR, newline stay in order", s.run("tests/probe.exe interleave"),
          "tests/probe.exe interleave\nOUTERR\n")

    # --- the segmentation fault message is on the display as well as the serial log ---
    s.run("echo Segmentation")
    reference = text_bands(s.screendump())[-2][1]
    crash = s.run("crash")
    check("crash: serial message unchanged", "Segmentation fault (address 0xffff800000000000" in crash, True)
    columns = len("Segmentation") * 8
    shown = any(
        [line[:columns] for line in band] == [line[:columns] for line in reference]
        for _row, band in text_bands(s.screendump())
    )
    check("crash: 'Segmentation' is on the display", shown, True)
