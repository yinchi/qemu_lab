"""Step 7 of `Stage12.md`: a command line is lexed and parsed (POSIX quoting, `#` comments, pipes and redirections
recognised) instead of split by `shlex`, and errors use bash's wording. Pipes and redirections parse but are not
run yet (Steps 8 and 11), and say so; what the parser accepts is tested exhaustively on the host, so this
module checks the shell end to end: what reaches a program's `argv`, and that a bad line is reported while the
prompt survives.
"""


def run(ctx):
    s, check = ctx.s, ctx.check

    def echo(line, output):
        check(f"echo: {line}", s.run(line), f"{line}\n{output}\n")

    # --- quoting: what reaches the program ---
    echo('echo "a b"', "a b")
    echo("echo 'a b'", "a b")
    echo(r"echo a\ b", "a b")
    echo('echo "|"', "|")
    echo("echo '|'", "|")
    echo(r"echo \|", "|")
    echo('echo ">" "<" ">>"', "> < >>")
    echo("echo '>'x", ">x")
    echo(r'echo "a\"b"', 'a"b')
    echo(r'echo "a\\b"', r"a\b")
    echo(r'echo "a\nb"', r"a\nb")  # any other backslash inside double quotes is itself
    echo("echo 'a\\b'", r"a\b")  # and single quotes keep everything
    echo('echo ""x', "x")
    echo("echo a'b c'd", "ab cd")

    # --- an empty argument is still an argument, and arrives as one ---
    out = s.run('tests/probe.exe args "" x')
    check("an empty quoted word is an argument", ("argc=4" in out, 'argv[2]=""' in out, 'argv[3]="x"' in out),
          (True, True, True))

    # --- comments start only at the start of a word ---
    echo("echo a #b", "a")
    echo("echo a#b", "a#b")
    echo('echo "#"', "#")
    echo(r"echo \#a", "#a")
    check("a comment-only line does nothing", s.run("# nothing to see"), "# nothing to see\n")
    check("a blank line does nothing", s.run(""), "\n")

    # --- $ * ? ~ { } are ordinary text for now ---
    echo("echo $x * ? ~ {a,b}", "$x * ? ~ {a,b}")

    # --- pipes and redirections are recognised, and refused for now ---
    check("a pipe is not run yet", s.run("echo a | cat"), "echo a | cat\npipes are not supported yet\n")
    check("a redirection is not run yet", s.run("echo a>b"), "echo a>b\nredirection is not supported yet\n")
    check("...with a descriptor number", s.run("echo a 2>b"), "echo a 2>b\nredirection is not supported yet\n")
    check("a digit that is not a descriptor is text", s.run("echo 2"), "echo 2\n2\n")

    # --- syntax errors are reported and the prompt survives ---
    check("unterminated double quote", s.run('echo "abc'), 'echo "abc\nsyntax error: unterminated quote\n')
    check("unterminated single quote", s.run("echo 'abc"), "echo 'abc\nsyntax error: unterminated quote\n")
    check("trailing backslash", s.run("echo abc\\"), "echo abc\\\nsyntax error: a backslash at the end of the line\n")
    check("a redirection with no file", s.run("cat >"), "cat >\nsyntax error: no file name after `>`\n")
    check("a bad duplication", s.run("echo a 2>&x"), "echo a 2>&x\nsyntax error: `>&` must be followed by 1 or 2\n")
    check("an empty pipeline stage", s.run("| echo a"), "| echo a\nsyntax error: missing command\n")
    check("...at the end", s.run("echo a |"), "echo a |\nsyntax error: missing command\n")
    check("a semicolon is not supported", s.run("echo a; echo b"), "echo a; echo b\nsyntax error: `;` is not supported\n")
    check("nor is an ampersand", s.run("echo a &"), "echo a &\nsyntax error: `&` is not supported\n")
    check("the shell is alive after all that", s.run("echo alive"), "echo alive\nalive\n")

    # --- bash's wording for the launcher's errors ---
    check("unknown command", s.run("nosuch"), "nosuch\nnosuch: command not found\n")
    check("a quoted command word is still a command", s.run('"nosuch"'), '"nosuch"\nnosuch: command not found\n')

