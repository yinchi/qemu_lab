"""Step 10's expanded scope: `mkdir`/`rm`/`mv`/`stat` (new programs), multi-operand `cp`/`chmod`/`mv`,
`chmod -R`, `rm -f`, the trivial/moderate flag catch-up (`echo -n`, `wc -L`/multi-file, `ls` multi-dir/`-l`,
`head -c`/`tail -c`), and `--help` on a representative sample of programs. See `Stage12.md`'s Step 10
section and `docs/progs.md`.

Fixtures: `disk/tests/hello.txt`, `disk/tests/notes.txt`, `disk/tests/docs/example.txt` (existing), plus
`disk/tests/tree/` (new: a 3-level directory tree for `rm -r`) -- `a.txt`, `sub1/b.txt`, `sub1/sub2/c.txt`.
"""

from datetime import datetime

from cases.core_utils import BINARIES

# `folder_to_img.sh` pins mtools' timestamps to this Unix time (2026-01-01T00:00:00Z) for
# reproducible builds; mtools converts it to *local* time when stamping FAT dates (there's no
# timezone in a FAT timestamp), so the expected wall-clock value depends on the host's zone --
# computed the same way here, rather than hardcoded, so this test isn't tied to one timezone.
BUILD_STAMP = datetime.fromtimestamp(1767225600).strftime("%Y-%m-%d %H:%M:%S")


