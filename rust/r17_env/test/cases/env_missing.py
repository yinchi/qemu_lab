"""Last updated: Stage 17, Step 1.

No `/etc/environment` at all: the initial environment is empty, a note says so on the serial log, and the shell starts.
"""

ENVIRONMENT = None  # the harness removes the file from this group's image


def run(ctx):
    s, check = ctx.s, ctx.check
    boot = s.log()
    check("boot: the missing file is reported", "Environment: /etc/environment: No such file or directory -- starting empty." in boot, True)
    check("the shell started anyway", s.run("echo ok"), "echo ok\nok\n")
    check("export still works in an empty environment", s.run("export A=1"), "export A=1\n")
