"""Last updated: Stage 19, Step 4d.

The options Stage 19's `mv` adds to `-f`, `-n` and `-v`: `-i` (asks before replacing: the question goes to standard error and
the answer is read from standard input, `y` to go on; declining is status 1 and moves nothing), `-u` (replaces only a target that is
older than the source, silently skipping the rest), `-b` (a replaced file is kept as `NAME~`), `-t DIR` (every operand is a
source, `DIR` the directory they go into) and `-T` (the destination is a file, never a directory to move into), with the long
spellings, the last of `-f`/`-i`/`-n` winning, and GNU's messages for what cannot be done. Outputs were taken from the host's
coreutils. (Moving between two volumes is `mv_volumes`'.)
"""

ENVIRONMENT = "HOME=/\nTZ=UTC\n"


def run(ctx):
    s, check = ctx.s, ctx.check

    def status(cmd, want_out, want_status):
        check(cmd, s.run_status(cmd), (f"{cmd}\n{want_out}", want_status))

    def ask(cmd, answer):
        """Runs `cmd`, which is expected to ask a question, answers it, and returns everything shown up to the next prompt."""
        s.type(cmd + "\n")
        question = s.wait_until(lambda t: t.endswith("? "), "the question")
        s.pos = len(s.log())
        s.type(answer + "\n")
        rest = s.wait_until(lambda t: t.endswith("> "), "the prompt")
        s.pos = len(s.log())
        return question + rest[: -len("> ")]

    s.run("mkdir /w")
    s.run("cd /w")

    # ================================================================= -i
    s.run("echo new > a")
    s.run("echo old > b")
    check("-i asks, and n declines", ask("mv -i a b", "n"), "mv -i a b\nmv: overwrite 'b'? n\n")
    check("...status 1, nothing moved", (s.run("echo $?"), s.run("cat a"), s.run("cat b")), ("echo $?\n1\n", "cat a\nnew\n", "cat b\nold\n"))
    check("-i asks, and y replaces", ask("mv -i a b", "y"), "mv -i a b\nmv: overwrite 'b'? y\n")
    check("...moved", (s.run("echo $?"), s.run_status("cat a")[1], s.run("cat b")), ("echo $?\n0\n", 1, "cat b\nnew\n"))
    s.run("echo x > a")
    check("an answer is judged by its first letter: Yes", ask("mv -i a b", "Yes"), "mv -i a b\nmv: overwrite 'b'? Yes\n")
    check("...replaced", s.run("cat b"), "cat b\nx\n")
    s.run("echo y > a")
    check("anything else declines", ask("mv --interactive a b", "maybe"), "mv --interactive a b\nmv: overwrite 'b'? maybe\n")
    check("...b is as it was", s.run("cat b"), "cat b\nx\n")
    status("mv -i a nothere", "", 0)  # no question when there is nothing to replace
    check("...and it moved", s.run("cat nothere"), "cat nothere\ny\n")
    s.run("echo 1 > a")
    s.run("echo 2 > b")
    status("mv -i -f a b", "", 0)  # the last of -f, -i and -n wins: no question
    check("...-f last: replaced", s.run("cat b"), "cat b\n1\n")
    s.run("echo 3 > a")
    status("mv -f -n a b", "mv: not replacing 'b'\n", 1)
    check("-n last: not replaced", s.run("cat b"), "cat b\n1\n")
    check("-n then -i: asks", ask("mv -n -i a b", "n"), "mv -n -i a b\nmv: overwrite 'b'? n\n")
    status("mv -i -n a b", "mv: not replacing 'b'\n", 1)
    check("-i -v: says so after a yes", ask("mv -iv a b", "y"), "mv -iv a b\nmv: overwrite 'b'? y\nrenamed 'a' -> 'b'\n")

    # ================================================================= -u
    s.run("echo old > o")
    s.run("touch -d 2020-01-01 o")
    s.run("echo new > n")
    status("mv -u n o", "", 0)
    check("-u: a newer source replaces", (s.run("cat o"), s.run_status("cat n")[1]), ("cat o\nnew\n", 1))
    s.run("touch -d 2030-01-01 o")
    s.run("echo stale > n")
    status("mv -u n o", "", 0)
    check("-u: an older source is skipped, silently, and stays", (s.run("cat o"), s.run("cat n")), ("cat o\nnew\n", "cat n\nstale\n"))
    status("mv -uv n o", "", 0)  # even with -v
    s.run("touch -d 2030-01-01 n")
    status("mv -u n o", "", 0)  # the same time is not newer
    check("...nor is an equal one", s.run("cat o"), "cat o\nnew\n")
    status("mv -u n fresh", "", 0)
    check("-u onto a missing target moves", s.run("cat fresh"), "cat fresh\nstale\n")
    s.run("echo again > n")
    s.run("touch -d 2040-01-01 n")
    status("mv --update n o", "", 0)
    check("--update", s.run("cat o"), "cat o\nagain\n")

    # ================================================================= -b
    s.run("echo 1 > a")
    s.run("echo 2 > b")
    status("mv -b a b", "", 0)
    check("-b keeps the replaced file as NAME~", (s.run("cat b"), s.run("cat b~")), ("cat b\n1\n", "cat b~\n2\n"))
    s.run("echo 3 > a")
    status("mv -b a b", "", 0)
    check("...a second one replaces the earlier backup", (s.run("cat b"), s.run("cat b~")), ("cat b\n3\n", "cat b~\n1\n"))
    s.run("echo 4 > a")
    check("-bv says where the old one went", s.run("mv -bv a b"), "mv -bv a b\nrenamed 'a' -> 'b' (backup: 'b~')\n")
    s.run("echo 5 > a")
    status("mv -b a fresh2", "", 0)
    check("-b with nothing to replace: no backup", s.run_status("cat fresh2~")[1], 1)
    s.run("echo 6 > a")
    status("mv --backup a b", "", 0)
    check("--backup", s.run("cat b~"), "cat b~\n4\n")

    # ================================================================= -t and -T
    s.run("mkdir d")
    s.run("echo 1 > t1")
    s.run("echo 2 > t2")
    status("mv -t d t1 t2", "", 0)
    check("-t moves every operand into the directory", s.run("ls d"), "ls d\nt1\nt2\n")
    s.run("echo 3 > t3")
    status("mv -td t3", "", 0)  # the value may be attached
    s.run("echo 4 > t4")
    status("mv --target-directory=d t4", "", 0)
    s.run("echo 5 > t5")
    status("mv --target-directory d t5", "", 0)
    check("...whatever the spelling", s.run("ls d"), "ls d\nt1\nt2\nt3\nt4\nt5\n")
    s.run("echo 6 > t6")
    status("mv t6 -t d", "", 0)
    check("-t may come after the operands", s.run("cat d/t6"), "cat d/t6\n6\n")
    status("mv -t nodir x", "mv: target directory 'nodir': No such file or directory\n", 1)
    status("mv -t b x", "mv: target directory 'b': Not a directory\n", 1)
    status("mv -t d", "mv: missing file operand\nTry 'mv --help' for more information.\n", 1)
    status("mv -t", "mv: option requires an argument -- 't'\nTry 'mv --help' for more information.\n", 1)
    status("mv -t d -T x y", "mv: cannot combine --target-directory (-t) and --no-target-directory (-T)\n", 1)
    s.run("echo tt > d/dup")
    s.run("echo new > dup")
    status("mv -t d dup", "", 0)
    check("-t replaces a file in the directory like any move", s.run("cat d/dup"), "cat d/dup\nnew\n")

    s.run("mkdir e")
    s.run("echo f > ff")
    status("mv ff e", "", 0)  # without -T, a directory destination is moved into
    check("without -T: into the directory", s.run("ls e"), "ls e\nff\n")
    s.run("echo g > gg")
    status("mv -T gg e", "mv: cannot move 'gg' to 'e': File exists\n", 1)
    check("-T: the directory is not moved into", (s.run("ls e"), s.run("cat gg")), ("ls e\nff\n", "cat gg\ng\n"))
    status("mv -T gg hh", "", 0)
    check("-T renames to a file name", s.run("cat hh"), "cat hh\ng\n")
    status("mv --no-target-directory hh e/../ii", "", 0)
    check("--no-target-directory", s.run("cat ii"), "cat ii\ng\n")
    status("mv -T a b c", "mv: extra operand 'b'\nTry 'mv --help' for more information.\n", 1)
    status("mv -T a", "mv: missing destination file operand after 'a'\nTry 'mv --help' for more information.\n", 1)

    # ================================================================= across volumes too is mv_volumes'; help
    check("mv --help", s.run("mv --help"),
          "mv --help\nusage: mv [-f | -i | -n] [-u] [-b] [-v] [-T | -t DIR] SRC... DST\n"
          "  -f  replace without asking (the default; the last of -f, -i and -n wins)\n"
          "  -i  ask before replacing an existing target (y to go on)\n"
          "  -n  do not replace an existing target\n"
          "  -u  replace only a target that is older than the source\n"
          "  -b  keep a replaced file as NAME~\n"
          "  -t DIR  move every SRC into the directory DIR\n"
          "  -T  treat DST as a file, never as a directory to move into\n"
          "  -v  print what is being moved\n")
    check("the shell is alive", s.run("echo ok"), "echo ok\nok\n")