def run(ctx):
    s, check = ctx.s, ctx.check
    hello_txt = ctx.fixture("hello.txt")
    notes_txt = ctx.fixture("notes.txt")
    example_txt = ctx.fixture("docs/example.txt")

    # ================================================================= mkdir
    check("mkdir creates a directory", s.run("mkdir tests/newdir"), "mkdir tests/newdir\n")
    check("ls on the new (empty) directory", s.run("ls tests/newdir"), "ls tests/newdir\n")
    check("mkdir again is File exists", s.run("mkdir tests/newdir"),
          "mkdir tests/newdir\nmkdir: tests/newdir: File exists\nexit 1\n")
    check("mkdir under a missing parent", s.run("mkdir tests/nosuchparent/x"),
          "mkdir tests/nosuchparent/x\nmkdir: tests/nosuchparent/x: No such file or directory\nexit 1\n")
    check("mkdir multi-operand continues past a failure", s.run("mkdir tests/newdir tests/newdir2"),
          "mkdir tests/newdir tests/newdir2\nmkdir: tests/newdir: File exists\nexit 1\n")
    check("...but the second operand was still created", s.run("ls tests/newdir2"), "ls tests/newdir2\n")

    # ================================================================= rm
    check("rm refuses a directory without -r", s.run("rm tests/newdir"),
          "rm tests/newdir\nrm: tests/newdir: Is a directory\nexit 1\n")
    check("rm -r removes an empty directory", s.run("rm -r tests/newdir"), "rm -r tests/newdir\n")
    check("...it's really gone", s.run("ls tests/newdir"),
          "ls tests/newdir\nls: tests/newdir: No such file or directory\nexit 1\n")
    check("rm on a missing file", s.run("rm tests/nosuchfile"),
          "rm tests/nosuchfile\nrm: tests/nosuchfile: No such file or directory\nexit 1\n")
    check("rm -f on a missing file is silent", s.run("rm -f tests/nosuchfile"), "rm -f tests/nosuchfile\n")
    check("rm .", s.run("rm ."), "rm .\nrm: .: Invalid argument\nexit 1\n")
    check("rm ..", s.run("rm .."), "rm ..\nrm: ..: Invalid argument\nexit 1\n")
    check("rm /", s.run("rm /"), "rm /\nrm: /: Invalid argument\nexit 1\n")
    check("rm -r / is refused too", s.run("rm -r /"), "rm -r /\nrm: /: Invalid argument\nexit 1\n")
    check("rm -r on a populated 3-level tree", s.run("rm -r tests/tree"), "rm -r tests/tree\n")
    check("...the whole tree is gone", s.run("ls tests/tree"),
          "ls tests/tree\nls: tests/tree: No such file or directory\nexit 1\n")

    s.run("cp tests/hello.txt tests/rmtarget.txt")
    s.run("chmod -w tests/rmtarget.txt")
    check("rm deletes a read-only file without -f (deletion isn't gated by the file's own attrs)",
          s.run("rm tests/rmtarget.txt"), "rm tests/rmtarget.txt\n")

    # ================================================================= mv
    s.run("mkdir tests/mvdir")
    s.run("cp tests/hello.txt tests/mvsrc.txt")
    check("mv renames", s.run("mv tests/mvsrc.txt tests/mvdst.txt"), "mv tests/mvsrc.txt tests/mvdst.txt\n")
    check("the old name is gone", s.run("ls tests/mvsrc.txt"),
          "ls tests/mvsrc.txt\nls: tests/mvsrc.txt: No such file or directory\nexit 1\n")
    check("the new name has the content", s.run("cat tests/mvdst.txt"), "cat tests/mvdst.txt\n" + hello_txt)

    check("mv into an existing directory", s.run("mv tests/mvdst.txt tests/mvdir"),
          "mv tests/mvdst.txt tests/mvdir\n")
    check("...lands at DIR/basename(SRC)", s.run("cat tests/mvdir/mvdst.txt"),
          "cat tests/mvdir/mvdst.txt\n" + hello_txt)

    check("mv a file onto itself", s.run("mv tests/mvdir/mvdst.txt tests/mvdir/mvdst.txt"),
          "mv tests/mvdir/mvdst.txt tests/mvdir/mvdst.txt\nmv: tests/mvdir/mvdst.txt: Invalid argument\nexit 1\n")
    check("mv a directory into its own descendant", s.run("mv tests/mvdir tests/mvdir/sub"),
          "mv tests/mvdir tests/mvdir/sub\nmv: tests/mvdir: Invalid argument\nexit 1\n")

    s.run("cp tests/hello.txt tests/repl1.txt")
    s.run("cp tests/notes.txt tests/repl2.txt")
    check("mv replaces an existing plain-file destination", s.run("mv tests/repl1.txt tests/repl2.txt"),
          "mv tests/repl1.txt tests/repl2.txt\n")
    check("...the destination now holds the source's content", s.run("cat tests/repl2.txt"),
          "cat tests/repl2.txt\n" + hello_txt)

    check("mv refuses to replace a file with a directory", s.run("mv tests/mvdir tests/repl2.txt"),
          "mv tests/mvdir tests/repl2.txt\nmv: tests/mvdir: File exists\nexit 1\n")

    check("mv onto a missing directory with a trailing slash", s.run("mv tests/hello.txt tests/nosuchdir/"),
          "mv tests/hello.txt tests/nosuchdir/\nmv: tests/nosuchdir/: Not a directory\nexit 1\n")

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
    check("mv with several sources onto a non-directory is a usage error", s.run("mv tests/x tests/y tests/z"),
          "mv tests/x tests/y tests/z\n"
          "usage: mv SRC SRC... DIR  (more than one source requires an existing directory destination)\nexit 1\n")

    # ================================================================= stat
    def stat_block(name, size, kind, ro, exe, dt):
        date = dt.split(" ")[0]
        return (
            f"  File: {name}\n"
            f"  Size: {size:<12} Type: {kind}\n"
            f" Attrs: read-only={ro}  exec={exe}\n"
            f"Modify: {dt}\n"
            f"Create: {dt}\n"
            f"Access: {date}\n"
        )

    check("stat a bundled file (built at image-build time)", s.run("stat tests/hello.txt"),
          "stat tests/hello.txt\n" +
          stat_block("tests/hello.txt", len(hello_txt), "regular file", "no", "no", BUILD_STAMP))

    check("stat the root", s.run("stat /"),
          "stat /\n" + stat_block("/", 0, "directory", "no", "no", "1980-01-01 00:00:00"))

    s.run("mkdir tests/statdir")
    check("stat a directory created this session (no RTC yet: the FAT epoch)", s.run("stat tests/statdir"),
          "stat tests/statdir\n" +
          stat_block("tests/statdir", 0, "directory", "no", "no", "1980-01-01 00:00:00"))

    s.run("cp tests/hello.txt tests/statfresh.txt")
    check("stat a file written this session", s.run("stat tests/statfresh.txt"),
          "stat tests/statfresh.txt\n" +
          stat_block("tests/statfresh.txt", len(hello_txt), "regular file", "no", "no", "1980-01-01 00:00:00"))

    check("stat a missing file", s.run("stat tests/nosuchstat"),
          "stat tests/nosuchstat\nstat: tests/nosuchstat: No such file or directory\nexit 1\n")

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
    check("...first file is now read-only", s.run("cp tests/hello.txt tests/chmod1.txt"),
          "cp tests/hello.txt tests/chmod1.txt\ncp: tests/chmod1.txt: Permission denied\nexit 1\n")
    check("...second file is now read-only", s.run("cp tests/hello.txt tests/chmod2.txt"),
          "cp tests/hello.txt tests/chmod2.txt\ncp: tests/chmod2.txt: Permission denied\nexit 1\n")

    s.run("mkdir tests/rtree")
    s.run("mkdir tests/rtree/sub")
    s.run("cp tests/hello.txt tests/rtree/f1.txt")
    s.run("cp tests/hello.txt tests/rtree/sub/f2.txt")
    check("chmod -w -R recurses into a directory", s.run("chmod -w -R tests/rtree"), "chmod -w -R tests/rtree\n")
    check("...a direct child is now read-only", s.run("cp tests/hello.txt tests/rtree/f1.txt"),
          "cp tests/hello.txt tests/rtree/f1.txt\ncp: tests/rtree/f1.txt: Permission denied\nexit 1\n")
    check("...and so is a grandchild, two levels down", s.run("cp tests/hello.txt tests/rtree/sub/f2.txt"),
          "cp tests/hello.txt tests/rtree/sub/f2.txt\ncp: tests/rtree/sub/f2.txt: Permission denied\nexit 1\n")

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

    check("ls with several directory operands", s.run("ls tests/docs bin"),
          "ls tests/docs bin\ntests/docs:\nexample.txt\n\nbin:\n" + "".join(f"{n}.exe\n" for n in BINARIES))

    ex_size = len(example_txt)
    check("ls -l", s.run("ls -l tests/docs"),
          f"ls -l tests/docs\n-w- {ex_size:>10} example.txt\n")

    # As with `echo -n` above: the shell fills in the missing trailing newline before its prompt,
    # since the fixture's first 5 bytes don't happen to end in one.
    check("head -c", s.run("head -c 5 tests/hello.txt"), "head -c 5 tests/hello.txt\n" + hello_txt[:5] + "\n")
    check("tail -c", s.run("tail -c 5 tests/hello.txt"), "tail -c 5 tests/hello.txt\n" + hello_txt[-5:])
    check("head -c and -n together is a usage error", s.run("head -n 1 -c 1 tests/hello.txt"),
          "head -n 1 -c 1 tests/hello.txt\nusage: head [-n N | -c N] [file]\nexit 1\n")

    # ================================================================= --help
    check("mkdir --help", s.run("mkdir --help"), "mkdir --help\nusage: mkdir DIR...\n")
    check("rm --help", s.run("rm --help"), "rm --help\n"
          "usage: rm [-r] [-f] PATH...\n"
          "  -r  remove directories and their contents recursively\n"
          "  -f  ignore nonexistent operands, never prompt\n")
    check("mv --help", s.run("mv --help"), "mv --help\nusage: mv SRC... DST\n")
    check("stat --help", s.run("stat --help"), "stat --help\nusage: stat FILE...\n")
    check("cp --help", s.run("cp --help"), "cp --help\nusage: cp SRC... DST\n")
    check("ls --help", s.run("ls --help"), "ls --help\n"
          "usage: ls [-F] [-l] [dir...]\n"
          "  -F  append / to directories and * to executable files\n"
          "  -l  long format: d/w/x flags, size, name\n")
    check("wc --help", s.run("wc --help"), "wc --help\n"
          "usage: wc [-l] [-w] [-c] [-L] [file...]\n"
          "  -l  count lines\n"
          "  -w  count words\n"
          "  -c  count bytes\n"
          "  -L  report the longest line's length\n")
    check("head --help", s.run("head --help"), "head --help\n"
          "usage: head [-n N | -c N] [file]\n"
          "  -n N  print the first N lines (default 10)\n"
          "  -c N  print the first N bytes\n")
    check("tail --help", s.run("tail --help"), "tail --help\n"
          "usage: tail [-n N | -c N] [file]\n"
          "  -n N  print the last N lines (default 10)\n"
          "  -c N  print the last N bytes\n")
    check("echo --help", s.run("echo --help"), "echo --help\n"
          "usage: echo [-n] args...\n"
          "  -n  suppress the trailing newline\n")
    check("cat --help", s.run("cat --help"), "cat --help\nusage: cat [file...]\n")
    check("hexdump --help", s.run("hexdump --help"), "hexdump --help\nusage: hexdump [file]\n")
    check("true --help", s.run("true --help"), "true --help\nusage: true\n")
    check("false --help", s.run("false --help"), "false --help\nusage: false\n")
    check("pwd --help", s.run("pwd --help"), "pwd --help\nusage: pwd\n")
    check("clear --help", s.run("clear --help"), "clear --help\nusage: clear\n")
