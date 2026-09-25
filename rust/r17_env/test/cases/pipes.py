"""Step 11: pipes via temp files (and, from Stage 17, each stage running in a subshell). See `Stage12.md`'s Step 11 section for the design -- this module
exercises execution end to end (the grammar itself is already covered by `syntax.py`'s host-level
parser tests and its own `"a pipe runs"` check).
"""


def run(ctx):
    s, check = ctx.s, ctx.check

    # ================================================================= basics
    check("two-stage pipe", s.run("echo hello | cat"), "echo hello | cat\nhello\n")
    check("cat | head", s.run("cat tests/hello.txt | head -n 3"),
          "cat tests/hello.txt | head -n 3\none\ntwo\nthree\n")
    check("three-stage chain", s.run("cat tests/hello.txt | head -n 3 | wc -l"),
          "cat tests/hello.txt | head -n 3 | wc -l\n3\n")
    check("ls | wc -l", s.run("ls tests/docs | wc -l"), "ls tests/docs | wc -l\n1\n")
    check("tee taps a pipeline's data while still passing it through",
          s.run("echo hello | tee tests/pipetee.txt | cat"),
          "echo hello | tee tests/pipetee.txt | cat\nhello\n")
    check("...and the file got a copy of it too", s.run("cat tests/pipetee.txt"),
          "cat tests/pipetee.txt\nhello\n")

    # ================================================================= exit status: last stage only
    check("false | true reports nothing (pipeline status is true's)",
          s.run("false | true"), "false | true\n")
    check("true | false has status 1 (the pipeline's is false's)", s.run_status("true | false"), ("true | false\n", 1))

    # ================================================================= pipe binds before a stage's own redirects
    check("a > f | b: a's output goes to f, b sees empty input",
          s.run("echo hi > tests/pf.txt | wc -c"), "echo hi > tests/pf.txt | wc -c\n0\n")
    check("...f really got echo's output", s.run("cat tests/pf.txt"), "cat tests/pf.txt\nhi\n")

    # ================================================================= stderr through the pipe
    check("cmd 2>&1 | b sends stderr through the pipe too",
          s.run("cat tests/nosuch.missing 2>&1 | wc -l"),
          "cat tests/nosuch.missing 2>&1 | wc -l\n1\n")
    check("without 2>&1, only stdout goes through (empty here, error stays on the console)",
          s.run("cat tests/nosuch.missing | wc -l"),
          "cat tests/nosuch.missing | wc -l\ncat: tests/nosuch.missing: No such file or directory\n0\n")

    # ================================================================= every stage always runs
    check("a missing first stage still lets the next stage run, on empty input",
          s.run("nosuchprogram | wc -c"), "nosuchprogram | wc -c\nnosuchprogram: command not found\n0\n")
    # `crash` writes "about to crash\n" (15 bytes) to its own stdout before faulting -- pipe-bound
    # here, not the console, so it doesn't appear on screen but does end up in the temp file `wc -c`
    # then reads (the write is unbuffered, and the pipe file is closed -- committing its size --
    # when the stage's redirect scope ends, fault or not). Only the fault message itself
    # bypasses redirection and reaches the console directly.
    check("a faulting middle stage still lets the last stage run",
          s.run("echo x | crash | wc -c"),
          "echo x | crash | wc -c\n"
          "Segmentation fault (address 0xffff800000000000, ESR_EL1 0x92000004)\n15\n")

    # ================================================================= temp-file naming: collision avoidance
    # A file at the counter's very first candidate name, created before any pipeline in this session
    # runs -- `next_pipe_path` must skip past it (via `stat`), not clobber it.
    s.run("echo user-data > /tmp/.pipe0")
    check("a pipeline runs fine even with something already at the first candidate name",
          s.run("echo hello | cat"), "echo hello | cat\nhello\n")
    check("...and that file is completely untouched", s.run("cat /tmp/.pipe0"),
          "cat /tmp/.pipe0\nuser-data\n")

    # ================================================================= no leftover temp files
    check("no leftover pipe temp files -- only the manually-created one from the collision test",
          s.run("ls /tmp"), "ls /tmp\n.pipe0\n")
    s.run("rm /tmp/.pipe0")

    # ================================================================= each stage is a subshell (Stage 17)
    # It starts with everything the shell has, unexported variables included, and nothing it does outlasts it -- so
    # a builtin or an assignment in a pipeline changes that stage only, and the same alone changes the shell.
    s.run("FOO=123")  # not exported
    check("a stage reads a variable the shell never exported", s.run("echo $FOO | cat"), "echo $FOO | cat\n123\n")
    check("echo $FOO | unset FOO prints nothing (the pipe took it)", s.run("echo $FOO | unset FOO"), "echo $FOO | unset FOO\n")
    check("...and FOO is still set", s.run('echo "$FOO"'), 'echo "$FOO"\n123\n')
    check("unset as the last stage", s.run("echo a | unset FOO"), "echo a | unset FOO\n")
    check("...", s.run('echo "$FOO"'), 'echo "$FOO"\n123\n')
    check("unset as the first stage", s.run("unset FOO | cat"), "unset FOO | cat\n")
    check("...", s.run('echo "$FOO"'), 'echo "$FOO"\n123\n')
    check("a lone unset does unset it", s.run("unset FOO"), "unset FOO\n")
    check("...", s.run("echo [$FOO]"), "echo [$FOO]\n[]\n")
    s.run("FOO=1")
    check("...and so does one with a redirect (that is not a pipeline)", s.run("unset FOO > /tmp/o"), "unset FOO > /tmp/o\n")
    check("...", s.run("echo [$FOO]"), "echo [$FOO]\n[]\n")

    check("cd in a stage", s.run("cd /tests | cat"), "cd /tests | cat\n")
    check("...leaves the working directory", s.run("pwd"), "pwd\n/\n")
    check("cd in the last stage", s.run("echo a | cd /tests"), "echo a | cd /tests\n")
    check("...too", s.run("pwd"), "pwd\n/\n")
    check("a lone cd changes it", s.run("cd /tests"), "cd /tests\n")
    check("...", s.run("pwd"), "pwd\n/tests\n")
    check("...and a stage runs where the shell is", s.run("pwd | cat"), "pwd | cat\n/tests\n")
    s.run("cd /")

    check("export in a stage", s.run("export X=1 | cat"), "export X=1 | cat\n")
    check("...exports nothing", s.run("printenv X"), "printenv X\n")
    check("an assignment in a stage", s.run("A=1 | cat"), "A=1 | cat\n")
    check("...sets nothing", s.run("echo [$A]"), "echo [$A]\n[]\n")
    check("nor does an earlier stage's reach a later one", s.run("A=1 | echo [$A]"), "A=1 | echo [$A]\n[]\n")
    check("a lone assignment sets it", s.run("A=1"), "A=1\n")
    check("...", s.run("echo [$A]"), "echo [$A]\n[1]\n")
    check("...and a stage sees it", s.run("echo $A | cat"), "echo $A | cat\n1\n")
    s.run("unset A")

    check("source in a stage", s.run("source tests/assign.sh | cat"),
          "source tests/assign.sh | cat\none\nstatus 1\none\nstatus 0\n")
    check("...leaves none of what it set", s.run("echo [$LOCALV]"), "echo [$LOCALV]\n[]\n")

    check("the pipeline's status is still the last stage's", s.run_status("unset NOSUCH | false"),
          ("unset NOSUCH | false\n", 1))
    check("the shell is alive", s.run("echo ok"), "echo ok\nok\n")
