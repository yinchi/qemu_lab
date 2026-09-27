"""Last updated: Stage 17, Step 2.

The shell's variables and the initial environment. The kernel reads `/etc/environment` (plain `NAME=VALUE` lines,
the format Linux's `pam_env` reads) once at boot, before the first prompt, into the shell's bottom frame, every variable
exported; a missing file or a bad line is a note on the serial log, never fatal. This group boots with the harness's
default file (`HOME=/`); `env_bad` and `env_missing` boot with a file that has bad lines and with none.

What is observable here is the boot note, what the environment holds at the first prompt (through `env`, Step 2's), and
what `export` and `unset` accept and refuse; `env.py` covers what programs then see. The frame's own rules (which variables a child
scope inherits, what a redirect leaves alone) are host tests in `frame_stack.rs`.
"""


def run(ctx):
    s, check = ctx.s, ctx.check

    boot = s.log()
    check("boot: the environment file was read", "Environment: 1 variable(s) from /etc/environment." in boot, True)
    check("boot: nothing was skipped", "ignored" not in boot, True)
    check("the variable is in the environment the first program gets", s.run("env"), "env\nHOME=/\n")

    # --- export ---
    check("export sets a variable", s.run("export FOO=bar"), "export FOO=bar\n")
    check("export several, with and without values", s.run("export A=1 B C=3"), "export A=1 B C=3\n")
    check("export of a name that is not set does nothing", s.run("export NOSUCHYET"), "export NOSUCHYET\n")
    check("export needs an operand", s.run("export"), "export\nexport: usage: export NAME[=VALUE]...\n")
    check("export refuses a bad name", s.run("export a-b=1"), "export a-b=1\nexport: 'a-b=1': not a valid identifier\n")
    check("...one starting with a digit", s.run("export 1x"), "export 1x\nexport: '1x': not a valid identifier\n")
    check("...an empty one", s.run("export =x"), "export =x\nexport: '=x': not a valid identifier\n")
    check("...and reports each bad one while doing the rest", s.run("export OK=1 a-b c.d"),
          "export OK=1 a-b c.d\nexport: 'a-b': not a valid identifier\nexport: 'c.d': not a valid identifier\n")
    check("export has no options here", s.run("export -p"), "export -p\nexport: -p: invalid option\n")

    # --- unset ---
    check("unset removes a variable", s.run("unset FOO"), "unset FOO\n")
    check("unset of one that is not set is not an error", s.run("unset FOO NOSUCH"), "unset FOO NOSUCH\n")
    check("unset with no operand is fine", s.run("unset"), "unset\n")
    check("unset refuses a bad name", s.run("unset a-b"), "unset a-b\nunset: 'a-b': not a valid identifier\n")
    check("unset has no options here", s.run("unset -v FOO"), "unset -v FOO\nunset: -v: invalid option\n")

    # --- they are builtins: they work under a redirect and in a pipeline stage without launching anything ---
    check("export under a redirect still runs", s.run("export R=1 > /tmp/o"), "export R=1 > /tmp/o\n")
    check("shell alive", s.run("echo ok"), "echo ok\nok\n")
