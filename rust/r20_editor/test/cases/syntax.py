"""Last updated: Stage 12, Step 11.

A command line is lexed and parsed (POSIX quoting, `#` comments, pipes and redirections recognised)
instead of split by `shlex`, and errors use bash's wording. Both pipes and redirections run for
real (Step 11 and Step 8 respectively; pipe execution itself is tested thoroughly in `pipes.py`,
redirection in `redirection.py`) -- what's checked here is only that they parse and reach a
program correctly. What the parser accepts is tested exhaustively on the host, so this module
checks the shell end to end: what reaches a program's `argv`, and that a bad line is reported
while the prompt survives.
"""


def run(ctx):
    s, check = ctx.s, ctx.check
    s.run("chmod +x tests/probe")  # self-sufficient, same reason as cwd.py's own copy of this line

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
    out = s.run('tests/probe args "" x')
    check("an empty quoted word is an argument", ("argc=4" in out, 'argv[2]=""' in out, 'argv[3]="x"' in out),
          (True, True, True))

    # --- comments start only at the start of a word ---
    echo("echo a #b", "a")
    echo("echo a#b", "a#b")
    echo('echo "#"', "#")
    echo(r"echo \#a", "#a")
    check("a comment-only line does nothing", s.run("# nothing to see"), "# nothing to see\n")
    check("a blank line does nothing", s.run(""), "\n")

    # --- * ? ~ { } are ordinary text (no globbing, tilde or brace expansion); `$` expands (expansion.py), except
    # where it names nothing ---
    echo("echo * ? ~ {a,b}", "* ? ~ {a,b}")
    echo("echo $ a$ $1 $$ $@", "$ a$ $1 $$ $@")
    check("${ that is not ${NAME} is refused", s.run("echo ${"), "echo ${\nsyntax error: bad substitution\n")
    check("...whatever follows it", s.run("echo ${a-b}"), "echo ${a-b}\nsyntax error: bad substitution\n")

    # --- pipes and redirections both run for real (Step 11 pipe execution tested thoroughly in
    # pipes.py; this just checks a pipe reaches a program's argv/runs at all) ---
    check("a pipe runs", s.run("echo a | cat"), "echo a | cat\na\n")
    check("a redirection parses and runs", s.run("echo a>synredir.txt"), "echo a>synredir.txt\n")
    check("...and took effect", s.run("cat synredir.txt"), "cat synredir.txt\na\n")
    check("...with a descriptor number", s.run("echo a 2>synredir2.txt"),
          "echo a 2>synredir2.txt\na\n")  # stdout unaffected: echo never writes to fd 2
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

