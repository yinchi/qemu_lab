"""The core utilities from `user/progs` (see `docs/progs.md`), exercised through the shell: `echo`, `cat`,
`ls`, `cp`, `head`, `tail`, `wc`, `hexdump`, `true`, `false`, `chmod`, the launcher's error paths, and a
program reading typed lines from stdin. This is the regression baseline the later Steps must keep green.

Fixtures are the files under `disk/tests/` (on the image as `/tests/`); files the cases write land there
too, on the private copy of the image.
"""

import os
import subprocess

from harness import CTRL_D, BACKSPACE, dir_attr, mcopy_out, text_bands


def run(ctx):
    s, check = ctx.s, ctx.check
    hello_txt = ctx.fixture("hello.txt")
    lines = hello_txt.splitlines(keepends=True)
    words = len(hello_txt.split())
    host_hexdump = subprocess.run(
        ["hexdump", "-C", ctx.fixture_path("data.bin")], capture_output=True, text=True, check=True
    ).stdout

    # --- programs ---
    check("echo", s.run("echo hello world"), "echo hello world\nhello world\n")
    check("hello", s.run("hello"), "hello\nhello from userspace\n")
    check(
        "crash",
        s.run("crash"),
        "crash\nabout to crash\nSegmentation fault (address 0xffff800000000000, ESR_EL1 0x92000004)\nexit 139\n",
    )

    # --- launcher ---
    check("unknown program", s.run("nosuch"), "nosuch\nnosuch: command not found\n")

    # --- cat / ls ---
    check("cat file", s.run("cat tests/hello.txt"), "cat tests/hello.txt\n" + hello_txt)
    check("cat subdirectory file", s.run("cat tests/docs/example.txt"),
          "cat tests/docs/example.txt\na file in a subdirectory\n")
    check("cat two files", s.run("cat tests/docs/example.txt tests/docs/example.txt"),
          "cat tests/docs/example.txt tests/docs/example.txt\n" + "a file in a subdirectory\n" * 2)
    check("cat missing", s.run("cat tests/nosuch.txt"),
          "cat tests/nosuch.txt\ncat: tests/nosuch.txt: No such file or directory\nexit 1\n")
    check("cat directory", s.run("cat tests/docs"), "cat tests/docs\ncat: tests/docs: Is a directory\nexit 1\n")
    check("ls", s.run("ls"), "ls\nbin\netc\nfonts\nroot\ntests\ntmp\n")
    check("ls -F", s.run("ls -F"), "ls -F\nbin/\netc/\nfonts/\nroot/\ntests/\ntmp/\n")
    # `bin/` is the one directory meant to grow as core utilities are added (unlike the fixed
    # fixtures under `tests/`), so the expected list is derived from what `just disk` actually
    # staged rather than a hand-maintained one -- alphabetical, matching `folder_to_img.sh`'s own
    # sort-then-copy order, which is what ends up as the FAT on-disk order `ls` reports.
    bin_names = sorted(n[:-4] for n in os.listdir(ctx.bin_dir) if n.endswith(".exe"))
    check("ls -F bin", s.run("ls -F bin"), "ls -F bin\n" + "".join(f"{n}.exe*\n" for n in bin_names))
    check("ls file", s.run("ls tests/hello.txt"),
          "ls tests/hello.txt\nls: cannot open directory 'tests/hello.txt': Not a directory\nexit 1\n")
    check("ls bad option", s.run("ls -x"), "ls -x\nls: invalid option -- 'x'\nTry 'ls --help' for more information.\nexit 1\n")

    # --- cp ---
    check("cp", s.run("cp tests/hello.txt tests/copy.txt"), "cp tests/hello.txt tests/copy.txt\n")
    check("cat copy", s.run("cat tests/copy.txt"), "cat tests/copy.txt\n" + hello_txt)
    check("cp binary", s.run("cp tests/data.bin tests/data2.bin"), "cp tests/data.bin tests/data2.bin\n")
    check("cp missing source", s.run("cp tests/nosuch.txt tests/x.txt"),
          "cp tests/nosuch.txt tests/x.txt\ncp: cannot stat 'tests/nosuch.txt': No such file or directory\nexit 1\n")
    check("cp directory source keeps dst", s.run("cp tests/docs tests/copy.txt"),
          "cp tests/docs tests/copy.txt\ncp: -r not specified; omitting directory 'tests/docs'\nexit 1\n")
    check("cp overwrite shorter", s.run("cp tests/docs/example.txt tests/copy.txt"),
          "cp tests/docs/example.txt tests/copy.txt\n")
    check("cat overwritten", s.run("cat tests/copy.txt"), "cat tests/copy.txt\na file in a subdirectory\n")

    # --- head / tail / wc / hexdump ---
    f = "tests/hello.txt"
    check("head -n 3", s.run(f"head -n 3 {f}"), f"head -n 3 {f}\n" + "".join(lines[:3]))
    check("head default", s.run(f"head {f}"), f"head {f}\n" + "".join(lines[:10]))
    check("head -n 0", s.run(f"head -n 0 {f}"), f"head -n 0 {f}\n")
    check("tail -n 2", s.run(f"tail -n 2 {f}"), f"tail -n 2 {f}\n" + "".join(lines[-2:]))
    check("tail default", s.run(f"tail {f}"), f"tail {f}\n" + "".join(lines[-10:]))
    check("tail -n 100", s.run(f"tail -n 100 {f}"), f"tail -n 100 {f}\n" + hello_txt)
    check("head bad count", s.run(f"head -n x {f}"), f"head -n x {f}\nhead: invalid number of lines: 'x'\nexit 1\n")
    check("head -c bad count", s.run(f"head -c x {f}"), f"head -c x {f}\nhead: invalid number of bytes: 'x'\nexit 1\n")
    check("tail bad count", s.run(f"tail -n -3 {f}"), f"tail -n -3 {f}\ntail: invalid number of lines: '-3'\nexit 1\n")
    check("head -n with no count", s.run("head -n"), "head -n\nhead: option requires an argument -- 'n'\nTry 'head --help' for more information.\nexit 1\n")
    check("cp a file onto itself", s.run("cp tests/hello.txt tests/hello.txt"),
          "cp tests/hello.txt tests/hello.txt\ncp: 'tests/hello.txt' and 'tests/hello.txt' are the same file\nexit 1\n")
    check("wc", s.run(f"wc {f}"), f"wc {f}\n{len(lines)} {words} {len(hello_txt)} {f}\n")
    check("wc -l", s.run(f"wc -l {f}"), f"wc -l {f}\n{len(lines)} {f}\n")
    check("wc -wc", s.run(f"wc -wc {f}"), f"wc -wc {f}\n{words} {len(hello_txt)} {f}\n")
    check("hexdump", s.run("hexdump tests/data.bin"), "hexdump tests/data.bin\n" + host_hexdump)
    check("hexdump of copy", s.run("hexdump tests/data2.bin"), "hexdump tests/data2.bin\n" + host_hexdump)

    # --- exit status ---
    check("true", s.run("true"), "true\n")
    check("false", s.run("false"), "false\nexit 1\n")

    # --- chmod ---
    check("chmod -x", s.run("chmod -x bin/hello.exe"), "chmod -x bin/hello.exe\n")
    check("run without exec bit", s.run("hello"), "hello\nhello: Permission denied\n")
    check("chmod +x", s.run("chmod +x bin/hello.exe"), "chmod +x bin/hello.exe\n")
    check("run with exec bit", s.run("hello"), "hello\nhello from userspace\n")
    check("chmod -w", s.run("chmod -w tests/copy.txt"), "chmod -w tests/copy.txt\n")
    check("cp onto read-only", s.run("cp tests/hello.txt tests/copy.txt"),
          "cp tests/hello.txt tests/copy.txt\ncp: cannot create regular file 'tests/copy.txt': Permission denied\nexit 1\n")
    check("chmod bad mode", s.run("chmod 755 tests/copy.txt"),
          "chmod 755 tests/copy.txt\nchmod: invalid mode: '755'\nexit 1\n")
    check("chmod missing", s.run("chmod +x tests/nosuch"),
          "chmod +x tests/nosuch\nchmod: cannot access 'tests/nosuch': No such file or directory\nexit 1\n")

    # --- tee ---
    check("tee copies stdin to stdout and a file", s.run("tee tests/tee1.txt < tests/hello.txt"),
          "tee tests/tee1.txt < tests/hello.txt\n" + hello_txt)
    check("...the file has the same content", s.run("cat tests/tee1.txt"), "cat tests/tee1.txt\n" + hello_txt)
    check("tee -a appends instead of truncating", s.run("tee -a tests/tee1.txt < tests/hello.txt"),
          "tee -a tests/tee1.txt < tests/hello.txt\n" + hello_txt)
    check("...the file now has it twice", s.run("cat tests/tee1.txt"), "cat tests/tee1.txt\n" + hello_txt * 2)
    check("tee with several files writes to all of them",
          s.run("tee tests/tee2.txt tests/tee3.txt < tests/hello.txt"),
          "tee tests/tee2.txt tests/tee3.txt < tests/hello.txt\n" + hello_txt)
    check("...first file", s.run("cat tests/tee2.txt"), "cat tests/tee2.txt\n" + hello_txt)
    check("...second file", s.run("cat tests/tee3.txt"), "cat tests/tee3.txt\n" + hello_txt)
    check("tee still passes stdin through even if a file can't be opened",
          s.run("tee tests/nosuchdir/x.txt < tests/hello.txt"),
          "tee tests/nosuchdir/x.txt < tests/hello.txt\n"
          "tee: tests/nosuchdir/x.txt: No such file or directory\n" + hello_txt + "exit 1\n")
    check("tee --help", s.run("tee --help"), "tee --help\n"
          "usage: tee [-a] [file...]\n"
          "  -a  append to each file instead of truncating it\n")

    # --- clear: the screen is emptied and the prompt comes back at the top ---
    s.run("cat tests/hello.txt")  # something on screen to clear
    check("clear: the serial terminal is cleared too", s.run("clear"), "clear\n\x1b[H\x1b[2J")
    bands = text_bands(s.screendump_settled())
    check("clear: only the new prompt is on screen, on the first row", [row for row, _ in bands], [0])

    # --- stdin: a program reading typed lines (Backspace absorbed, Ctrl+D ends) ---
    s.type("cat\n")
    s.wait_until(lambda t: t.endswith("cat\n"), "cat to start")
    s.type("hellp")
    s.keys([BACKSPACE])
    s.type("o\n")
    s.wait_until(lambda t: t.endswith("hello\nhello\n"), "cat to echo the line back")
    s.type("second line\n")
    s.wait_until(lambda t: t.endswith("second line\nsecond line\n"), "cat to echo the second line")
    s.keys([CTRL_D])
    check("cat stdin", s.wait_prompt(), "cat\nhello\nhello\nsecond line\nsecond line\n")

    s.type("wc\n")
    s.wait_until(lambda t: t.endswith("wc\n"), "wc to start")
    s.type("one two\nthree\n")
    s.wait_until(lambda t: t.endswith("three\n"), "wc to read")
    s.keys([CTRL_D])
    check("wc stdin", s.wait_prompt(), "wc\none two\nthree\n2 3 14\n")

    s.type("tail -n 1\n")
    s.wait_until(lambda t: t.endswith("tail -n 1\n"), "tail to start")
    s.type("a\nb\nc\n")
    s.wait_until(lambda t: t.endswith("c\n"), "tail to read")
    s.keys([CTRL_D])
    check("tail stdin", s.wait_prompt(), "tail -n 1\na\nb\nc\nc\n")


def verify_disk(ctx):
    check, img = ctx.check, ctx.img
    out = os.path.join(ctx.workdir, "out")
    os.makedirs(out, exist_ok=True)
    mcopy_out(img, "tests/copy.txt", os.path.join(out, "copy.txt"))
    mcopy_out(img, "tests/data2.bin", os.path.join(out, "data2.bin"))
    check("disk: copy.txt is the overwritten content",
          open(os.path.join(out, "copy.txt")).read(), "a file in a subdirectory\n")
    check("disk: data2.bin matches data.bin byte for byte",
          open(os.path.join(out, "data2.bin"), "rb").read(), open(ctx.fixture_path("data.bin"), "rb").read())
    check("disk: copy.txt is read-only", dir_attr(img, b"COPY    TXT") & 0x01, 0x01)
    check("disk: hello.exe has the exec bit again", dir_attr(img, b"HELLO   EXE") & 0x40, 0x40)
