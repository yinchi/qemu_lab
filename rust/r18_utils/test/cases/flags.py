"""Last updated: Stage 18, Step 2.

The flags Stage 18's tier adds to the file-management programs: `mkdir -p -v`, `cp -r -n -v`, `mv -n -v -f`, `rm -v -d`
and `ls -a -d -R -r -S -t -h` -- with `ls` hiding names that start with `.` unless `-a` is given, and listing a file operand
as itself. Everything here is the shell end to end; the size formatting of `-h` is also a host test (`human.rs`).

`ls -t` orders by modify time, and FAT time has a 2-second tick, so the files it sorts are made with real waits between
them (the guest's clock follows the host's); the time zone is UTC.
"""

import os
import time

ENVIRONMENT = "HOME=/\nPATH=/bin\nTZ=UTC\n"


def human(n):
    """GNU's `-h`: powers of 1024, rounded up, one decimal under 10 -- written independently of the Rust version."""
    if n < 1024:
        return str(n)
    for i, suffix in enumerate("KMGT", start=1):
        unit = 1024 ** i
        tenths = -(-n * 10 // unit)
        if tenths < 100:
            return f"{tenths // 10}.{tenths % 10}{suffix}"
        whole = -(-n // unit)
        if whole < 1024 or suffix == "T":
            return f"{whole}{suffix}"


def run(ctx):
    s, check = ctx.s, ctx.check

    def status(cmd, out, st):
        check(cmd, s.run_status(cmd), (f"{cmd}\n{out}", st))

    TRY = lambda prog: f"Try '{prog} --help' for more information.\n"

    # ================================================================= mkdir -p -v
    status("mkdir -p tests/pa/pb/pc", "", 0)
    status("ls tests/pa/pb", "pc\n", 0)
    status("mkdir -pv tests/pd/pe", "mkdir: created directory 'tests/pd'\nmkdir: created directory 'tests/pd/pe'\n", 0)
    status("mkdir -p tests/pa/pb", "", 0)  # already there
    status("mkdir -pv tests/pa/pb", "", 0)  # ...and nothing to say about it
    status("mkdir -pv tests/pa/pb/pf/", "mkdir: created directory 'tests/pa/pb/pf'\n", 0)  # a trailing slash
    status("mkdir -p tests//dbl//x", "", 0)
    status("mkdir -p /tmp/pq/rs", "", 0)
    status("ls /tmp/pq", "rs\n", 0)
    status("mkdir -p /", "", 0)
    status("mkdir -p tests/hello.txt", "mkdir: cannot create directory 'tests/hello.txt': File exists\n", 1)
    status("mkdir -p tests/hello.txt/x", "mkdir: cannot create directory 'tests/hello.txt': Not a directory\n", 1)
    status("mkdir tests/pa", "mkdir: cannot create directory 'tests/pa': File exists\n", 1)  # without -p, still an error
    status("mkdir tests/nodir/x", "mkdir: cannot create directory 'tests/nodir/x': No such file or directory\n", 1)
    status("mkdir -v tests/mv1", "mkdir: created directory 'tests/mv1'\n", 0)
    status("mkdir -p tests/pg tests/hello.txt tests/ph", "mkdir: cannot create directory 'tests/hello.txt': File exists\n", 1)
    status("ls tests/pg tests/ph", "tests/pg:\n\ntests/ph:\n", 0)  # the others were made
    status("mkdir", "mkdir: missing operand\n" + TRY("mkdir"), 1)
    check("mkdir --help", s.run("mkdir --help"),
          "mkdir --help\nusage: mkdir [-p] [-v] DIR...\n"
          "  -p  make missing parent directories, and do not fail if DIR exists as a directory\n"
          "  -v  print a message for each directory created\n")

    # ================================================================= cp -r -n -v
    s.run("mkdir -p tests/ct/sub")
    s.run("echo 1 > tests/ct/a")
    s.run("echo 2 > tests/ct/sub/b")
    s.run("echo h > tests/ct/.hid")
    status("cp tests/ct tests/cx", "cp: -r not specified; omitting directory 'tests/ct'\n", 1)
    status("cp -r tests/ct tests/cx", "", 0)
    check("cp -r copied the tree", s.run("cat tests/cx/a tests/cx/sub/b tests/cx/.hid"),
          "cat tests/cx/a tests/cx/sub/b tests/cx/.hid\n1\n2\nh\n")
    status("cp -R tests/ct tests/cx", "", 0)  # the destination is a directory now: copied *into* it
    check("...as tests/cx/ct", s.run("cat tests/cx/ct/a tests/cx/ct/sub/b"), "cat tests/cx/ct/a tests/cx/ct/sub/b\n1\n2\n")
    status("cp -rv tests/ct tests/cv",
           "'tests/ct' -> 'tests/cv'\n'tests/ct/.hid' -> 'tests/cv/.hid'\n'tests/ct/a' -> 'tests/cv/a'\n"
           "'tests/ct/sub' -> 'tests/cv/sub'\n'tests/ct/sub/b' -> 'tests/cv/sub/b'\n", 0)
    status("cp -r tests/ct tests/ct/inside", "cp: cannot copy a directory, 'tests/ct', into itself, 'tests/ct/inside'\n", 1)
    status("cp -r tests/ct tests/ct", "cp: cannot copy a directory, 'tests/ct', into itself, 'tests/ct/ct'\n", 1)  # the destination is a directory: the target is inside it
    status("cp -r tests/ct tests/hello.txt", "cp: cannot overwrite non-directory 'tests/hello.txt' with directory 'tests/ct'\n", 1)
    s.run("mkdir tests/cmulti")
    status("cp -r tests/ct tests/cv/a tests/cmulti", "", 0)  # a directory and a file, into a directory
    check("...both there", s.run("cat tests/cmulti/ct/a tests/cmulti/a"), "cat tests/cmulti/ct/a tests/cmulti/a\n1\n1\n")
    status("cp -r tests/nosuch tests/cz", "cp: cannot stat 'tests/nosuch': No such file or directory\n", 1)
    s.run("echo new > tests/cn1")
    s.run("echo old > tests/cn2")
    status("cp -n tests/cn1 tests/cn2", "", 0)  # exists: skipped, silently
    check("...untouched", s.run("cat tests/cn2"), "cat tests/cn2\nold\n")
    status("cp -nv tests/cn1 tests/cn2", "", 0)
    status("cp -nv tests/cn1 tests/cn3", "'tests/cn1' -> 'tests/cn3'\n", 0)  # missing: copied
    status("cp tests/cn1 tests/cn2", "", 0)  # without -n it overwrites
    check("...overwritten", s.run("cat tests/cn2"), "cat tests/cn2\nnew\n")
    status("cp -v tests/cn1 tests/cn4", "'tests/cn1' -> 'tests/cn4'\n", 0)
    status("cp -r tests/cn1 tests/cn5", "", 0)  # -r on a plain file is just a copy
    status("cp -rn tests/ct tests/cv", "", 0)  # merging into an existing tree, keeping what is there
    status("cp -x tests/cn1 tests/cn6", "cp: invalid option -- 'x'\n" + TRY("cp"), 1)
    check("cp --help", s.run("cp --help"),
          "cp --help\nusage: cp [-r] [-n] [-v] SRC... DST\n  -r  copy directories recursively\n"
          "  -n  do not overwrite an existing file\n  -v  print what is being copied\n")

    # ================================================================= mv -n -v -f
    s.run("echo 1 > tests/m1")
    s.run("echo 3 > tests/m3")
    s.run("echo 4 > tests/m4")
    s.run("echo 5 > tests/m5")
    status("mv -v tests/m1 tests/m2", "renamed 'tests/m1' -> 'tests/m2'\n", 0)
    status("mv -n tests/m3 tests/m4", "mv: not replacing 'tests/m4'\n", 1)
    check("...both files as they were", s.run("cat tests/m3 tests/m4"), "cat tests/m3 tests/m4\n3\n4\n")
    status("mv -nv tests/m3 tests/m7", "renamed 'tests/m3' -> 'tests/m7'\n", 0)  # nothing there: moved
    status("mv -f tests/m5 tests/m4", "", 0)  # accepted; replaces as a plain mv does
    check("...replaced", s.run("cat tests/m4"), "cat tests/m4\n5\n")
    s.run("mkdir tests/mvd")
    status("mv -v tests/m7 tests/mvd", "renamed 'tests/m7' -> 'tests/mvd/m7'\n", 0)
    status("mv -y tests/m2 tests/m9", "mv: invalid option -- 'y'\n" + TRY("mv"), 1)

    # ================================================================= rm -v -d
    s.run("echo x > tests/rf1")
    status("rm -v tests/rf1", "removed 'tests/rf1'\n", 0)
    s.run("mkdir -p tests/rt/sub")
    s.run("echo a > tests/rt/a")
    s.run("echo b > tests/rt/sub/b")
    s.run("echo z > tests/rt/z")
    status("rm -rv tests/rt",
           "removed 'tests/rt/a'\nremoved 'tests/rt/sub/b'\nremoved directory 'tests/rt/sub'\nremoved 'tests/rt/z'\n"
           "removed directory 'tests/rt'\n", 0)
    s.run("mkdir tests/rde tests/rde2")
    status("rm -d tests/rde", "", 0)
    status("rm -dv tests/rde2", "removed directory 'tests/rde2'\n", 0)
    s.run("mkdir tests/rdn")
    s.run("echo x > tests/rdn/f")
    status("rm -d tests/rdn", "rm: cannot remove 'tests/rdn': Directory not empty\n", 1)
    status("rm tests/rdn", "rm: cannot remove 'tests/rdn': Is a directory\n", 1)  # still refused without -d or -r
    s.run("echo x > tests/rf2")
    status("rm -d tests/rf2", "", 0)  # -d on a file is an ordinary remove
    status("rm -fv tests/nosuch", "", 0)
    status("rm -d /", "rm: cannot remove '/': Is a directory\n", 1)
    check("rm --help", s.run("rm --help"),
          "rm --help\nusage: rm [-r] [-f] [-v] [-d] PATH...\n  -r  remove directories and their contents recursively\n"
          "  -f  ignore nonexistent operands, never prompt\n  -v  print a message for each file removed\n  -d  remove empty directories\n")

    # ================================================================= ls
    s.run("mkdir -p tests/lsd/sub tests/lsd/.hd")
    s.run("echo x > tests/lsd/sub/x")
    s.run("echo y > tests/lsd/.hd/y")
    s.run("echo 1234 > tests/lsd/big")  # 5 bytes
    s.run("echo 12 > tests/lsd/mid")  # 3
    s.run("echo 1 > tests/lsd/small")  # 2
    s.run("echo h > tests/lsd/.hidden")
    s.run("cp /bin/hello tests/lsd/prog")
    s.run("chmod +x tests/lsd/prog")

    status("ls tests/lsd", "big\nmid\nprog\nsmall\nsub\n", 0)  # dot-names hidden
    status("ls -a tests/lsd", ".hd\n.hidden\nbig\nmid\nprog\nsmall\nsub\n", 0)
    status("ls -r tests/lsd", "sub\nsmall\nprog\nmid\nbig\n", 0)
    status("ls -ar tests/lsd", "sub\nsmall\nprog\nmid\nbig\n.hidden\n.hd\n", 0)
    status("ls -F tests/lsd", "big\nmid\nprog*\nsmall\nsub/\n", 0)
    hello_size = os.path.getsize(os.path.join(ctx.bin_dir, "hello"))
    status("ls -l tests/lsd",
           f"-w- {5:>10} big\n-w- {3:>10} mid\n-wx {hello_size:>10} prog\n-w- {2:>10} small\ndw- {0:>10} sub\n", 0)
    status("ls -lh tests/lsd", f"-w-      5 big\n-w-      3 mid\n-wx {human(hello_size):>6} prog\n-w-      2 small\ndw-      0 sub\n", 0)
    status("ls -S tests/lsd", "prog\nbig\nmid\nsmall\nsub\n", 0)  # largest first, ties by name (sub is 0 bytes)
    status("ls -Sr tests/lsd", "sub\nsmall\nmid\nbig\nprog\n", 0)
    big = os.path.getsize(os.path.join(ctx.tests_dir, "bigpad"))
    status("ls -lh tests/bigpad", f"-w- {human(big):>6} tests/bigpad\n", 0)  # a file operand, listed as itself

    # -d, and a file operand is listed as itself
    status("ls tests/hello.txt", "tests/hello.txt\n", 0)
    status("ls -l tests/hello.txt", f"-w- {len(ctx.fixture('hello.txt')):>10} tests/hello.txt\n", 0)
    status("ls -d tests/lsd", "tests/lsd\n", 0)
    status("ls -dl tests/lsd", f"dw- {0:>10} tests/lsd\n", 0)
    status("ls -d tests/lsd tests", "tests\ntests/lsd\n", 0)
    status("ls -dF tests/lsd tests/hello.txt", "tests/hello.txt\ntests/lsd/\n", 0)
    status("ls tests/hello.txt tests/lsd/sub", "tests/hello.txt\n\ntests/lsd/sub:\nx\n", 0)  # files first, then a header per directory
    status("ls tests/lsd/sub tests/lsd/.hd", "tests/lsd/.hd:\ny\n\ntests/lsd/sub:\nx\n", 0)  # directory operands are sorted, and always shown
    status("ls tests/lsd/.hidden", "tests/lsd/.hidden\n", 0)  # a dot-named operand is not hidden

    # -R
    status("ls -R tests/lsd", "tests/lsd:\nbig\nmid\nprog\nsmall\nsub\n\ntests/lsd/sub:\nx\n", 0)
    status("ls -aR tests/lsd",
           "tests/lsd:\n.hd\n.hidden\nbig\nmid\nprog\nsmall\nsub\n\ntests/lsd/.hd:\ny\n\ntests/lsd/sub:\nx\n", 0)
    s.run("cd tests/lsd")
    check("ls -R with no operand names the directory '.'", s.run("ls -R"), "ls -R\n.:\nbig\nmid\nprog\nsmall\nsub\n\n./sub:\nx\n")
    check("ls with no operand", s.run("ls"), "ls\nbig\nmid\nprog\nsmall\nsub\n")
    s.run("cd /")
    status("ls -R tests/nosuch", "ls: cannot access 'tests/nosuch': No such file or directory\n", 1)
    status("ls tests/nosuch tests/lsd/sub", "ls: cannot access 'tests/nosuch': No such file or directory\ntests/lsd/sub:\nx\n", 1)

    # -t: newest first (real waits between the files: FAT time has a 2 s tick)
    s.run("mkdir tests/lst")
    s.run("echo 1 > tests/lst/oldest")
    time.sleep(2.4)
    s.run("echo 1 > tests/lst/middle")
    time.sleep(2.4)
    s.run("echo 1 > tests/lst/newest")
    status("ls -t tests/lst", "newest\nmiddle\noldest\n", 0)
    status("ls -tr tests/lst", "oldest\nmiddle\nnewest\n", 0)
    status("ls tests/lst", "middle\nnewest\noldest\n", 0)  # by name otherwise
    status("ls -t tests/lst/oldest tests/lst/newest", "tests/lst/newest\ntests/lst/oldest\n", 0)  # file operands too

    status("ls -x", "ls: invalid option -- 'x'\n" + TRY("ls"), 1)
    check("ls --help", s.run("ls --help"),
          "ls --help\nusage: ls [-1] [-a] [-d] [-F] [-h] [-l] [-R] [-r] [-S] [-t] [FILE...]\n"
          "  -a  show names starting with '.'\n  -d  list directories themselves, not their contents\n"
          "  -F  append / to directories and * to executable files\n  -h  with -l, sizes like 1.5K\n"
          "  -l  long format: d/w/x flags, size, name\n  -R  list subdirectories recursively\n  -r  reverse the order\n"
          "  -S  sort by size, largest first\n  -t  sort by modify time, newest first\n")

    check("the shell is alive", s.run("echo ok"), "echo ok\nok\n")
