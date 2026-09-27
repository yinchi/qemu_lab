"""Last updated: Stage 17, Step 10.

`PS1` in `/etc/environment` is in effect at the very first prompt (the file's value is literal text, so the backslash
escapes are still there to be filled in when the prompt is drawn), the shell starts in `$HOME`, and a prompt with a
wide character in it lays out and edits correctly. (The wide character can only come from the file: the test harness
types ASCII.)
"""

from harness import BACKSPACE, CTRL_U

ENVIRONMENT = "HOME=/tests\nPS1=日 \\W> \n"


def run(ctx):
    s, check = ctx.s, ctx.check

    log = s.log()
    check("the first prompt was drawn from PS1, at $HOME", log.endswith("\n日 tests> "), True)
    check("the variable is what the file said, backslash and all", s.run("printenv PS1"), "printenv PS1\n日 \\W> \n日 tests")
    check("the prompt follows the directory", s.run("cd /"), "cd /\n日 /")

    # A wide glyph in the prompt: an edited line looks as if it had been typed in one go, and it runs.
    s.type("echo abcX")
    s.keys([BACKSPACE])
    s.type("d")
    edited = s.screendump_settled()
    s.keys([CTRL_U])
    s.type("echo abcd")
    typed = s.screendump_settled()
    check("editing after a wide-character prompt redraws exactly", edited == typed, True)
    s.type("\n")
    check("...and runs", s.wait_prompt(), "echo abcd\nabcd\n日 /")

    s.run("unset PS1")
    check("unset: the default", s.run("echo ok"), "echo ok\nok\n")
