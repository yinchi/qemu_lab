"""Last updated: Stage 17, Step 11.

A profile that is not UTF-8 text: one note on the serial log, the profile is skipped, and the shell starts in `$HOME`
as usual. (An oversized one is `profile_big`; a directory named `.profile` is the same note with another reason.)
"""

ENVIRONMENT = "HOME=/tests\n"
PROFILE = b"\xff\xfe echo not-run\n"


def run(ctx):
    s, check = ctx.s, ctx.check
    boot = s.log().replace("\r", "")
    check("the note", "Profile: /tests/.profile: not a text file of at most 64 KiB -- skipped." in boot, True)
    check("nothing of it ran", "not-run" not in boot, True)
    check("the shell started in $HOME", s.run("pwd"), "pwd\n/tests\n")
    check("the shell is alive", s.run("echo ok"), "echo ok\nok\n")
