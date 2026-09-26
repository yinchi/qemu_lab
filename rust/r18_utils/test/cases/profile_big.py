"""Last updated: Stage 17, Step 11.

A profile over 64 KiB (as for `/etc/environment`) is not read: one note, and the shell starts in `$HOME`.
"""

ENVIRONMENT = "HOME=/tests\n"
PROFILE = "echo not-run\n" * 6000  # 78000 bytes


def run(ctx):
    s, check = ctx.s, ctx.check
    boot = s.log().replace("\r", "")
    check("the note", "Profile: /tests/.profile: not a text file of at most 64 KiB -- skipped." in boot, True)
    check("nothing of it ran", "not-run" not in boot, True)
    check("the shell started in $HOME", s.run("pwd"), "pwd\n/tests\n")
