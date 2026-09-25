"""Last updated: Stage 17, Step 11.

`$HOME/.profile`: a per-user start-up script the init shell runs after the environment file and `enter_home`, in the
shell itself (as `source` would), before the first prompt. This group's environment is `HOME=/tests` and `PATH=/bin`,
and its profile does one of everything: an `export`, a plain assignment, an expansion (`PATH=$PATH:...`), a `PS1`, a
command that prints, a `cd`, a command that fails, one more line after it, and a failing line last.

Two more groups cover a profile that cannot be run (`profile_bad`: not UTF-8; `profile_big`: over 64 KiB). A missing
profile is silent -- every other group has none, and `home` checks the log.
"""

ENVIRONMENT = "HOME=/tests\nPATH=/bin\n"

PROFILE = """# a comment, then a blank line

export X=from-profile
Y=plain
PATH=$PATH:/tmp/none
PS1='\\W> '
echo profile-ran
cd /bin
nosuchcmd1
echo after-bad-line
nosuchcmd2
"""


def run(ctx):
    s, check = ctx.s, ctx.check

    def prompted(line, printed):
        # the prompt is `\W> ` in /bin: `bin> `, whose front part is left on the end of a transcript
        check(line, s.run(line), f"{line}\n{printed}bin")

    boot = s.log().replace("\r", "")
    check("the profile ran before the first prompt: its output, a failing line, and the rest after it",
          boot.endswith("profile-ran\nnosuchcmd1: command not found\nafter-bad-line\nnosuchcmd2: command not found\nbin> "), True)
    check("no note: the profile was found and run", "Profile:" not in boot, True)

    prompted("echo $?", "127\n")  # the profile's last command failed, and `$?` is left as it set it (as in bash and dash)
    prompted("pwd", "/bin\n")  # its `cd` stuck: it is not a scope
    prompted("printenv X", "from-profile\n")
    prompted("echo [$Y]", "[plain]\n")
    check("...a plain assignment is the shell's, not exported", s.run_status("printenv Y"), ("printenv Y\nbin", 1))
    prompted("printenv PATH", "/bin:/tmp/none\n")  # `$PATH` expanded in it
    check("cd with no operand: HOME is still /tests", s.run("cd"), "cd\ntests")
    check("...", s.run("pwd"), "pwd\n/tests\ntests")
