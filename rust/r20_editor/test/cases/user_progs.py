"""Step 10's expanded scope: `mkdir`/`rm`/`mv`/`stat` (new programs), multi-operand `cp`/`chmod`/`mv`,
`chmod -R`, `rm -f`, the trivial/moderate flag catch-up (`echo -n`, `wc -L`/multi-file, `ls` multi-dir/`-l`,
`head -c`/`tail -c`), and `--help` on a representative sample of programs. See `Stage12.md`'s Step 10
section and `docs/progs.md`.

Fixtures: `disk/tests/hello.txt`, `disk/tests/notes.txt`, `disk/tests/docs/example.txt` (existing), plus
`disk/tests/tree/` (new: a 3-level directory tree for `rm -r`) -- `a.txt`, `sub1/b.txt`, `sub1/sub2/c.txt`.
"""

# `stat` shows times in `$TZ`; the expectations below are Toronto's.
ENVIRONMENT = "HOME=/\nTZ=America/Toronto\n"

import calendar
import os
import time
from datetime import datetime, timezone
from zoneinfo import ZoneInfo

# `folder_to_img.sh` pins mtools' timestamps to this Unix time (2026-01-01T00:00:00Z) for
# reproducible builds; mtools converts it to *local* time when stamping FAT dates (there's no
# timezone in a FAT timestamp), so the expected wall-clock value depends on the host's zone --
# computed the same way here, rather than hardcoded, so this test isn't tied to one timezone.
TORONTO = ZoneInfo("America/Toronto")
# The bundled fixtures' fixed stamp is UTC (the image is built in UTC); `stat` shows it in the local zone.
BUILD_STAMP = datetime.fromtimestamp(1767225600, tz=TORONTO).strftime("%Y-%m-%d %H:%M:%S %Z")


