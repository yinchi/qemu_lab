"""Last updated: Stage 18, Step 3.

The filters Stage 18 adds -- `sort`, `uniq`, `cut`, `tr`, `find` and `fgrep` -- as the shell reaches them, mostly through
pipes (which go through temp files) and through small files written with `echo`. What each one accepts is spelled out by
the helpers' host tests (`glob.rs`, `cutlist.rs`, `trset.rs`, `sortkey.rs`, `textutil.rs`); here it is whole programs:
arguments, standard input and files, several files, output, status and every message.

`seq` (Step 1) is the source of large inputs.
"""

import os

ENVIRONMENT = "HOME=/\nPATH=/bin\n"


def run(ctx):
    s, check = ctx.s, ctx.check

    def status(cmd, out, st=0):
        check(cmd, s.run_status(cmd), (f"{cmd}\n{out}", st))

    TRY = lambda prog: f"Try '{prog} --help' for more information.\n"

    def write(path, *lines, last_newline=True):
        """Makes `path` hold `lines` (one `echo` each); `last_newline=False` leaves the final newline off."""
        for i, line in enumerate(lines):
            redirect = ">" if i == 0 else ">>"
            flag = "-n " if (i == len(lines) - 1 and not last_newline) else ""
            s.run(f"echo {flag}{line} {redirect} {path}")

    # ================================================================= sort
    write("tests/s1", "banana", "apple", "cherry", "apple")
    status("sort tests/s1", "apple\napple\nbanana\ncherry\n")
    status("sort -r tests/s1", "cherry\nbanana\napple\napple\n")
    status("sort -u tests/s1", "apple\nbanana\ncherry\n")
    status("sort -ru tests/s1", "cherry\nbanana\napple\n")
    write("tests/n1", "10", "9", "2", "33", "-5", "0", "abc", "3.5")
    status("sort tests/n1", "-5\n0\n10\n2\n3.5\n33\n9\nabc\n")  # as text
    status("sort -n tests/n1", "-5\n0\nabc\n2\n3.5\n9\n10\n33\n")  # a line with no number is 0; ties in byte order
    status("sort -nr tests/n1", "33\n10\n9\n3.5\n2\nabc\n0\n-5\n")
    status("sort -nu tests/n1", "-5\n0\n2\n3.5\n9\n10\n33\n")  # 0 and abc are the same key: the first stays
    write("tests/s2", "zebra", "apple")
    status("sort tests/s1 tests/s2", "apple\napple\napple\nbanana\ncherry\nzebra\n")  # all files together
    write("tests/s3", "b", "a", last_newline=False)
    status("sort tests/s3", "a\nb\n")  # a last line with no newline is still a line
    s.run("echo -n > tests/empty")
    status("sort tests/empty", "")
    check("sort from stdin", s.run("cat tests/s1 | sort"), "cat tests/s1 | sort\napple\napple\nbanana\ncherry\n")
    check("...and with -", s.run("sort - < tests/s1"), "sort - < tests/s1\napple\napple\nbanana\ncherry\n")
    check("...and with a file and -", s.run("cat tests/s2 | sort tests/s1 -"),
          "cat tests/s2 | sort tests/s1 -\napple\napple\napple\nbanana\ncherry\nzebra\n")
    status("sort tests/nosuch", "sort: cannot read: tests/nosuch: No such file or directory\n", 2)
    status("sort -x tests/s1", "sort: invalid option -- 'x'\n" + TRY("sort"), 1)
    check("sort --help", s.run("sort --help"),
          "sort --help\nusage: sort [-r] [-n] [-u] [file...]\n  -n  compare by the number at the start of the line\n"
          "  -r  reverse the order\n  -u  print only the first of lines that compare equal\n")
    check("a long input: 3000 numbers as text", s.run("seq 1 3000 | sort | head -n 4"),
          "seq 1 3000 | sort | head -n 4\n1\n10\n100\n1000\n")
    check("...and by number", s.run("seq 1 20000 | sort -nr | head -n 2"), "seq 1 20000 | sort -nr | head -n 2\n20000\n19999\n")
    check("...counted", s.run("seq 1 20000 | sort -u | wc -l"), "seq 1 20000 | sort -u | wc -l\n20000\n")

    # ================================================================= uniq
    write("tests/u1", "a", "a", "b", "a", "c", "c", "c")
    status("uniq tests/u1", "a\nb\na\nc\n")  # only adjacent lines
    status("uniq -c tests/u1", f"{2:>7} a\n{1:>7} b\n{1:>7} a\n{3:>7} c\n")
    status("uniq -d tests/u1", "a\nc\n")
    status("uniq -u tests/u1", "b\na\n")
    status("uniq -cd tests/u1", f"{2:>7} a\n{3:>7} c\n")
    check("sort | uniq -c", s.run("sort tests/u1 | uniq -c"), f"sort tests/u1 | uniq -c\n{3:>7} a\n{1:>7} b\n{3:>7} c\n")
    check("uniq from stdin", s.run("cat tests/u1 | uniq"), "cat tests/u1 | uniq\na\nb\na\nc\n")
    status("uniq tests/empty", "")
    status("uniq tests/nosuch", "uniq: tests/nosuch: No such file or directory\n", 1)
    status("uniq tests/u1 tests/s1", "uniq: extra operand 'tests/s1'\n" + TRY("uniq"), 1)
    check("uniq --help", s.run("uniq --help"),
          "uniq --help\nusage: uniq [-c] [-d] [-u] [file]\n  -c  prefix each line with the number of times it occurs\n"
          "  -d  print only lines that are repeated\n  -u  print only lines that are not repeated\n")

    # ================================================================= cut
    write("tests/c1", "a:b:c:d", "one:two", "nodelim", ":x")
    status("cut -d : -f 2 tests/c1", "b\ntwo\nnodelim\nx\n")  # a line with no delimiter comes out whole
    status("cut -d : -f 2 -s tests/c1", "b\ntwo\nx\n")
    status("cut -d: -f1,3 tests/c1", "a:c\none\nnodelim\n\n")
    status("cut -d : -f 2- tests/c1", "b:c:d\ntwo\nnodelim\nx\n")
    status("cut -d : -f -2 tests/c1", "a:b\none:two\nnodelim\n:x\n")
    status("cut -f 1 -d : tests/c1", "a\none\nnodelim\n\n")  # options in either order
    check("the default delimiter is a tab", s.run("echo a b c | tr ' ' '\\t' | cut -f 2"),
          "echo a b c | tr ' ' '\\t' | cut -f 2\nb\n")
    # Characters, not bytes: a fixture holding three CJK characters (nine bytes).
    status("cut -c 1 tests/cjk.txt", "日\n")
    status("cut -c 2-3 tests/cjk.txt", "本語\n")
    status("cut -c 2- tests/cjk.txt", "本語\n")
    status("cut -c 1,3 tests/cjk.txt", "日語\n")
    status("cut -c 4- tests/cjk.txt", "\n")
    write("tests/c2", "abcdef", "xy")
    status("cut -c 1-3 tests/c2", "abc\nxy\n")
    status("cut -c 2,4 tests/c2", "bd\ny\n")
    status("cut -c 5- tests/c2", "ef\n\n")
    check("cut from stdin", s.run("cat tests/c1 | cut -d : -f 1"), "cat tests/c1 | cut -d : -f 1\na\none\nnodelim\n\n")
    status("cut -d : -f 1 tests/c1 tests/c1", "a\none\nnodelim\n\na\none\nnodelim\n\n")  # several files
    status("cut -f 1 tests/nosuch", "cut: tests/nosuch: No such file or directory\n", 1)
    status("cut tests/c1", "cut: you must specify a list of characters or fields\n" + TRY("cut"), 1)
    status("cut -f 1 -c 1 tests/c1", "cut: only one type of list may be specified\n" + TRY("cut"), 1)
    status("cut -f 0 tests/c1", "cut: fields are numbered from 1\n" + TRY("cut"), 1)
    status("cut -c 0 tests/c1", "cut: character positions are numbered from 1\n" + TRY("cut"), 1)
    status("cut -f a tests/c1", "cut: invalid field value 'a'\n" + TRY("cut"), 1)
    status("cut -f 3-1 tests/c1", "cut: invalid decreasing range\n" + TRY("cut"), 1)
    status("cut -d ab -f 1 tests/c1", "cut: the delimiter must be a single character\n" + TRY("cut"), 1)
    status("cut -s -c 1 tests/c1", "cut: suppressing non-delimited lines makes sense only when operating on fields\n" + TRY("cut"), 1)
    check("cut --help", s.run("cut --help"),
          "cut --help\nusage: cut (-c LIST | -f LIST) [-d C] [-s] [file...]\n  -c LIST  select these character positions\n"
          "  -f LIST  select these fields\n  -d C  the field delimiter, one character (default: tab)\n"
          "  -s  with -f, skip lines that have no delimiter\n")

    # ================================================================= tr
    def tr(cmd, out, st=0):
        check(cmd, s.run_status(cmd), (f"{cmd}\n{out}", st))

    tr("echo hello | tr a-z A-Z", "HELLO\n")
    tr("echo hello | tr el ip", "hippo\n")
    tr("echo hello | tr a-y b-z", "ifmmp\n")
    tr("echo abcd | tr a-d xy", "xyyy\n")  # a short second set is padded with its last character
    tr("echo hello world | tr -d lo", "he wrd\n")
    tr("echo aabbcc | tr -s abc", "abc\n")
    tr("echo 'a  b   c' | tr -s ' '", "a b c\n")
    tr("echo aabbcc | tr -s a-c x", "x\n")  # translated, then squeezed
    tr("echo 'Hello 123' | tr '[:upper:]' '[:lower:]'", "hello 123\n")
    tr("echo a1b22c | tr -d '[:digit:]'", "abc\n")
    tr("echo a1b22c | tr -ds '[:digit:]' '[:alpha:]'", "abc\n")
    tr("echo abc | tr b '\\n'", "a\nc\n")  # an escape in a set
    tr("echo abc | tr '\\141' X", "Xbc\n")  # octal
    tr("echo a-b | tr '\\-' _", "a_b\n")
    cjk = ctx.fixture("cjk.txt")
    check("tr passes other bytes through: UTF-8 text stays intact", s.run("cat tests/cjk.txt | tr a-z A-Z"),
          "cat tests/cjk.txt | tr a-z A-Z\n" + "".join(c.upper() if c.isascii() else c for c in cjk))
    tr("tr", "tr: missing operand\n" + TRY("tr"), 1)
    tr("tr a", "tr: missing operand after 'a'\n" + TRY("tr"), 1)
    tr("tr -d a b", "tr: extra operand 'b'\n" + TRY("tr"), 1)
    tr("tr a b c", "tr: extra operand 'c'\n" + TRY("tr"), 1)
    tr("tr z-a x", "tr: range endpoints in 'z-a' are in reverse order\n", 1)
    tr("tr '[:nosuch:]' x", "tr: invalid character class in '[:nosuch:]'\n", 1)
    tr("tr a ''", "tr: when not truncating set1, string2 must be non-empty\n", 1)
    check("tr --help", s.run("tr --help"),
          "tr --help\nusage: tr [-d] [-s] SET1 [SET2]\n  -d  delete the characters in SET1\n"
          "  -s  replace each run of a repeated character (from the last set) with one\n")

    # ================================================================= find
    s.run("mkdir -p tests/ft/sub/deep tests/ft/.hid tests/ft/empty")
    for f in ["a.txt", "b.log", "sub/c.txt", "sub/deep/d.txt", ".hid/e.txt"]:
        s.run(f"echo x > tests/ft/{f}")
    everything = ("tests/ft\ntests/ft/.hid\ntests/ft/.hid/e.txt\ntests/ft/a.txt\ntests/ft/b.log\ntests/ft/empty\n"
                  "tests/ft/sub\ntests/ft/sub/c.txt\ntests/ft/sub/deep\ntests/ft/sub/deep/d.txt\n")
    status("find tests/ft", everything)  # depth first, name order, dot-names included
    status("find tests/ft -name '*.txt'",
           "tests/ft/.hid/e.txt\ntests/ft/a.txt\ntests/ft/sub/c.txt\ntests/ft/sub/deep/d.txt\n")
    status("find tests/ft -name b.log", "tests/ft/b.log\n")
    status("find tests/ft -name '[a-c].*'", "tests/ft/a.txt\ntests/ft/b.log\ntests/ft/sub/c.txt\n")
    status("find tests/ft -name 'd*' -type d", "tests/ft/sub/deep\n")
    status("find tests/ft -type d", "tests/ft\ntests/ft/.hid\ntests/ft/empty\ntests/ft/sub\ntests/ft/sub/deep\n")
    status("find tests/ft -type f",
           "tests/ft/.hid/e.txt\ntests/ft/a.txt\ntests/ft/b.log\ntests/ft/sub/c.txt\ntests/ft/sub/deep/d.txt\n")
    status("find tests/ft -maxdepth 0", "tests/ft\n")
    status("find tests/ft -maxdepth 1", "tests/ft\ntests/ft/.hid\ntests/ft/a.txt\ntests/ft/b.log\ntests/ft/empty\ntests/ft/sub\n")
    status("find tests/ft -mindepth 1 -maxdepth 1 -type d", "tests/ft/.hid\ntests/ft/empty\ntests/ft/sub\n")
    status("find tests/ft -mindepth 3", "tests/ft/sub/deep/d.txt\n")
    status("find tests/ft/a.txt", "tests/ft/a.txt\n")  # a file operand is itself
    status("find tests/ft/a.txt -type d", "")
    status("find tests/ft/empty tests/ft/sub -maxdepth 0", "tests/ft/empty\ntests/ft/sub\n")  # several start paths
    status("find tests/nosuch", "find: 'tests/nosuch': No such file or directory\n", 1)
    status("find tests/nosuch tests/ft/empty", "find: 'tests/nosuch': No such file or directory\ntests/ft/empty\n", 1)
    s.run("cd tests/ft")
    check("find with no path starts at '.'", s.run("find -maxdepth 1 -name '*.log'"), "find -maxdepth 1 -name '*.log'\n./b.log\n")
    check("...and names things under it as ./name", s.run("find . -maxdepth 1 -type d"),
          "find . -maxdepth 1 -type d\n.\n./.hid\n./empty\n./sub\n")
    s.run("cd /")
    status("find tests/ft -name", "find: missing argument to '-name'\n", 1)
    status("find tests/ft -type x", "find: Unknown argument to -type: x\n", 1)
    status("find tests/ft -maxdepth x", "find: invalid argument 'x' to '-maxdepth'\n", 1)
    status("find tests/ft -foo", "find: unknown predicate '-foo'\n", 1)
    status("find tests/ft -name a.txt -print", "tests/ft/a.txt\n")
    check("find --help", s.run("find --help"),
          "find --help\nusage: find [PATH...] [-name GLOB] [-type f|d] [-mindepth N] [-maxdepth N]\n"
          "  -name GLOB  the last component matches GLOB (* ? [a-z])\n  -type f|d  a file, or a directory\n"
          "  -mindepth N  at least N levels below the start path\n  -maxdepth N  at most N levels below the start path\n")
    check("find in a pipeline", s.run("find tests/ft -type f | fgrep -c .txt"), "find tests/ft -type f | fgrep -c .txt\n4\n")

    # ================================================================= fgrep
    write("tests/g1", "Apple pie", "banana split", "cherry.pie", "APPLE juice", "a*b")
    write("tests/g2", "rhubarb pie", "nothing")
    write("tests/g3", "none here")
    status("fgrep pie tests/g1", "Apple pie\ncherry.pie\n")
    status("fgrep -i apple tests/g1", "Apple pie\nAPPLE juice\n")
    status("fgrep -v pie tests/g1", "banana split\nAPPLE juice\na*b\n")
    status("fgrep -n pie tests/g1", "1:Apple pie\n3:cherry.pie\n")
    status("fgrep -c pie tests/g1", "2\n")
    status("fgrep -ci apple tests/g1", "2\n")
    status("fgrep -cv pie tests/g1", "3\n")
    status("fgrep -e pie -e juice tests/g1", "Apple pie\ncherry.pie\nAPPLE juice\n")  # any of the patterns
    status("fgrep -e split tests/g1", "banana split\n")
    status("fgrep 'a*b' tests/g1", "a*b\n")  # fixed strings: * is a star
    status("fgrep . tests/g1", "cherry.pie\n")
    status("fgrep '' tests/g1", "Apple pie\nbanana split\ncherry.pie\nAPPLE juice\na*b\n")  # the empty pattern is everywhere
    status("fgrep zzz tests/g1", "", 1)
    status("fgrep -c zzz tests/g1", "0\n", 1)
    status("fgrep -q pie tests/g1", "", 0)
    status("fgrep -q zzz tests/g1", "", 1)
    status("fgrep pie tests/g1 tests/g2", "tests/g1:Apple pie\ntests/g1:cherry.pie\ntests/g2:rhubarb pie\n")
    status("fgrep -n pie tests/g1 tests/g2", "tests/g1:1:Apple pie\ntests/g1:3:cherry.pie\ntests/g2:1:rhubarb pie\n")
    status("fgrep -c pie tests/g1 tests/g2 tests/g3", "tests/g1:2\ntests/g2:1\ntests/g3:0\n")
    status("fgrep -l pie tests/g1 tests/g2 tests/g3", "tests/g1\ntests/g2\n")
    status("fgrep -l zzz tests/g1", "", 1)
    status("fgrep pie tests/g1 tests/nosuch", "tests/g1:Apple pie\ntests/g1:cherry.pie\nfgrep: tests/nosuch: No such file or directory\n", 2)
    check("fgrep from stdin", s.run("cat tests/g1 | fgrep -n split"), "cat tests/g1 | fgrep -n split\n2:banana split\n")
    check("...and with -", s.run("fgrep split - < tests/g1"), "fgrep split - < tests/g1\nbanana split\n")
    status("fgrep", "fgrep: missing operand\n" + TRY("fgrep"), 2)
    status("fgrep -x pie tests/g1", "fgrep: invalid option -- 'x'\n" + TRY("fgrep"), 1)
    check("fgrep --help", s.run("fgrep --help"),
          "fgrep --help\nusage: fgrep [-i] [-v] [-n] [-c] [-l] [-q] [-e PATTERN]... [PATTERN] [file...]\n"
          "  -e PATTERN  a pattern to look for (may be repeated; without -e the first operand is the pattern)\n"
          "  -i  ignore case (ASCII letters)\n  -v  select the lines that do not match\n"
          "  -n  prefix each line with its line number\n  -c  print only a count of the selected lines per file\n"
          "  -l  print only the names of files with a selected line\n"
          "  -q  print nothing; the status says whether any line was selected\n")

    # ================================================================= together
    check("digits: seq | fgrep | wc", s.run("seq 1 100 | fgrep 7 | wc -l"), "seq 1 100 | fgrep 7 | wc -l\n19\n")
    check("top three: seq | sort -nr | head", s.run("seq 1 10 | sort -nr | head -n 3"), "seq 1 10 | sort -nr | head -n 3\n10\n9\n8\n")
    check("tr then cut: a line with its newline removed still comes out as a line", s.run("echo hello | tr -d '\\n' | cut -c 1-5"),
          "echo hello | tr -d '\\n' | cut -c 1-5\nhello\n")
    check("the shell is alive", s.run("echo ok"), "echo ok\nok\n")
