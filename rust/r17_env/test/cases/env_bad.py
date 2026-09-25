"""Last updated: Stage 17, Step 1.

A `/etc/environment` with bad lines: each is skipped and reported on the serial log with its line number, the good
ones are kept, a repeated name takes its last value, and the shell starts normally.
"""

ENVIRONMENT = "# a comment\nHOME=/\nnot a variable\n=orphan\n1x=2\n  \nTZ=UTC\nHOME=/x\nBAD NAME=y\n"


def run(ctx):
    s, check = ctx.s, ctx.check
    boot = s.log()
    for note in [
        "Environment: /etc/environment: line 3: expected NAME=VALUE (no '=') -- ignored.",
        "Environment: /etc/environment: line 4: expected NAME=VALUE (empty name) -- ignored.",
        "Environment: /etc/environment: line 5: not a valid variable name -- ignored.",
        "Environment: /etc/environment: line 9: not a valid variable name -- ignored.",
    ]:
        check(note.split(": ", 2)[2], note in boot, True)
    check("the good lines were kept (HOME twice counts once)", "Environment: 2 variable(s) from /etc/environment." in boot, True)
    check("the shell started anyway", s.run("echo ok"), "echo ok\nok\n")
