"""Last updated: Stage 18, Step 4.

The flags Stage 18's tier adds to the text tools: `cat -n -E -T -s`, `head`/`tail` with several files (`==> name <==` headers,
`-q`, `-v`), `head -n -N` (all but the last N) and `tail -n +N` (from line N on), `wc -m` (characters) and `echo -e -E` (backslash
escapes). What each pure part accepts is in the host tests (`countspec.rs`); here it is the programs, end to end.
"""

ENVIRONMENT = "HOME=/\nPATH=/bin\n"


def run(ctx):
    s, check = ctx.s, ctx.check

    def status(cmd, out, st=0):
        check(cmd, s.run_status(cmd), (f"{cmd}\n{out}", st))

    TRY = lambda prog: f"Try '{prog} --help' for more information.\n"

    # ================================================================= cat -n -E -T -s
    for i, line in enumerate(["a", "", "", "b", "", "c"]):
        s.run(f"echo {line} {'>' if i == 0 else '>>'} tests/t1")
    s.run("echo x y | tr ' ' '\\t' > tests/tab1")  # one line with a tab in it
    s.run("echo 1 > tests/t2")
    s.run("echo 2 >> tests/t2")
    s.run("echo -n end > tests/t3")  # no final newline

    def numbered(lines, start=1):
        return "".join(f"{i:>6}\t{l}\n" for i, l in enumerate(lines, start))

    status("cat -n tests/t1", numbered(["a", "", "", "b", "", "c"]))  # every line, blank ones too
    status("cat -s tests/t1", "a\n\nb\n\nc\n")  # a run of blank lines becomes one
    status("cat -ns tests/t1", numbered(["a", "", "b", "", "c"]))  # numbered after squeezing
    status("cat -E tests/t1", "a$\n$\n$\nb$\n$\nc$\n")
    status("cat -T tests/tab1", "x^Iy\n")
    status("cat -nET tests/tab1", f"{1:>6}\tx^Iy$\n")
    status("cat -n tests/t2 tests/t2", numbered(["1", "2", "1", "2"]))  # the count goes on across files
    status("cat -s tests/t1 tests/t1", "a\n\nb\n\nc\n" + "a\n\nb\n\nc\n")
    status("cat -n tests/t3", f"{1:>6}\tend\n")  # (the shell finishes an unterminated last line)
    check("cat -E on a last line with no newline adds no $", s.run("cat -E tests/t3 | wc -c"), "cat -E tests/t3 | wc -c\n3\n")
    check("cat -n from stdin", s.run("cat tests/t2 | cat -n"), "cat tests/t2 | cat -n\n" + numbered(["1", "2"]))
    check("...and by name with -", s.run("cat -n < tests/t2"), "cat -n < tests/t2\n" + numbered(["1", "2"]))
    check("without flags cat is still a plain copy (a binary file survives)", s.run("cat tests/bigpad | wc -c"),
          f"cat tests/bigpad | wc -c\n{__import__('os').path.getsize(__import__('os').path.join(ctx.tests_dir, 'bigpad'))}\n")
    status("cat -n tests/nosuch", "cat: tests/nosuch: No such file or directory\n", 1)
    check("cat --help", s.run("cat --help"),
          "cat --help\nusage: cat [-n] [-E] [-T] [-s] [file...]\n  -n  number all output lines\n"
          "  -E  display $ at the end of each line\n  -T  display tabs as ^I\n  -s  squeeze runs of blank lines into one\n")

    # ================================================================= head / tail: several files, headers, -q, -v
    s.run("seq 1 12 > tests/h1")
    s.run("seq 1 3 > tests/h2")
    status("head -n 2 tests/h1 tests/h2", "==> tests/h1 <==\n1\n2\n\n==> tests/h2 <==\n1\n2\n")
    status("head -q -n 1 tests/h1 tests/h2", "1\n1\n")
    status("head -v -n 1 tests/h1", "==> tests/h1 <==\n1\n")
    status("head -n 1 tests/h1", "1\n")  # one file: no header
    status("head -c 2 tests/h1 tests/h2", "==> tests/h1 <==\n1\n\n==> tests/h2 <==\n1\n")
    status("tail -n 2 tests/h1 tests/h2", "==> tests/h1 <==\n11\n12\n\n==> tests/h2 <==\n2\n3\n")
    status("tail -q -n 1 tests/h1 tests/h2", "12\n3\n")
    status("tail -v -n 1 tests/h2", "==> tests/h2 <==\n3\n")
    status("head -n 1 tests/h2 tests/nosuch tests/h1",
           "==> tests/h2 <==\n1\nhead: cannot open 'tests/nosuch' for reading: No such file or directory\n\n==> tests/h1 <==\n1\n", 1)
    status("tail -n 1 tests/nosuch tests/h2",
           "tail: cannot open 'tests/nosuch' for reading: No such file or directory\n==> tests/h2 <==\n3\n", 1)  # no header for a file that fails

    # ================================================================= head -n -N, -c -N
    status("head -n -10 tests/h1", "1\n2\n")
    status("head -n -11 tests/h1", "1\n")
    status("head -n -12 tests/h1", "")
    status("head -n -20 tests/h1", "")
    status("head -n -0 tests/h1", "".join(f"{i}\n" for i in range(1, 13)))
    status("head --lines=-10 tests/h1", "1\n2\n")
    status("head -n +2 tests/h1", "1\n2\n")  # a plus is the same as no sign
    check("head -c -N: all but the last N bytes", s.run("head -c -3 tests/h2 | wc -c"), "head -c -3 tests/h2 | wc -c\n3\n")
    check("head -c -N with N past the end", s.run("head -c -100 tests/h2 | wc -c"), "head -c -100 tests/h2 | wc -c\n0\n")
    check("head -n -N from stdin", s.run("seq 1 12 | head -n -10"), "seq 1 12 | head -n -10\n1\n2\n")
    check("...with an unterminated last line", s.run("cat tests/t3 | head -n -0"), "cat tests/t3 | head -n -0\nend\n")
    status("head -n -x tests/h1", "head: invalid number of lines: '-x'\n", 1)
    status("head -n + tests/h1", "head: invalid number of lines: '+'\n", 1)
    status("head -c -x tests/h1", "head: invalid number of bytes: '-x'\n", 1)

    # ================================================================= tail -n +N, -c +N
    status("tail -n +11 tests/h1", "11\n12\n")
    status("tail -n +1 tests/h2", "1\n2\n3\n")
    status("tail -n +0 tests/h2", "1\n2\n3\n")  # from the first: everything
    status("tail -n +4 tests/h2", "")
    status("tail -n +12 tests/h1", "12\n")
    status("tail --lines=+11 tests/h1", "11\n12\n")
    status("tail -n -2 tests/h1", "11\n12\n")  # a minus is the same as no sign: the last two
    check("tail -c +N: from byte N on", s.run("tail -c +5 tests/h2 | wc -c"), "tail -c +5 tests/h2 | wc -c\n2\n")
    check("...from the first byte", s.run("tail -c +1 tests/h2 | wc -c"), "tail -c +1 tests/h2 | wc -c\n6\n")
    check("tail -n +N from stdin", s.run("seq 1 5 | tail -n +4"), "seq 1 5 | tail -n +4\n4\n5\n")
    check("...on a long stream", s.run("seq 1 100000 | tail -n +99999"), "seq 1 100000 | tail -n +99999\n99999\n100000\n")
    status("tail -n +x tests/h1", "tail: invalid number of lines: '+x'\n", 1)
    check("head --help", s.run("head --help"),
          "head --help\nusage: head [-q] [-v] [-n N | -c N] [file...]\n"
          "  -n N  print the first N lines (default 10); -n -N prints all but the last N\n"
          "  -c N  print the first N bytes; -c -N prints all but the last N\n"
          "  -q  never print headers giving file names\n  -v  always print headers giving file names\n")
    check("tail --help", s.run("tail --help"),
          "tail --help\nusage: tail [-q] [-v] [-n N | -c N] [file...]\n"
          "  -n N  print the last N lines (default 10); -n +N prints from line N on\n"
          "  -c N  print the last N bytes; -c +N prints from byte N on\n"
          "  -q  never print headers giving file names\n  -v  always print headers giving file names\n")

    # ================================================================= wc -m
    text = ctx.fixture("cjk.txt")  # three CJK characters and a newline
    data = text.encode()
    status("wc -m tests/cjk.txt", f"{len(text)} tests/cjk.txt\n")
    status("wc -c tests/cjk.txt", f"{len(data)} tests/cjk.txt\n")
    status("wc -lwmc tests/cjk.txt", f"1 1 {len(text)} {len(data)} tests/cjk.txt\n")  # lines words chars bytes
    status("wc tests/cjk.txt", f"1 1 {len(data)} tests/cjk.txt\n")  # the default is lines, words, bytes
    status("wc -m tests/h2", "6 tests/h2\n")  # ASCII: the same as bytes
    status("wc -m tests/h2 tests/cjk.txt", f"6 tests/h2\n{len(text)} tests/cjk.txt\n{6 + len(text)} total\n")
    check("wc -m from stdin", s.run("cat tests/cjk.txt | wc -m"), f"cat tests/cjk.txt | wc -m\n{len(text)}\n")
    check("wc --help", s.run("wc --help"),
          "wc --help\nusage: wc [-l] [-w] [-m] [-c] [-L] [file...]\n  -l  count lines\n  -w  count words\n  -m  count characters\n"
          "  -c  count bytes\n  -L  report the longest line's length\n")

    # ================================================================= echo -e -E
    status("echo -e 'a\\tb'", "a\tb\n")
    status("echo -e 'one\\ntwo'", "one\ntwo\n")
    status("echo -e 'a\\\\b'", "a\\b\n")  # \\ is one backslash
    status("echo -e '\\x41\\x4a'", "AJ\n")
    status("echo -e '\\0101'", "A\n")  # \0NNN, octal
    status("echo -e 'q\\qq'", "q\\qq\n")  # an unknown escape is left as written
    status("echo -e 'a\\'", "a\\\n")  # a trailing backslash
    status("echo -e 'x\\x'", "x\\x\n")  # \x with no digits
    check("\\c stops all output, the newline too", s.run("echo -e 'ab\\cde' fg | wc -c"), "echo -e 'ab\\cde' fg | wc -c\n2\n")
    check("echo -ne", s.run("echo -ne 'a\\tb' | wc -c"), "echo -ne 'a\\tb' | wc -c\n3\n")
    check("echo -en, the other order", s.run("echo -en 'a\\tb' | wc -c"), "echo -en 'a\\tb' | wc -c\n3\n")
    check("echo -n -e, separately", s.run("echo -n -e 'a\\tb' | wc -c"), "echo -n -e 'a\\tb' | wc -c\n3\n")
    status("echo -e -E 'a\\tb'", "a\\tb\n")  # the last of -e and -E wins
    status("echo -E 'a\\tb'", "a\\tb\n")  # the default
    status("echo 'a\\tb'", "a\\tb\n")
    status("echo -e", "\n")
    status("echo -x foo", "-x foo\n")  # not an option group: text
    status("echo -n -x", "-x\n")
    status("echo -e -x", "-x\n")
    status("echo -ex foo", "-ex foo\n")  # a group with a letter other than n, e, E is text
    status("echo foo -e", "foo -e\n")  # options only lead
    check("echo --help", s.run("echo --help"),
          "echo --help\nusage: echo [-neE] args...\n  -n  suppress the trailing newline\n  -e  interpret backslash escapes\n"
          "  -E  do not interpret backslash escapes (the default)\n")

    check("the shell is alive", s.run("echo ok"), "echo ok\nok\n")
