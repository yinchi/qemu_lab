"""Last updated: Stage 17, Step 7.

A `HOME` that names no directory: the shell says so on the serial log and starts in `/`, as it does with no `HOME` at
all (`env_missing`). Never fatal.
"""

ENVIRONMENT = "HOME=/nosuchdir\n"


def run(ctx):
    s, check = ctx.s, ctx.check
    boot = s.log()
    check("boot: HOME is reported", "Environment: cannot enter HOME=/nosuchdir: No such file or directory -- staying in /." in boot, True)
    check("the shell starts in /", s.run("pwd"), "pwd\n/\n")
    check("the variable is still set", s.run("printenv HOME"), "printenv HOME\n/nosuchdir\n")
    check("cd with no operand fails the same way", s.run_status("cd"), ("cd\ncd: /nosuchdir: No such file or directory\n", 1))
    check("the shell is alive", s.run("echo ok"), "echo ok\nok\n")
