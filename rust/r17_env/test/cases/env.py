"""Last updated: Stage 17, Step 2.

What a program is given as its environment: the shell's exported variables, as `NAME=VALUE` strings in `envp`
(`x2`), decoded by `userlib::env`. `env` prints them all, `printenv` some; `probe env` shows the array's layout.

This group boots with an `/etc/environment` whose values exercise the file's format: a value is everything after
the first `=` (a space, quotes, `$` and an empty value all literal). `environment.py` covers the boot notes and
what `export` refuses; the frame's rules for who inherits what are host tests in `frame_stack.rs`.
"""

ENVIRONMENT = "# test environment\nHOME=/\nTZ=UTC\nGREETING=hello world\nEMPTY=\nQ=\"quoted\" $x\n"

INITIAL = "HOME=/\nTZ=UTC\nGREETING=hello world\nEMPTY=\nQ=\"quoted\" $x\n"

BIG = ["BIG1", "BIG2", "BIG3", "BIG4", "BIG5"]


def run(ctx):
    s, check = ctx.s, ctx.check

    # --- the file's variables reach a program, in file order, values literal ---
    check("env prints the initial environment", s.run("env"), "env\n" + INITIAL)
    check("printenv with no name is the same", s.run("printenv"), "printenv\n" + INITIAL)
    check("printenv HOME", s.run("printenv HOME"), "printenv HOME\n/\n")
    check("a value with a space", s.run("printenv GREETING"), "printenv GREETING\nhello world\n")
    check("an empty value is set, and prints an empty line", s.run("printenv EMPTY"), "printenv EMPTY\n\n")
    check("quotes and $ in the file are literal", s.run("printenv Q"), "printenv Q\n\"quoted\" $x\n")
    check("a name that is not set: nothing, status 1", s.run_status("printenv NOSUCH"), ("printenv NOSUCH\n", 1))
    check("several names, in the order given; one missing makes it 1", s.run_status("printenv TZ NOSUCH HOME"), ("printenv TZ NOSUCH HOME\nUTC\n/\n", 1))
    check("names are case-sensitive", s.run_status("printenv home"), ("printenv home\n", 1))
    check("a name with '=' is not a variable", s.run_status("printenv HOME=/"), ("printenv HOME=/\n", 1))

    # --- export and unset change what the next program sees ---
    s.run("export FOO=bar")
    check("an exported variable is inherited", s.run("printenv FOO"), "printenv FOO\nbar\n")
    check("...and appears after the existing ones", s.run("env"), "env\n" + INITIAL + "FOO=bar\n")
    s.run("export FOO=baz")
    check("exporting again replaces the value, in place", s.run("env"), "env\n" + INITIAL + "FOO=baz\n")
    s.run("export HOME")
    check("export NAME alone keeps the value", s.run("printenv HOME"), "printenv HOME\n/\n")
    s.run("unset FOO")
    check("unset removes it", s.run_status("printenv FOO"), ("printenv FOO\n", 1))
    check("...and nothing else", s.run("env"), "env\n" + INITIAL)

    # --- a redirect or a pipeline does not change the environment; both programs see it ---
    check("redirected", s.run("printenv HOME > /tmp/o"), "printenv HOME > /tmp/o\n")
    check("the file has it", s.run("cat /tmp/o"), "cat /tmp/o\n/\n")
    check("piped", s.run("printenv GREETING | wc -c"), "printenv GREETING | wc -c\n12\n")

    # --- a script is a child scope: it inherits, and what it exports goes with it; `source` is the shell itself ---
    s.run("chmod +x tests/exportenv.sh")
    check("a script sees the exports and can make its own",
          s.run("./tests/exportenv.sh"), "./tests/exportenv.sh\ninside\n")
    check("...which are gone afterwards", s.run_status("printenv SCRIPTVAR"), ("printenv SCRIPTVAR\n", 1))
    check("source keeps them", s.run("source tests/exportenv.sh"), "source tests/exportenv.sh\ninside\n")
    check("...so a later command has it", s.run("printenv SCRIPTVAR"), "printenv SCRIPTVAR\ninside\n")
    s.run("unset SCRIPTVAR")

    # --- the options ---
    check("env takes no operand (the shell runs a command in a changed environment, Step 4)", s.run_status("env FOO=bar"), ("env FOO=bar\nenv: extra operand 'FOO=bar'\nTry 'env --help' for more information.\n", 1))
    check("env -i", s.run_status("env -i"), ("env -i\nenv: invalid option -- 'i'\nTry 'env --help' for more information.\n", 1))
    check("env --help", s.run("env --help"), "env --help\nusage: env\n")
    check("printenv --help", s.run("printenv --help"), "printenv --help\nusage: printenv [NAME]...\n")
    check("printenv -x", s.run_status("printenv -x"), ("printenv -x\nprintenv: invalid option -- 'x'\nTry 'printenv --help' for more information.\n", 1))

    # --- the layout of envp on the stack ---
    s.run("chmod +x tests/probe.exe")
    check("probe env: NULL-terminated, right after argv's NULL, and in step with vars()",
          s.run("tests/probe.exe env"),
          "tests/probe.exe env\nenvc=5\nenvp[envc] is NULL: yes\nenvp directly follows argv's NULL: yes\nvars() agrees: yes\n")
    s.run("export A=1 B=2")
    check("...with more", s.run("tests/probe.exe env").split("\n")[1], "envc=7")
    s.run("unset A B")

    # --- arguments and environment share one limit (ARG_MAX, 128 KiB) ---
    check("exports that overflow it", s.run("source tests/bigenv.sh"), "source tests/bigenv.sh\n")
    check("no program can start", s.run("env"), "env\nenv: Argument list too long\n")
    check("nor a program given no arguments beyond its name", s.run("tests/probe.exe env"),
          "tests/probe.exe env\ntests/probe.exe: Argument list too long\n")
    check("a builtin still runs", s.run("unset " + " ".join(BIG)), "unset " + " ".join(BIG) + "\n")
    check("and then programs do", s.run("printenv HOME"), "printenv HOME\n/\n")
    check("the shell is alive", s.run("echo ok"), "echo ok\nok\n")
