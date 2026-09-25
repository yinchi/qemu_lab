"""Last updated: Stage 17, Step 3.

`$NAME`, `${NAME}` and `$?`: what the shell substitutes into a command line just before running it. Words are
split into fields the POSIX way (an unquoted expansion at blanks, a quoted one never), an unquoted empty expansion
disappears, and `$?` is the last pipeline's status (a program's, 139 for a fault, 127 not found, 126 found but not
runnable, 2 a syntax error, 1 for a builtin that failed or a redirect that could not be made).

The exact rules for splitting and quoting are host tests in `expand.rs` and `lexer.rs`; here it is the shell end to
end: what a program's `argv` holds, where an expansion may appear (a command word, a redirect target, a builtin's
operand) and where `$?` comes from. The variables are the ones the group's `/etc/environment` defines.
"""

ENVIRONMENT = (
    "HOME=/\nSP=a b  c\nEMPTY=\nBLANKS=   \nCMD=echo\nOUT=/tmp/exp.txt\nDIR=/tests\nQ=\"x\" 'y' $HOME\n"
)


def run(ctx):
    s, check = ctx.s, ctx.check
    s.run("chmod +x tests/probe.exe")

    def echo(line, printed):
        check(line, s.run(line), f"{line}\n{printed}\n")

    def args(line):
        """The arguments after `args` that `tests/probe.exe args ...` got."""
        out = s.run("tests/probe.exe args " + line)
        return [l.split("=", 1)[1] for l in out.split("\n") if l.startswith("argv[") and "]=" in l and not l.startswith(("argv[0]", "argv[1]"))]

    # --- a variable is replaced by its value ---
    echo("echo $HOME", "/")
    echo("echo ${HOME}", "/")
    echo("echo [$HOME]", "[/]")
    echo("echo $HOME/tests", "//tests")
    echo("echo ${HOME}tests", "/tests")
    echo("echo $CMD $CMD", "echo echo")
    echo("echo $Q", "\"x\" 'y' $HOME")  # a value is not read again: its quotes and `$` stay as they are
    check("a variable set by `export`", s.run("export FOO=bar"), "export FOO=bar\n")
    echo("echo $FOO", "bar")
    echo("echo $FOO$FOO ${FOO}x x$FOO", "barbar barx xbar")
    echo("echo $FOO.txt", "bar.txt")
    echo("echo $FOO-$FOO", "bar-bar")
    check("...gone once unset", s.run("unset FOO"), "unset FOO\n")
    echo("echo [$FOO]", "[]")

    # --- quoting ---
    s.run("export FOO=bar")
    echo("echo '$FOO'", "$FOO")
    echo('echo "$FOO"', "bar")
    echo('echo "a $FOO b"', "a bar b")
    echo(r"echo \$FOO", "$FOO")
    echo(r'echo "\$FOO"', "$FOO")
    echo('echo "${FOO}bar"', "barbar")
    echo("echo 'a'$FOO'b'", "abarb")
    echo("echo $ a$ $1 $$", "$ a$ $1 $$")
    echo('echo "$"', "$")

    # --- splitting: what the program's argv gets ---
    check("an unquoted value splits at blanks", args("$SP"), ['"a"', '"b"', '"c"'])
    check("...a quoted one does not", args('"$SP"'), ['"a b  c"'])
    check("...text around it joins the first and last field", args("[$SP]"), ['"[a"', '"b"', '"c]"'])
    check("an unquoted empty value is no argument", args("x $EMPTY y"), ['"x"', '"y"'])
    check("...nor is one of only blanks", args("x $BLANKS y"), ['"x"', '"y"'])
    check("...nor one that is not set", args("x $NOSUCH y"), ['"x"', '"y"'])
    check("a quoted empty value is an empty argument", args('x "$EMPTY" y'), ['"x"', '""', '"y"'])
    check("...as is `\"\"` next to an empty one", args("$EMPTY''"), ['""'])
    check("a value of blanks, quoted, is kept", args('"$BLANKS"'), ['"   "'])
    check("two expansions in one word", args("$SP$SP"), ['"a"', '"b"', '"ca"', '"b"', '"c"'])

    # --- the command word and where else an expansion may appear ---
    echo("$CMD hi", "hi")
    check("a command word that expands to nothing runs nothing", s.run("$NOSUCH"), "$NOSUCH\n")
    check("...and its arguments become the command", s.run("$NOSUCH echo shifted"), "$NOSUCH echo shifted\nshifted\n")
    check("an operand of a builtin", s.run("cd $DIR"), "cd $DIR\n")
    echo("pwd", "/tests")
    check("`cd` with the value of $HOME", s.run("cd $HOME"), "cd $HOME\n")
    echo("pwd", "/")

    # --- redirect targets: one word, or `ambiguous redirect` ---
    check("a redirect target", s.run("echo hi > $OUT"), "echo hi > $OUT\n")
    check("...went where the value says", s.run("cat /tmp/exp.txt"), "cat /tmp/exp.txt\nhi\n")
    check("an input redirect", s.run("cat < $OUT"), "cat < $OUT\nhi\n")
    check("a target that is not set", s.run("echo x > $NOSUCH"), "echo x > $NOSUCH\n$NOSUCH: ambiguous redirect\n")
    check("...one that splits into two", s.run("echo x > $SP"), "echo x > $SP\n$SP: ambiguous redirect\n")
    check("...but not when it is quoted", s.run('echo x > "$OUT"'), 'echo x > "$OUT"\n')
    check("nothing ran for the ambiguous ones", s.run("cat $OUT"), "cat $OUT\nx\n")
    check("`>&` needs a digit, not a variable", s.run("echo x 2>&$FOO"),
          "echo x 2>&$FOO\nsyntax error: `>&` must be followed by 1 or 2\n")

    # --- $? ---
    echo("echo $?", "2")  # the last line was a syntax error
    check("after a failing program", s.run("false"), "false\n")
    echo("echo $?", "1")
    echo("echo $?", "0")  # ...and the `echo` itself succeeded
    out = s.run("tests/probe.exe poke 0")
    check("after a fault", "Segmentation fault" in out, True)
    echo("echo $?", "139")
    check("after a command that is not found", s.run("nosuchcommand"), "nosuchcommand\nnosuchcommand: command not found\n")
    echo("echo $?", "127")
    check("...also given as a path", s.run("tests/nosuch.exe"),
          "tests/nosuch.exe\ntests/nosuch.exe: No such file or directory\n")
    echo("echo $?", "127")
    check("after a file with no exec bit", s.run("tests/hello.txt"), "tests/hello.txt\ntests/hello.txt: Permission denied\n")
    echo("echo $?", "126")
    check("after a directory", s.run("tests/docs"), "tests/docs\ntests/docs: Is a directory\n")
    echo("echo $?", "126")
    check("after a syntax error", s.run("echo 'x"), "echo 'x\nsyntax error: unterminated quote\n")
    echo("echo $?", "2")
    check("after a builtin that failed", s.run("cd /nosuchdir"), "cd /nosuchdir\ncd: /nosuchdir: No such file or directory\n")
    echo("echo $?", "1")
    check("after one that worked", s.run("cd /"), "cd /\n")
    echo("echo $?", "0")
    check("after a redirect that could not be made", s.run("echo x > /nosuchdir/f"),
          "echo x > /nosuchdir/f\n/nosuchdir/f: No such file or directory\n")
    echo("echo $?", "1")
    check("after an ambiguous one", s.run("echo x > $NOSUCH"), "echo x > $NOSUCH\n$NOSUCH: ambiguous redirect\n")
    echo("echo $?", "1")
    check("a blank line changes nothing", s.run("false"), "false\n")
    check("...even a blank one", s.run(""), "\n")
    check("...nor does a comment", s.run("# just a comment"), "# just a comment\n")
    echo("echo $?", "1")
    check("a pipeline's is its last stage's (a failing first one)", s.run("false | true"), "false | true\n")
    echo("echo $?", "0")
    check("...and the other way round", s.run("true | false"), "true | false\n")
    echo("echo $?", "1")

    s.run("false")
    echo('echo "$?" x$?y', "1 x1y")

    # --- `$?` and scripts: a script's status is its last command's ---
    s.run("chmod +x tests/exportenv.sh")
    check("a script", s.run("./tests/exportenv.sh"), "./tests/exportenv.sh\ninside\n")
    echo("echo $?", "0")
    check("source of a script", s.run("source tests/bad.sh"), "source tests/bad.sh\nbefore\nnosuchcommand: command not found\nafter\n")
    echo("echo $?", "0")  # its last line, `echo after`, succeeded

    # --- a variable set in a sourced script is there afterwards; from a ./script it is not (env.py) ---
    check("source of a script that exports", s.run("source tests/exportenv.sh"), "source tests/exportenv.sh\ninside\n")
    echo("echo $SCRIPTVAR", "inside")
    s.run("unset SCRIPTVAR")

    check("the shell is alive", s.run("echo ok"), "echo ok\nok\n")
