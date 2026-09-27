"""Last updated: Stage 17, Step 4.

`NAME=value` on its own sets a shell variable (readable with `$NAME`, but not exported: a program does not see it
until `export`); `NAME=value command` gives that one command the variable for as long as it runs and puts things
back afterwards. The value is expanded but never split; a `NAME=` word only counts before the command word.

The rules for what is an assignment word are host tests (`lexer.rs`, `syntax.rs`); the save-and-restore of the
variables is one too (`frame_stack.rs`). Here it is the shell end to end, seen through `printenv`, `env` and
`echo`. The group's `/etc/environment` is `HOME=/` and `SP=x  y` (two blanks).
"""

ENVIRONMENT = "HOME=/\nSP=x  y\n"


def run(ctx):
    s, check = ctx.s, ctx.check
    s.run("chmod +x tests/probe")

    def echo(line, printed):
        check(line, s.run(line), f"{line}\n{printed}\n")

    def no(name):
        """`printenv NAME` for a variable programs do not see."""
        check(f"printenv {name}: not in the environment", s.run_status(f"printenv {name}"), (f"printenv {name}\n", 1))

    # --- on its own: a shell variable, not exported ---
    check("assign", s.run("FOO=bar"), "FOO=bar\n")
    echo("echo $FOO", "bar")
    no("FOO")
    check("...and not in `env`", s.run("env"), "env\nHOME=/\nSP=x  y\n")
    check("export marks it", s.run("export FOO"), "export FOO\n")
    check("...now a program sees it", s.run("printenv FOO"), "printenv FOO\nbar\n")
    check("assigning again keeps it exported", s.run("FOO=baz"), "FOO=baz\n")
    check("...with the new value", s.run("printenv FOO"), "printenv FOO\nbaz\n")
    check("unset", s.run("unset FOO"), "unset FOO\n")
    echo("echo [$FOO]", "[]")
    check("an empty value is set, and different from unset", s.run("FOO="), "FOO=\n")
    echo('echo "[$FOO]"', "[]")
    check("...exported, a program sees it as empty", s.run("export FOO"), "export FOO\n")
    check("printenv FOO", s.run("printenv FOO"), "printenv FOO\n\n")
    s.run("unset FOO")

    # --- the value: expanded, quoted, never split ---
    s.run('FOO="a b"')
    echo('echo "$FOO"', "a b")
    s.run("FOO='$HOME'")
    echo('echo "$FOO"', "$HOME")
    s.run("FOO=$HOME")
    echo('echo "$FOO"', "/")
    s.run("FOO=a$HOME.b${HOME}c")
    echo('echo "$FOO"', "a/.b/c")
    s.run("FOO=$SP")
    echo('echo "[$FOO]"', "[x  y]")  # no splitting: both blanks are kept
    s.run("FOO=$NOSUCH")
    echo('echo "[$FOO]"', "[]")
    s.run("FOO=a=b")
    echo('echo "$FOO"', "a=b")
    s.run("false")
    s.run("FOO=$?")
    echo('echo "$FOO"', "1")
    s.run("false")
    s.run("FOO=1")
    echo('echo "$?"', "0")  # an assignment alone is status 0

    # --- `export NAME=$X` is an assignment too: not split (bash's rule for its declaration commands) ---
    check("export with a value that has blanks", s.run("export FOO=$SP"), "export FOO=$SP\n")
    check("...is all of it", s.run("printenv FOO"), "printenv FOO\nx  y\n")
    check("several, and a name alone", s.run("export A=$SP B=$SP FOO"), "export A=$SP B=$SP FOO\n")
    check("...", s.run("printenv A B"), "printenv A B\nx  y\nx  y\n")
    check("an empty value", s.run("export A=$NOSUCH"), "export A=$NOSUCH\n")
    check("...is set", s.run("printenv A"), "printenv A\n\n")
    body = lambda out: out.split("\n", 1)[1]  # what the program printed, without the echoed command line
    check("other commands still split: the arguments are `A=x` and `y`",
          body(s.run("tests/probe args A=$SP")), body(s.run("tests/probe args A=x y")))
    s.run("unset FOO A B")

    # --- several, each seeing the ones before it ---
    s.run("A=1 B=$A C=${B}2")
    echo("echo $A $B $C", "1 1 12")
    s.run("unset A B C FOO")

    # --- `NAME=value command`: for that command only ---
    check("the command sees it", s.run("FOO=x printenv FOO"), "FOO=x printenv FOO\nx\n")
    no("FOO")
    echo("echo [$FOO]", "[]")
    check("as the last variable of `env`", s.run("FOO=x env"), "FOO=x env\nHOME=/\nSP=x  y\nFOO=x\n")
    check("several", s.run("A=1 B=2 printenv A B"), "A=1 B=2 printenv A B\n1\n2\n")
    check("later ones see earlier ones", s.run("A=1 B=$A printenv B"), "A=1 B=$A printenv B\n1\n")
    check("the command's own words see the old values", s.run("FOO=new echo [$FOO]"), "FOO=new echo [$FOO]\n[]\n")
    check("the exit status is the command's", s.run("FOO=1 false"), "FOO=1 false\n")
    echo('echo "$?"', "1")
    check("a program that fails to start leaves nothing behind", s.run("FOO=1 nosuchcommand"),
          "FOO=1 nosuchcommand\nnosuchcommand: command not found\n")
    no("FOO")
    check("an existing exported variable is replaced for the command", s.run("export V=orig"), "export V=orig\n")
    check("...", s.run("V=tmp printenv V"), "V=tmp printenv V\ntmp\n")
    check("...and restored", s.run("printenv V"), "printenv V\norig\n")
    check("an existing unexported one is exported for the command", s.run("U=1"), "U=1\n")
    check("...", s.run("U=2 printenv U"), "U=2 printenv U\n2\n")
    echo("echo $U", "1")
    no("U")  # ... and is not exported afterwards
    check("in a pipeline, and a redirect", s.run("FOO=x printenv FOO | cat > /tmp/o"), "FOO=x printenv FOO | cat > /tmp/o\n")
    check("...", s.run("cat /tmp/o"), "cat /tmp/o\nx\n")
    check("in a later stage", s.run("echo a | FOO=y printenv FOO"), "echo a | FOO=y printenv FOO\ny\n")
    no("FOO")
    s.run("unset V U")

    # --- a command word that expands to nothing: the assignment is the shell's ---
    s.run("FOO=kept $NOSUCH")
    echo("echo $FOO", "kept")
    no("FOO")
    s.run("unset FOO")

    # --- redirects around an assignment alone ---
    check("an assignment and a redirect", s.run("FOO=1 > /tmp/af"), "FOO=1 > /tmp/af\n")
    check("...creates the file", s.run("cat /tmp/af"), "cat /tmp/af\n")
    echo("echo $FOO", "1")
    s.run("unset FOO")

    # --- what is not an assignment ---
    echo("echo FOO=bar", "FOO=bar")
    echo("echo A=1 B=2", "A=1 B=2")
    check("a quoted name", s.run('"FOO"=bar'), '"FOO"=bar\nFOO=bar: command not found\n')
    check("a name that starts with a digit", s.run("1A=x"), "1A=x\n1A=x: command not found\n")
    check("a name with a dash", s.run("a-b=x"), "a-b=x\na-b=x: command not found\n")
    check("no name", s.run("=x"), "=x\n=x: command not found\n")
    check("...status 127", s.run("echo $?"), "echo $?\n127\n")
    echo("echo [$FOO]", "[]")

    # --- scripts: `./script` is a child that inherits only what is exported; `source` is the shell ---
    s.run("chmod +x tests/assign.sh")
    check("./script: its variable is its own, not exported until it says so", s.run("./tests/assign.sh"),
          "./tests/assign.sh\none\nstatus 1\none\nstatus 0\n")
    echo("echo [$LOCALV]", "[]")
    check("source: the variable stays", s.run("source tests/assign.sh"), "source tests/assign.sh\none\nstatus 1\none\nstatus 0\n")
    echo("echo $LOCALV", "one")
    check("...and was exported by the script", s.run("printenv LOCALV"), "printenv LOCALV\none\n")
    s.run("unset LOCALV")
    check("an assignment before a script is its environment, and so is exported in it", s.run("LOCALV=outer ./tests/assign.sh"),
          "LOCALV=outer ./tests/assign.sh\none\none\nstatus 0\none\nstatus 0\n")
    no("LOCALV")

    check("the shell is alive", s.run("echo ok"), "echo ok\nok\n")
