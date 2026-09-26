"""Last updated: Stage 18, Step 1.

The small programs Stage 18 adds: `rmdir` (empty directories only), `touch` (create a file, or make an existing file's
modify time now without touching its contents), `seq` (integer sequences) and `cmp` (byte-compare two files).

`touch` has no syscall to set a time, so it opens the file for append and closes it; that this really moves the modify
time of a file that already has data (and changes nothing else) is what the first checks below pin down -- the FAT
timestamp has 2-second resolution, so the test waits a few seconds of real time between the two `stat`s (the guest's
clock follows the host's). The time zone is UTC here so the two stamps compare as plain text.
"""

import os
import time

ENVIRONMENT = "HOME=/\nPATH=/bin\nTZ=UTC\n"


def run(ctx):
    s, check = ctx.s, ctx.check

    def fields(out):
        """The Size, Modify and Create lines of a `stat` transcript."""
        return {l.split(":")[0].strip(): l.split(":", 1)[1].strip() for l in out.split("\n") if l.startswith(("  Size", "Modify", "Create"))}

    def status(cmd, want_out, want_status):
        check(cmd, s.run_status(cmd), (f"{cmd}\n{want_out}", want_status))

    TRY = lambda prog: f"Try '{prog} --help' for more information.\n"

    # ================================================================= touch: the modify time
    s.run("echo hello > tests/tm1")
    before = fields(s.run("stat tests/tm1"))
    time.sleep(3.2)  # more than one FAT time tick (2 s), as the guest's clock follows real time
    check("touch an existing file", s.run("touch tests/tm1"), "touch tests/tm1\n")
    after = fields(s.run("stat tests/tm1"))
    check("touch: the modify time moved forward", after["Modify"] > before["Modify"], True)
    check("...the size is the same", after["Size"], before["Size"])
    check("...and so is the creation time", after["Create"], before["Create"])
    check("...and the data is untouched", s.run("cat tests/tm1"), "cat tests/tm1\nhello\n")

    # An empty file too, and an executable stays executable.
    s.run("echo > tests/tm2")
    s.run("chmod +x tests/tm2")
    s.run("touch tests/tm2")
    check("touch keeps the exec bit", s.run("ls -F tests").count("tm2*\n"), 1)

    # ================================================================= touch: the rest
    check("touch creates a missing file", s.run("touch tests/tc1"), "touch tests/tc1\n")
    check("...empty", fields(s.run("stat tests/tc1"))["Size"].split()[0], "0")
    check("touch several", s.run("touch tests/ta tests/tb"), "touch tests/ta tests/tb\n")
    check("...both exist", s.run("cat tests/ta tests/tb"), "cat tests/ta tests/tb\n")
    status("touch -c tests/tno", "", 0)
    status("stat tests/tno", "stat: cannot stat 'tests/tno': No such file or directory\n", 1)
    status("touch -c tests/ta tests/tno", "", 0)
    s.run("echo x > tests/tro")
    s.run("chmod -w tests/tro")
    status("touch tests/tro", "touch: cannot touch 'tests/tro': Permission denied\n", 1)
    s.run("chmod +w tests/tro")
    status("touch tests/nodir/x", "touch: cannot touch 'tests/nodir/x': No such file or directory\n", 1)
    out = s.run_status("touch tests/docs")
    check("touch a directory is refused", out[1], 1)
    check("...with the reason", out[0].startswith("touch tests/docs\ntouch: cannot touch 'tests/docs': "), True)
    status("touch tests/tro tests/nodir/x tests/tc2", "touch: cannot touch 'tests/nodir/x': No such file or directory\n", 1)
    check("...the operands around a failing one are still done", s.run_status("stat tests/tc2")[1], 0)
    status("touch", "touch: missing file operand\n" + TRY("touch"), 1)
    status("touch -x", "touch: invalid option -- 'x'\n" + TRY("touch"), 1)
    check("touch --help", s.run("touch --help"), "touch --help\nusage: touch [-c] FILE...\n  -c  do not create a file that does not exist\n")

    # ================================================================= rmdir
    s.run("mkdir tests/rd1 tests/rd3 tests/rd4")
    status("rmdir tests/rd1", "", 0)
    status("ls tests/rd1", "ls: cannot access 'tests/rd1': No such file or directory\n", 1)
    s.run("mkdir tests/rd2")
    s.run("echo x > tests/rd2/f")
    status("rmdir tests/rd2", "rmdir: failed to remove 'tests/rd2': Directory not empty\n", 1)
    check("...and it is still there", s.run("cat tests/rd2/f"), "cat tests/rd2/f\nx\n")
    status("rmdir tests/hello.txt", "rmdir: failed to remove 'tests/hello.txt': Not a directory\n", 1)
    status("rmdir tests/nosuch", "rmdir: failed to remove 'tests/nosuch': No such file or directory\n", 1)
    status("rmdir tests/rd3 tests/nosuch tests/rd4", "rmdir: failed to remove 'tests/nosuch': No such file or directory\n", 1)
    status("ls tests/rd3", "ls: cannot access 'tests/rd3': No such file or directory\n", 1)  # the others were removed
    status("ls tests/rd4", "ls: cannot access 'tests/rd4': No such file or directory\n", 1)
    s.run("mkdir tests/rd5")
    s.run("mkdir tests/rd5/inner")
    status("rmdir tests/rd5/inner tests/rd5", "", 0)  # the inner one first, so the outer is empty by then
    status("rmdir", "rmdir: missing operand\n" + TRY("rmdir"), 1)
    check("rmdir --help", s.run("rmdir --help"), "rmdir --help\nusage: rmdir DIR...\n")

    # ================================================================= seq
    def seq(args, out, st=0):
        status(f"seq {args}", out, st)

    seq("3", "1\n2\n3\n")
    seq("2 4", "2\n3\n4\n")
    seq("1 2 10", "1\n3\n5\n7\n9\n")
    seq("5 -1 1", "5\n4\n3\n2\n1\n")
    seq("3 1", "")  # nothing to print
    seq("0 0", "0\n")
    seq("-3 -1", "-3\n-2\n-1\n")
    seq("-s , 5", "1,2,3,4,5\n")
    seq("-s ', ' 3", "1, 2, 3\n")
    seq("-s: 2 4", "2:3:4\n")
    seq("-w 8 11", "08\n09\n10\n11\n")
    seq("-w -2 2", "-2\n-1\n00\n01\n02\n")
    seq("9223372036854775806 9223372036854775807", "9223372036854775806\n9223372036854775807\n")  # stops at the end of i64
    seq("", "seq: missing operand\n" + TRY("seq"), 1)
    seq("a", "seq: invalid integer argument: 'a'\n" + TRY("seq"), 1)
    seq("1.5", "seq: invalid integer argument: '1.5'\n" + TRY("seq"), 1)
    seq("1 0 5", "seq: invalid Zero increment value: '0'\n" + TRY("seq"), 1)
    seq("1 2 3 4", "seq: extra operand '4'\n" + TRY("seq"), 1)
    seq("-x 3", "seq: invalid option -- 'x'\n" + TRY("seq"), 1)
    seq("-s", "seq: option requires an argument -- 's'\n" + TRY("seq"), 1)
    check("seq --help", s.run("seq --help"),
          "seq --help\nusage: seq [-s SEP] [-w] [FIRST [INCREMENT]] LAST\n  -s SEP  separate the numbers with SEP instead of a newline\n"
          "  -w  pad with leading zeros to the width of the widest of FIRST and LAST\n")
    check("a long run, through a pipe", s.run("seq 1 20000 | wc -l"), "seq 1 20000 | wc -l\n20000\n")
    check("...and its last line", s.run("seq 1 100000 | tail -n 1"), "seq 1 100000 | tail -n 1\n100000\n")

    # ================================================================= cmp
    s.run("cp tests/hello.txt tests/cm1")
    status("cmp tests/hello.txt tests/cm1", "", 0)
    s.run("echo abc > tests/ca")
    s.run("echo abd > tests/cb")
    status("cmp tests/ca tests/cb", "tests/ca tests/cb differ: byte 3, line 1\n", 1)
    status("cmp -s tests/ca tests/cb", "", 1)
    status("cmp -s tests/ca tests/ca", "", 0)
    s.run("echo one > tests/cl1")
    s.run("echo two >> tests/cl1")
    s.run("echo one > tests/cl2")
    s.run("echo twx >> tests/cl2")
    status("cmp tests/cl1 tests/cl2", "tests/cl1 tests/cl2 differ: byte 7, line 2\n", 1)
    s.run("echo abc > tests/ce1")
    s.run("echo abc > tests/ce2")
    s.run("echo def >> tests/ce2")
    status("cmp tests/ce1 tests/ce2", "cmp: EOF on tests/ce1 after byte 4\n", 1)
    status("cmp tests/ce2 tests/ce1", "cmp: EOF on tests/ce1 after byte 4\n", 1)
    status("cmp -s tests/ce1 tests/ce2", "", 1)
    check("cmp with standard input", s.run_status("cmp - tests/hello.txt < tests/hello.txt"),
          ("cmp - tests/hello.txt < tests/hello.txt\n", 0))
    check("...and one that differs", s.run_status("cmp tests/hello.txt - < tests/ca"),
          ("cmp tests/hello.txt - < tests/ca\ntests/hello.txt - differ: byte 1, line 1\n", 1))
    # A 3 MiB program, many read buffers long.
    big = os.path.getsize(os.path.join(ctx.tests_dir, "bigpad"))
    s.run("cp tests/bigpad tests/bigcmp")
    status("cmp tests/bigpad tests/bigcmp", "", 0)
    s.run("echo x >> tests/bigcmp")
    status("cmp tests/bigpad tests/bigcmp", f"cmp: EOF on tests/bigpad after byte {big}\n", 1)
    status("cmp tests/nosuch tests/hello.txt", "cmp: tests/nosuch: No such file or directory\n", 2)
    status("cmp tests/hello.txt tests/nosuch", "cmp: tests/nosuch: No such file or directory\n", 2)
    status("cmp tests/docs tests/hello.txt", "cmp: tests/docs: Is a directory\n", 2)
    status("cmp", "cmp: missing operand\n" + TRY("cmp"), 2)
    status("cmp tests/ca", "cmp: missing operand after 'tests/ca'\n" + TRY("cmp"), 2)
    status("cmp tests/ca tests/cb tests/cl1", "cmp: extra operand 'tests/cl1'\n" + TRY("cmp"), 2)
    status("cmp -x tests/ca tests/cb", "cmp: invalid option -- 'x'\n" + TRY("cmp"), 2)
    check("cmp --help", s.run("cmp --help"), "cmp --help\nusage: cmp [-s] FILE1 FILE2\n  -s  print nothing; the exit status says whether they differ\n")

    check("the shell is alive", s.run("echo ok"), "echo ok\nok\n")
