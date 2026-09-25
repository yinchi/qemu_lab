"""Last updated: Stage 17, Step 2.

No `/etc/environment` at all: the initial environment is empty, a note says so on the serial log, and the shell starts.
"""

ENVIRONMENT = None  # the harness removes the file from this group's image


def run(ctx):
    s, check = ctx.s, ctx.check
    boot = s.log()
    check("boot: the missing file is reported", "Environment: /etc/environment: No such file or directory -- starting empty." in boot, True)
    check("the shell started anyway", s.run("echo ok"), "echo ok\nok\n")
    check("programs see an empty environment", s.run("env"), "env\n")
    check("...until something is exported", s.run("export A=1"), "export A=1\n")
    check("...which they then do", s.run("env"), "env\nA=1\n")