def run(ctx):
    s, check = ctx.s, ctx.check
    hello_txt = ctx.fixture("hello.txt")
    notes_txt = ctx.fixture("notes.txt")
    example_txt = ctx.fixture("docs/example.txt")

    # ================================================================= mkdir
    check("mkdir creates a directory", s.run("mkdir tests/newdir"), "mkdir tests/newdir\n")
    check("ls on the new (empty) directory", s.run("ls tests/newdir"), "ls tests/newdir\n")
    check("mkdir again is File exists", s.run_status("mkdir tests/newdir"), ("mkdir tests/newdir\nmkdir: cannot create directory 'tests/newdir': File exists\n", 1))
    check("mkdir under a missing parent", s.run_status("mkdir tests/nosuchparent/x"), ("mkdir tests/nosuchparent/x\nmkdir: cannot create directory 'tests/nosuchparent/x': No such file or directory\n", 1))
    check("mkdir multi-operand continues past a failure", s.run_status("mkdir tests/newdir tests/newdir2"), ("mkdir tests/newdir tests/newdir2\nmkdir: cannot create directory 'tests/newdir': File exists\n", 1))
    check("...but the second operand was still created", s.run("ls tests/newdir2"), "ls tests/newdir2\n")

    # ================================================================= rm
    check("rm refuses a directory without -r", s.run_status("rm tests/newdir"), ("rm tests/newdir\nrm: cannot remove 'tests/newdir': Is a directory\n", 1))
    check("rm -r removes an empty directory", s.run("rm -r tests/newdir"), "rm -r tests/newdir\n")
    check("...it's really gone", s.run_status("ls tests/newdir"), ("ls tests/newdir\nls: cannot access 'tests/newdir': No such file or directory\n", 1))
    check("rm on a missing file", s.run_status("rm tests/nosuchfile"), ("rm tests/nosuchfile\nrm: cannot remove 'tests/nosuchfile': No such file or directory\n", 1))
    check("rm -f on a missing file is silent", s.run("rm -f tests/nosuchfile"), "rm -f tests/nosuchfile\n")
    check("rm .", s.run_status("rm ."), ("rm .\nrm: refusing to remove '.' or '..' directory: skipping '.'\n", 1))
    check("rm ..", s.run_status("rm .."), ("rm ..\nrm: refusing to remove '.' or '..' directory: skipping '..'\n", 1))
    check("rm /", s.run_status("rm /"), ("rm /\nrm: cannot remove '/': Is a directory\n", 1))
    check("rm -r / is refused too", s.run_status("rm -r /"), ("rm -r /\nrm: it is dangerous to operate recursively on '/'\n", 1))
    check("rm -r on a populated 3-level tree", s.run("rm -r tests/tree"), "rm -r tests/tree\n")
    check("...the whole tree is gone", s.run_status("ls tests/tree"), ("ls tests/tree\nls: cannot access 'tests/tree': No such file or directory\n", 1))

    s.run("cp tests/hello.txt tests/rmtarget.txt")
    s.run("chmod -w tests/rmtarget.txt")
    check("rm deletes a read-only file without -f (deletion isn't gated by the file's own attrs)",
          s.run("rm tests/rmtarget.txt"), "rm tests/rmtarget.txt\n")

    # ================================================================= mv
    s.run("mkdir tests/mvdir")
    s.run("cp tests/hello.txt tests/mvsrc.txt")
    check("mv renames", s.run("mv tests/mvsrc.txt tests/mvdst.txt"), "mv tests/mvsrc.txt tests/mvdst.txt\n")
    check("the old name is gone", s.run_status("ls tests/mvsrc.txt"), ("ls tests/mvsrc.txt\nls: cannot access 'tests/mvsrc.txt': No such file or directory\n", 1))
    check("the new name has the content", s.run("cat tests/mvdst.txt"), "cat tests/mvdst.txt\n" + hello_txt)

    check("mv into an existing directory", s.run("mv tests/mvdst.txt tests/mvdir"),
          "mv tests/mvdst.txt tests/mvdir\n")
    check("...lands at DIR/basename(SRC)", s.run("cat tests/mvdir/mvdst.txt"),
          "cat tests/mvdir/mvdst.txt\n" + hello_txt)

    check("mv a file onto itself", s.run_status("mv tests/mvdir/mvdst.txt tests/mvdir/mvdst.txt"), ("mv tests/mvdir/mvdst.txt tests/mvdir/mvdst.txt\n"
          "mv: 'tests/mvdir/mvdst.txt' and 'tests/mvdir/mvdst.txt' are the same file\n", 1))
    check("mv a directory into its own descendant", s.run_status("mv tests/mvdir tests/mvdir/sub"), ("mv tests/mvdir tests/mvdir/sub\n"
          "mv: cannot move 'tests/mvdir' to a subdirectory of itself, 'tests/mvdir/sub'\n", 1))

    s.run("cp tests/hello.txt tests/repl1.txt")
    s.run("cp tests/notes.txt tests/repl2.txt")
    check("mv replaces an existing plain-file destination", s.run("mv tests/repl1.txt tests/repl2.txt"),
          "mv tests/repl1.txt tests/repl2.txt\n")
    check("...the destination now holds the source's content", s.run("cat tests/repl2.txt"),
          "cat tests/repl2.txt\n" + hello_txt)

    check("mv refuses to replace a file with a directory", s.run_status("mv tests/mvdir tests/repl2.txt"), ("mv tests/mvdir tests/repl2.txt\nmv: cannot move 'tests/mvdir' to 'tests/repl2.txt': File exists\n", 1))

    check("mv onto a missing directory with a trailing slash", s.run_status("mv tests/hello.txt tests/nosuchdir/"), ("mv tests/hello.txt tests/nosuchdir/\nmv: cannot move 'tests/hello.txt' to 'tests/nosuchdir/': Not a directory\n", 1))

    s.run("mkdir tests/multidst")
    s.run("cp tests/hello.txt tests/msrc1.txt")
    s.run("cp tests/notes.txt tests/msrc2.txt")
    check("mv with several sources into a directory",
          s.run("mv tests/msrc1.txt tests/msrc2.txt tests/multidst"),
          "mv tests/msrc1.txt tests/msrc2.txt tests/multidst\n")
    check("...first source landed", s.run("cat tests/multidst/msrc1.txt"),
          "cat tests/multidst/msrc1.txt\n" + hello_txt)
    check("...second source landed", s.run("cat tests/multidst/msrc2.txt"),
          "cat tests/multidst/msrc2.txt\n" + notes_txt)
    check("mv with several sources onto a non-directory is a usage error", s.run_status("mv tests/x tests/y tests/z"), ("mv tests/x tests/y tests/z\n"
          "mv: target 'tests/z' is not a directory\n", 1))

    # ================================================================= stat
    def stat_block(name, size, kind, ro, exe, dt):
        """`dt` is the local time as `stat` shows it. (No accessed date: `stat` leaves it out.)"""
        return (
            f"  File: {name}\n"
            f"  Size: {size:<12} Type: {kind}\n"
            f" Attrs: read-only={ro}  exec={exe}\n"
            f"Modify: {dt}\n"
            f"Create: {dt}\n"
        )

    check("stat a bundled file (built at image-build time)", s.run("stat tests/hello.txt"),
          "stat tests/hello.txt\n" +
          stat_block("tests/hello.txt", len(hello_txt), "regular file", "no", "no", BUILD_STAMP))

    check("stat the root", s.run("stat /"),
          "stat /\n" + stat_block("/", 0, "directory", "no", "no", "1979-12-31 19:00:00 EST"))  # the FAT epoch, UTC, in Toronto

    # Files and directories the kernel creates are stamped from the real-time clock, in UTC. QEMU sets the RTC
    # from the host's clock, so a stamp must fall within a few seconds of the host's own time (FAT keeps
    # 2-second steps, hence the slack).
    def stat_of(name):
        """`stat`'s two timestamps for `name`: (modify, create), and the rest of the block."""
        out = s.run(f"stat {name}").split("\n")
        fields = {line.split(":", 1)[0].strip(): line.split(":", 1)[1].strip() for line in out[1:] if ":" in line}
        return fields["Modify"], fields["Create"], fields

    def size_type(fields):
        """`  Size: 12           Type: regular file` -> ("12", "regular file")."""
        size, kind = fields["Size"].split("Type:")
        return size.strip(), kind.strip()

    def epoch(stamp):
        """`YYYY-MM-DD HH:MM:SS ZONE` as `stat` shows it (local time, America/Toronto) -> Unix seconds."""
        text, abbreviation = stamp.rsplit(" ", 1)
        naive = datetime.strptime(text, "%Y-%m-%d %H:%M:%S")
        # The abbreviation says which side of a daylight-saving change a repeated hour is on.
        fold = 0 if naive.replace(tzinfo=TORONTO, fold=0).tzname() == abbreviation else 1
        return naive.replace(tzinfo=TORONTO, fold=fold).timestamp()

    def recent(stamp, before, after, slack=4):
        return before - slack <= epoch(stamp) <= after + slack

    before = time.time()
    s.run("mkdir tests/statdir")
    s.run("cp tests/hello.txt tests/statfresh.txt")
    after = time.time()

    modify, create, fields = stat_of("tests/statdir")
    check("a new directory: size and type", size_type(fields), ("0", "directory"))
    check("...is stamped from the RTC (created and modified now, UTC)",
          (recent(modify, before, after), recent(create, before, after)), (True, True))

    modify, create, fields = stat_of("tests/statfresh.txt")
    check("a copied file: size and type", size_type(fields), (str(len(hello_txt)), "regular file"))
    check("...is stamped from the RTC too", (recent(modify, before, after), recent(create, before, after)), (True, True))

    # Writing moves the modify time on and leaves the creation time alone.
    s.run("chmod +x tests/spin")
    s.run("tests/spin 3")
    before = time.time()
    s.run("echo more >> tests/statfresh.txt")
    after = time.time()
    modify2, create2, _fields = stat_of("tests/statfresh.txt")
    check("appending advances the modify time to now", recent(modify2, before, after), True)
    check("...by at least the wait, since the copy", epoch(modify2) - epoch(modify) >= 2, True)
    check("...and keeps the creation time", create2, create)

    before = time.time()
    s.run("echo replaced > tests/statfresh.txt")
    after = time.time()
    modify3, _create3, _fields = stat_of("tests/statfresh.txt")
    check("rewriting a file stamps its modify time", recent(modify3, before, after), True)

    before = time.time()
    s.run("echo made > tests/statredir.txt")
    after = time.time()
    modify4, create4, _fields = stat_of("tests/statredir.txt")
    check("a file created by a redirect is stamped the same way",
          (recent(modify4, before, after), recent(create4, before, after)), (True, True))

    # The bundled fixtures keep their fixed build-time stamp: nothing rewrote them.
    # Reading never rewrites a directory entry (Linux's `noatime`), so nothing has changed `tests/hello.txt`'s stamps
    # in all the times it has been read by now. (`stat` no longer shows the accessed date, which would have said the same.)
    modify, create, _fields = stat_of("tests/hello.txt")
    check("a bundled file still has its build-time stamp, though it has been read many times",
          (modify, create), (BUILD_STAMP, BUILD_STAMP))

    check("stat a missing file", s.run_status("stat tests/nosuchstat"), ("stat tests/nosuchstat\nstat: cannot stat 'tests/nosuchstat': No such file or directory\n", 1))

    # ================================================================= cp: multi-source + directory dest
    s.run("mkdir tests/cpdst")
    check("cp with several sources into a directory", s.run("cp tests/hello.txt tests/notes.txt tests/cpdst"),
          "cp tests/hello.txt tests/notes.txt tests/cpdst\n")
    check("...first source copied", s.run("cat tests/cpdst/hello.txt"), "cat tests/cpdst/hello.txt\n" + hello_txt)
    check("...second source copied", s.run("cat tests/cpdst/notes.txt"), "cat tests/cpdst/notes.txt\n" + notes_txt)

    # ================================================================= chmod: multi-file, -R
    s.run("cp tests/hello.txt tests/chmod1.txt")
    s.run("cp tests/hello.txt tests/chmod2.txt")
    check("chmod on several files at once", s.run("chmod -w tests/chmod1.txt tests/chmod2.txt"),
          "chmod -w tests/chmod1.txt tests/chmod2.txt\n")
    check("...first file is now read-only", s.run_status("cp tests/hello.txt tests/chmod1.txt"), ("cp tests/hello.txt tests/chmod1.txt\ncp: cannot create regular file 'tests/chmod1.txt': Permission denied\n", 1))
    check("...second file is now read-only", s.run_status("cp tests/hello.txt tests/chmod2.txt"), ("cp tests/hello.txt tests/chmod2.txt\ncp: cannot create regular file 'tests/chmod2.txt': Permission denied\n", 1))

    s.run("mkdir tests/rtree")
    s.run("mkdir tests/rtree/sub")
    s.run("cp tests/hello.txt tests/rtree/f1.txt")
    s.run("cp tests/hello.txt tests/rtree/sub/f2.txt")
    check("chmod -w -R recurses into a directory", s.run("chmod -w -R tests/rtree"), "chmod -w -R tests/rtree\n")
    check("...a direct child is now read-only", s.run_status("cp tests/hello.txt tests/rtree/f1.txt"), ("cp tests/hello.txt tests/rtree/f1.txt\ncp: cannot create regular file 'tests/rtree/f1.txt': Permission denied\n", 1))
    check("...and so is a grandchild, two levels down", s.run_status("cp tests/hello.txt tests/rtree/sub/f2.txt"), ("cp tests/hello.txt tests/rtree/sub/f2.txt\ncp: cannot create regular file 'tests/rtree/sub/f2.txt': Permission denied\n", 1))

    # ================================================================= trivial/moderate flags
    # The shell's own "start a fresh line before drawing the next prompt" behavior (used for
    # segfault/error messages since Step 2) also fires here, since -n genuinely leaves the cursor
    # mid-line -- that trailing \n is the shell's, not echo's own (echo itself wrote none).
    check("echo -n suppresses the trailing newline", s.run("echo -n hello"), "echo -n hello\nhello\n")

    max_line = max(len(line) for line in hello_txt.splitlines())
    check("wc -L reports the longest line", s.run(f"wc -L tests/hello.txt"),
          f"wc -L tests/hello.txt\n{max_line} tests/hello.txt\n")

    h_lines, h_words, h_bytes = len(hello_txt.splitlines()), len(hello_txt.split()), len(hello_txt)
    n_lines, n_words, n_bytes = len(notes_txt.splitlines()), len(notes_txt.split()), len(notes_txt)
    check("wc with several files prints one line per file plus a total", s.run("wc tests/hello.txt tests/notes.txt"),
          "wc tests/hello.txt tests/notes.txt\n"
          f"{h_lines} {h_words} {h_bytes} tests/hello.txt\n"
          f"{n_lines} {n_words} {n_bytes} tests/notes.txt\n"
          f"{h_lines + n_lines} {h_words + n_words} {h_bytes + n_bytes} total\n")

    bin_names = sorted(os.listdir(ctx.bin_dir))
    # Directory operands are listed in name order (Stage 18; before it, in the order given), each under its own header.
    check("ls with several directory operands", s.run("ls tests/docs bin"),
          "ls tests/docs bin\nbin:\n" + "".join(f"{n}\n" for n in bin_names) + "\ntests/docs:\nexample.txt\n")

    ex_size = len(example_txt)
    grouped = s.run("ls -lF tests/docs").split("\n", 1)[1]
    separate = s.run("ls -l -F tests/docs").split("\n", 1)[1]
    check("ls -lF is ls -l -F", (grouped == separate, "-w-" in grouped), (True, True))
    check("ls -Fl groups in either order", s.run("ls -Fl /").split("\n", 1)[1], s.run("ls -lF /").split("\n", 1)[1])
    check("ls -1F: -1 is accepted", "bin/" in s.run("ls -1F /"), True)
    check("ls -lx names the bad letter", s.run_status("ls -lx"), ("ls -lx\nls: invalid option -- 'x'\nTry 'ls --help' for more information.\n", 1))
    check("ls --long is not a supported option", s.run_status("ls --long"), ("ls --long\nls: unrecognized option '--long'\nTry 'ls --help' for more information.\n", 1))
    check("ls -l", s.run("ls -l tests/docs"),
          f"ls -l tests/docs\n-w- {ex_size:>10} example.txt\n")

    # As with `echo -n` above: the shell fills in the missing trailing newline before its prompt,
    # since the fixture's first 5 bytes don't happen to end in one.
    check("head -c", s.run("head -c 5 tests/hello.txt"), "head -c 5 tests/hello.txt\n" + hello_txt[:5] + "\n")
    check("tail -c", s.run("tail -c 5 tests/hello.txt"), "tail -c 5 tests/hello.txt\n" + hello_txt[-5:])
    check("head -c and -n together is a usage error", s.run_status("head -n 1 -c 1 tests/hello.txt"), ("head -n 1 -c 1 tests/hello.txt\nhead: options '-n' and '-c' are mutually exclusive\nTry 'head --help' for more information.\n", 1))

    # ================================================================= --help
    # (mkdir, rm, mv, cp and ls have more flags since Stage 18: their `--help` is checked in `flags.py`.)
    check("stat --help", s.run("stat --help"), "stat --help\nusage: stat FILE...\n")
    # (wc, head, tail, echo and cat gained flags in Stage 18: their `--help` is checked in `textflags.py`.)
    check("hexdump --help", s.run("hexdump --help"), "hexdump --help\nusage: hexdump [file]\n")
    check("true --help", s.run("true --help"), "true --help\nusage: true\n")
    check("false --help", s.run("false --help"), "false --help\nusage: false\n")
    check("pwd --help", s.run("pwd --help"), "pwd --help\nusage: pwd\n")
    check("clear --help", s.run("clear --help"), "clear --help\nusage: clear\n")

    # ================================================================= GNU-shaped diagnostics
    # Option and operand errors: GNU's wording, then its `Try` line. (Bare `prog: path: reason` stays
    # for cat/wc/tee, where GNU words it that way too.)
    def try_line(prog):
        return f"Try '{prog} --help' for more information.\n"

    def refused(cmd, message):
        prog = cmd.split()[0]
        check(f"{cmd!r} is refused", s.run_status(cmd), (f"{cmd}\n{prog}: {message}\n{try_line(prog)}", 1))

    refused("cat -x", "invalid option -- 'x'")
    refused("cat --foo", "unrecognized option '--foo'")
    refused("wc -lx", "invalid option -- 'x'")
    refused("tee -z", "invalid option -- 'z'")
    refused("rm -rq x", "invalid option -- 'q'")
    refused("hexdump tests/hello.txt tests/notes.txt", "extra operand 'tests/notes.txt'")
    refused("reboot now", "extra operand 'now'")
    refused("rm", "missing operand")
    refused("mkdir", "missing operand")
    refused("stat", "missing operand")
    refused("chmod", "missing operand")
    refused("chmod +x", "missing operand after '+x'")
    refused("cp", "missing file operand")
    refused("cp tests/hello.txt", "missing destination file operand after 'tests/hello.txt'")
    refused("mv", "missing file operand")
    refused("mv tests/hello.txt", "missing destination file operand after 'tests/hello.txt'")

    # Failures reading input, worded as GNU's head/tail do.
    check("head on a missing file", s.run_status("head tests/nosuch.txt"), ("head tests/nosuch.txt\nhead: cannot open 'tests/nosuch.txt' for reading: No such file or directory\n", 1))
    check("tail on a missing file", s.run_status("tail tests/nosuch.txt"), ("tail tests/nosuch.txt\ntail: cannot open 'tests/nosuch.txt' for reading: No such file or directory\n", 1))
    check("head on a directory", s.run_status("head tests"), ("head tests\nhead: error reading 'tests': Is a directory\n", 1))
    check("cat on a missing file stays in GNU's bare form", s.run_status("cat tests/nosuch.txt"), ("cat tests/nosuch.txt\ncat: tests/nosuch.txt: No such file or directory\n", 1))

    # ================================================================= one argument parser (getargs)
    # `--` ends the options, so a file named like an option can be named; a lone `-` is a plain operand.
    s.run("cd tests")
    s.run("echo dash > ./-dash.txt")
    check("cat -- -dash.txt", s.run("cat -- -dash.txt"), "cat -- -dash.txt\ndash\n")
    check("cat -dash.txt is an option", s.run_status("cat -dash.txt"), ("cat -dash.txt\ncat: invalid option -- 'd'\nTry 'cat --help' for more information.\n", 1))
    check("cat - is an ordinary file name", s.run_status("cat -"), ("cat -\ncat: -: No such file or directory\n", 1))
    check("rm -- -dash.txt", s.run("rm -- -dash.txt"), "rm -- -dash.txt\n")
    check("...it is gone", s.run_status("cat -- -dash.txt"), ("cat -- -dash.txt\ncat: -dash.txt: No such file or directory\n", 1))
    s.run("cd ..")

    # Option values, however they are written.
    first = hello_txt.split("\n")[0] + "\n"
    last = hello_txt.rstrip("\n").split("\n")[-1] + "\n"
    for cmd, want in [
        ("head -n1 tests/hello.txt", first),
        ("head -n 1 tests/hello.txt", first),
        ("head --lines=1 tests/hello.txt", first),
        ("head --lines 1 tests/hello.txt", first),
        ("head -c3 tests/hello.txt", hello_txt[:3]),
        ("head --bytes=3 tests/hello.txt", hello_txt[:3]),
        ("tail -n1 tests/hello.txt", last),
        ("tail --lines=1 tests/hello.txt", last),
        ("tail -c3 tests/hello.txt", hello_txt[-3:]),
        ("tail --bytes 3 tests/hello.txt", hello_txt[-3:]),
        ("head tests/hello.txt -n1", first),  # options may follow the operand
    ]:
        # Output that stops mid-line gets the prompt's newline (see `uart_ensure_newline`).
        check(cmd, s.run(cmd), f"{cmd}\n{want}" + ("" if want.endswith("\n") else "\n"))
    check("head --lines with no value", s.run_status("head --lines"), ("head --lines\nhead: option '--lines' requires an argument\n"
          "Try 'head --help' for more information.\n", 1))
    check("head --lines=x", s.run_status("head --lines=x tests/hello.txt"), ("head --lines=x tests/hello.txt\nhead: invalid number of lines: 'x'\n", 1))

    # Flags apply to the whole command line, wherever they are written.
    s.run("mkdir tests/optdir")
    check("rm dir -r (the flag after the operand)", s.run("rm tests/optdir -r"), "rm tests/optdir -r\n")
    check("...the directory went", s.run_status("ls tests/optdir"), ("ls tests/optdir\nls: cannot access 'tests/optdir': No such file or directory\n", 1))
    s.run("echo x > tests/teeapp.txt")
    check("tee file -a (the flag after the file) appends", s.run("echo y | tee tests/teeapp.txt -a"),
          "echo y | tee tests/teeapp.txt -a\ny\n")
    check("...so the file has both lines", s.run("cat tests/teeapp.txt"), "cat tests/teeapp.txt\nx\ny\n")
    grouped = s.run("wc -lc tests/hello.txt").split("\n", 1)[1]
    separate = s.run("wc -l -c tests/hello.txt").split("\n", 1)[1]
    trailing = s.run("wc tests/hello.txt -lc").split("\n", 1)[1]
    check("wc -lc, wc -l -c and wc FILE -lc agree", grouped == separate == trailing, True)

    # chmod's mode is the first operand even though `-x` looks like an option.
    s.run("cp tests/hello.txt tests/chmopt.txt")
    check("chmod +x with -R after the file", s.run("chmod +x tests/chmopt.txt -R"), "chmod +x tests/chmopt.txt -R\n")
    check("...it is executable", "chmopt.txt*" in s.run("ls -F tests"), True)
    check("chmod -- -x file: -x is the mode", s.run("chmod -- -x tests/chmopt.txt"), "chmod -- -x tests/chmopt.txt\n")
    check("...it no longer is", "chmopt.txt*" in s.run("ls -F tests"), False)
    check("chmod -q file is an unknown option", s.run_status("chmod -q tests/chmopt.txt"), ("chmod -q tests/chmopt.txt\nchmod: invalid option -- 'q'\nTry 'chmod --help' for more information.\n", 1))
