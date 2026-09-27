"""Last updated: Stage 12, Step 9.

Scripts: `source`/`.` runs a file's lines against the current frame (its `cd`s and redirects stick);
`sh FILE` and a `./`-executed file with the exec bit run them against a pushed one (`with_scope`), so
neither leaks past it, the same way a real child shell process would isolate them. A redirect on the
invoking line (`./s.sh > out`) covers every line the script runs, and is gone once the script's own
scope pops. A `#!` line is just a comment (there is only one interpreter). No positional parameters
yet, so extra arguments to any of the three forms are rejected. A recursion-depth cap (16) protects
the kernel stack from a script that sources or runs itself.

Runs from `/tests`, where the fixtures live; returns to `/` at the end.
"""


def run(ctx):
    s, check = ctx.s, ctx.check

    check("start from /tests", s.run("cd /tests"), "cd /tests\n")

    # --- missing / non-executable script errors, before any fixture gets its exec bit ---
    check("source of a missing file errors",
          s.run("source nosuchfile"), "source nosuchfile\nsource: nosuchfile: No such file or directory\n")
    check(". is the same builtin as source", s.run(". nosuchfile"),
          ". nosuchfile\n.: nosuchfile: No such file or directory\n")
    check("sh of a missing file errors",
          s.run("sh nosuchfile"), "sh nosuchfile\nsh: nosuchfile: No such file or directory\n")
    check("./ of a file without the exec bit is Permission denied (the ordinary launch check, not a "
          "script-specific one)", s.run("./bad.sh"), "./bad.sh\n./bad.sh: Permission denied\n")
    check("but sh doesn't need the exec bit", s.run("sh bad.sh"),
          "sh bad.sh\nbefore\nnosuchcommand: command not found\nafter\n")

    # --- ./script.sh runs scoped: its cd doesn't leak ---
    check("chmod +x cdbin.sh", s.run("chmod +x cdbin.sh"), "chmod +x cdbin.sh\n")
    check("./cdbin.sh scoped: prints nothing, cwd unaffected",
          (s.run("./cdbin.sh"), s.run("pwd")), ("./cdbin.sh\n", "pwd\n/tests\n"))

    # --- source/. run unscoped: their cd sticks ---
    check("source cdbin.sh moves the shell", s.run("source cdbin.sh"), "source cdbin.sh\n")
    check("pwd after source", s.run("pwd"), "pwd\n/bin\n")
    check("cd back to /tests", s.run("cd /tests"), "cd /tests\n")
    check(". cdbin.sh is the same as source", s.run(". cdbin.sh"), ". cdbin.sh\n")
    check("pwd after .", s.run("pwd"), "pwd\n/bin\n")
    check("cd back to /tests again", s.run("cd /tests"), "cd /tests\n")

    # --- sh FILE runs scoped, and needs no exec bit ---
    check("sh cdbin.sh scoped, no exec bit needed",
          (s.run("sh cdbin.sh"), s.run("pwd")), ("sh cdbin.sh\n", "pwd\n/tests\n"))

    # --- extra arguments are rejected on all three forms: no positional parameters yet ---
    check("source with an extra argument", s.run("source cdbin.sh x"),
          "source cdbin.sh x\nsource: too many arguments\n")
    check("sh with an extra argument", s.run("sh cdbin.sh x"), "sh cdbin.sh x\nsh: too many arguments\n")
    check("./ with an extra argument", s.run("./cdbin.sh x"), "./cdbin.sh x\n./cdbin.sh: too many arguments\n")
    check("none of that moved the shell", s.run("pwd"), "pwd\n/tests\n")

    # --- nested scripts: inner's cd doesn't leak into outer, outer's doesn't leak past its pop ---
    check("chmod +x outer.sh", s.run("chmod +x outer.sh"), "chmod +x outer.sh\n")
    check("chmod +x inner.sh", s.run("chmod +x inner.sh"), "chmod +x inner.sh\n")
    check("./outer.sh: inner prints /bin, outer prints /fonts",
          s.run("./outer.sh"), "./outer.sh\n/bin\n/fonts\n")
    check("shell's own cwd is unaffected by either", s.run("pwd"), "pwd\n/tests\n")

    # --- a redirect on the invoking line covers every line the script runs, scope-wide ---
    check("chmod +x redir.sh", s.run("chmod +x redir.sh"), "chmod +x redir.sh\n")
    check("./redir.sh > out.txt prints nothing itself", s.run("./redir.sh > out.txt"), "./redir.sh > out.txt\n")
    check("out.txt has both lines' output", s.run("cat out.txt"), "cat out.txt\none\ntwo\n")
    check("the redirect is gone once the script's scope pops",
          s.run("echo after"), "echo after\nafter\n")

    # --- a failing line reports and the script continues; a leading #! is just a comment ---
    check("chmod +x bad.sh", s.run("chmod +x bad.sh"), "chmod +x bad.sh\n")
    check("./bad.sh continues past its failing line",
          s.run("./bad.sh"), "./bad.sh\nbefore\nnosuchcommand: command not found\nafter\n")

    # --- recursion depth cap: stops cleanly, no hang, no kernel-stack overflow ---
    check("source recur.sh stops at the depth cap",
          s.run("source recur.sh"), "source recur.sh\nsource: /tests/recur.sh: too many levels of scripts\n")
    check("the shell is alive after that", s.run("echo alive"), "echo alive\nalive\n")

    check("back to /", s.run("cd /"), "cd /\n")
