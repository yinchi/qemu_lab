"""Last updated: Stage 17, Step 7.

`$HOME`: where the init shell starts (there is no login program, so the shell does the `chdir` itself at boot,
after reading `/etc/environment`) and where `cd` with no operand goes. This group's file says `HOME=/tests`.
`home_bad` boots with a `HOME` that names no directory, and `env_missing` with none at all.
"""

ENVIRONMENT = "HOME=/tests\n"


def run(ctx):
    s, check = ctx.s, ctx.check

    boot = s.log()
    check("boot: no complaint about HOME", "cannot enter" not in boot, True)
    check("the shell starts in $HOME", s.run("pwd"), "pwd\n/tests\n")
    check("...so a relative path is relative to it", s.run("cat hello.txt"), "cat hello.txt\n" + ctx.fixture("hello.txt"))
    check("cd elsewhere", s.run("cd /bin"), "cd /bin\n")
    check("cd with no operand goes to $HOME", s.run("cd"), "cd\n")
    check("...", s.run("pwd"), "pwd\n/tests\n")

    # --- it is read when `cd` runs, so a change (or an overlay) takes effect at once ---
    check("HOME for one command: a builtin sees the overlay", s.run("HOME=/fonts cd"), "HOME=/fonts cd\n")
    check("...", s.run("pwd"), "pwd\n/fonts\n")
    check("...and it is gone again", s.run("cd"), "cd\n")
    check("...", s.run("pwd"), "pwd\n/tests\n")
    s.run("export HOME=/bin")
    s.run("cd /")
    check("after `export HOME=/bin`, cd goes there", s.run("cd"), "cd\n")
    check("...", s.run("pwd"), "pwd\n/bin\n")
    check("an explicit operand is not affected", s.run("cd /fonts"), "cd /fonts\n")
    check("...", s.run("pwd"), "pwd\n/fonts\n")

    # --- a stage of a pipeline is a subshell: a cd in it does not move the shell ---
    check("cd in a pipeline stage", s.run("cd | cat"), "cd | cat\n")
    check("...leaves the directory", s.run("pwd"), "pwd\n/fonts\n")

    # --- HOME missing, empty, or wrong ---
    s.run("unset HOME")
    check("cd with HOME unset", s.run_status("cd"), ("cd\ncd: HOME not set\n", 1))
    check("...leaves the directory", s.run("pwd"), "pwd\n/fonts\n")
    s.run("export HOME=")
    check("cd with HOME empty", s.run_status("cd"), ("cd\ncd: HOME not set\n", 1))
    s.run("export HOME=/nosuchdir")
    check("cd with HOME naming nothing", s.run_status("cd"), ("cd\ncd: /nosuchdir: No such file or directory\n", 1))
    s.run("export HOME=/tests/hello.txt")
    check("...or a file", s.run_status("cd"), ("cd\ncd: /tests/hello.txt: Not a directory\n", 1))
    check("...still where it was", s.run("pwd"), "pwd\n/fonts\n")
    check("an operand still works with HOME unset", s.run("cd /"), "cd /\n")
    check("the shell is alive", s.run("echo ok"), "echo ok\nok\n")
