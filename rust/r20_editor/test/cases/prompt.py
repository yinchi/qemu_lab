"""Last updated: Stage 17, Step 10.

`$PS1`: the prompt is `$PS1` with `\w` (the working directory), `\W` (its last component), `\$` (a `#`) and `\\`
filled in, worked out again each time a prompt is drawn -- so an assignment or a `cd` shows at the next prompt. Unset
or empty is the default `> `. The rules for the escapes, control characters and the length limit are host tests
(`prompt.rs`); here it is the shell and the line editor end to end.

The harness ends a transcript at the prompt's final `> `, so a longer prompt leaves its front part on the end of what
`run` returns: with `PS1=\w> ` at `/tests`, `run("echo a")` gives `echo a\na\n/tests`. Every `PS1` used here ends in `> `
because the harness waits for that.
"""

from harness import BACKSPACE, CTRL_U

LONG = "x" * 100 + "> "  # wider than the 80-column screen, so it wraps


def run(ctx):
    s, check = ctx.s, ctx.check

    def prompted(line, printed, prompt):
        """`line`'s transcript, ending with the next prompt's text before its `> `."""
        check(line, s.run(line), f"{line}\n{printed}{prompt}")

    prompted("echo a", "a\n", "")  # the default: nothing before the `> `

    # --- \w and \W follow the working directory, at the next prompt ---
    prompted("PS1='\\w> '", "", "/")
    prompted("cd /tests", "", "/tests")
    prompted("echo a", "a\n", "/tests")
    prompted("cd /bin", "", "/bin")
    prompted("PS1='\\W> '", "", "bin")
    prompted("cd /", "", "/")  # the root's last component is `/`
    prompted("cd /tests", "", "tests")
    prompted("cd docs", "", "docs")
    prompted("cd /", "", "/")

    # --- \$ and \\, and what is left alone ---
    prompted("PS1='\\$> '", "", "#")
    prompted("PS1='a\\\\b> '", "", "a\\b")  # a single backslash between a and b
    prompted("PS1='\\n\\x> '", "", "\\n\\x")  # unknown escapes stay as typed
    prompted("PS1='$HOME> '", "", "$HOME")  # no `$` expansion in a prompt
    prompted("PS1='[\\w]\\$> '", "", "[/]#")

    # --- unset and empty are the default; the value is the shell's own, not exported ---
    prompted("PS1=", "", "")
    prompted("PS1='p> '", "", "p")
    prompted("unset PS1", "", "")
    check("PS1 is not exported unless asked", s.run_status("printenv PS1"), ("printenv PS1\n", 1))
    prompted("export PS1='e> '", "", "e")
    check("...and then a program sees it", s.run("printenv PS1"), "printenv PS1\ne> \ne")
    prompted("unset PS1", "", "")

    # --- a stage of a pipeline or a script does not change the prompt ---
    prompted("PS1='k> ' | cat", "", "")
    prompted("PS1='k> '", "", "k")
    prompted("cd /tests | cat", "", "k")
    prompted("unset PS1", "", "")

    # --- editing and history with a prompt that is not two cells wide, and one that wraps ---
    s.run("PS1='ab> '")
    s.type("echo abcX")
    s.keys([BACKSPACE])
    s.type("d\n")
    check("typing and Backspace after a longer prompt", s.wait_prompt(), "echo abcd\nabcd\nab")
    s.keys(["up"])
    s.type("\n")
    check("history recall (Up, Enter) redraws after it", s.wait_prompt(), "echo abcd\nabcd\nab")

    s.run(f"PS1='{LONG}'")
    s.type("echo abcX")
    s.keys([BACKSPACE])
    s.type("d")
    edited = s.screendump_settled()
    s.keys([CTRL_U])
    s.type("echo abcd")
    typed = s.screendump_settled()
    check("a prompt that wraps the row: an edited line looks as if typed in one go", edited == typed, True)
    s.type("\n")
    check("...and runs", s.wait_prompt(), "echo abcd\nabcd\n" + "x" * 100)
    prompted("unset PS1", "", "")

    check("the shell is alive", s.run("echo ok"), "echo ok\nok\n")
