"""Last updated: Stage 17, Step 8.

`$PATH`: the directories a bare command name is searched in, in order. Unset means
`/bin`; set but empty means nowhere; empty entries are skipped (not the working directory); relative entries are
relative to the working directory; a name with a `/` in it is a path and ignores `PATH`. The candidate list is a host
test (`path_search.rs`); here it is the shell end to end.

Two directories of programs are made under `/tmp`: `pbin` holds `tool` (a copy of `hello`, so it prints a fixed
line) and `pbin2` holds `tool` (a copy of `echo`, so it prints its arguments) and `dotted.exe`, another `echo` -- so
what ran says which directory it came from, and the last shows that a name means exactly the file of that name (there is
no `.exe` fallback since Stage 17).
"""

ENVIRONMENT = "HOME=/\nPATH=/bin\n"  # like the general image's file

HELLO = "hello from userspace\n"


def run(ctx):
    s, check = ctx.s, ctx.check

    def prints(line, out):
        check(line, s.run(line), f"{line}\n{out}")

    def not_found(line, name=None, status=127):
        name = name or line.split()[-1].split("=")[-1]
        check(line, s.run_status(line), (f"{line}\n{name}: command not found\n", status))

    def not_found_here(name):
        """`name` is not found, with the shell's own PATH empty -- so the status cannot be read back with `echo $?`
        (`echo` is a program too); the message is what shows it."""
        check(name, s.run(name), f"{name}\n{name}: command not found\n")

    for cmd in ["mkdir /tmp/pbin", "mkdir /tmp/pbin2",
                "cp /bin/hello /tmp/pbin/tool", "cp /bin/echo /tmp/pbin2/tool",
                "cp /bin/echo /tmp/pbin2/dotted.exe", "chmod +x /tmp/pbin/tool /tmp/pbin2/tool /tmp/pbin2/dotted.exe"]:
        s.run(cmd)

    # --- the starting point: PATH=/bin ---
    prints("hello", HELLO)
    not_found("tool")
    check("PATH is the shell's and exported", s.run("printenv PATH"), "printenv PATH\n/bin\n")

    # --- a prefix for one command ---
    prints("PATH=/tmp/pbin tool", HELLO)
    not_found("tool")  # ...and gone again
    prints("PATH=/tmp/pbin:/bin hello", HELLO)  # a later directory still searched
    check("PATH is in the command's environment", s.run("PATH=/tmp/pbin:/bin printenv PATH"),
          "PATH=/tmp/pbin:/bin printenv PATH\n/tmp/pbin:/bin\n")

    # --- export, and appending with $PATH ---
    s.run("PATH=$PATH:/tmp/pbin")
    check("appended", s.run("printenv PATH"), "printenv PATH\n/bin:/tmp/pbin\n")
    prints("tool", HELLO)
    prints("hello", HELLO)  # /bin is still on the list
    s.run("PATH=/tmp/pbin:$PATH")
    prints("tool", HELLO)  # a directory earlier in the list wins over a later one
    check("...and the name resolution reaches /bin last", s.run("printenv PATH"), "printenv PATH\n/tmp/pbin:/bin:/tmp/pbin\n")

    # --- precedence between two directories that both have `tool` ---
    prints("PATH=/tmp/pbin:/tmp/pbin2 tool x", HELLO)
    prints("PATH=/tmp/pbin2:/tmp/pbin tool x", "x\n")
    # A name is exact: no `.exe` is added, and none is taken away.
    prints("PATH=/tmp/pbin2 dotted.exe z", "z\n")
    not_found("PATH=/tmp/pbin2 dotted")
    not_found("PATH=/bin hello.exe")
    not_found("PATH=/bin cat.exe")
    s.run("PATH=/bin")

    # --- a directory of that name is skipped, and a file without the exec bit is refused ---
    s.run("mkdir /tmp/pbin/hello")
    prints("PATH=/tmp/pbin:/bin hello", HELLO)
    s.run("cp /bin/hello /tmp/pbin/noexec")
    s.run("chmod -x /tmp/pbin/noexec")
    check("a file found without the exec bit", s.run_status("PATH=/tmp/pbin noexec"),
          ("PATH=/tmp/pbin noexec\nnoexec: Permission denied\n", 126))

    # --- empty: nowhere. unset: /bin. empty entries: skipped ---
    not_found("PATH= hello")
    s.run("export PATH=")
    not_found_here("hello")
    check("...but a path still works", s.run("/bin/hello"), "/bin/hello\n" + HELLO)
    check("...and a relative path", s.run("bin/hello"), "bin/hello\n" + HELLO)
    s.run("unset PATH")
    prints("hello", HELLO)  # unset is /bin
    s.run("export PATH=:")
    not_found_here("hello")
    s.run("export PATH=:/bin:")
    prints("hello", HELLO)
    s.run("cd /tmp/pbin")
    not_found("PATH=: tool")  # an empty entry is not the working directory
    not_found("PATH=/bin: tool")
    prints("PATH=.:/bin tool", HELLO)  # but `.` is
    s.run("cd /")

    # --- relative entries are relative to the working directory ---
    s.run("cd /tmp")
    prints("PATH=pbin tool", HELLO)
    prints("PATH=./pbin2:pbin tool x", "x\n")
    s.run("cd /")
    not_found("PATH=pbin tool")
    s.run("cd /tmp/pbin2")
    prints("PATH=../pbin tool", HELLO)
    s.run("cd /")

    # --- a name with a slash ignores PATH ---
    prints("PATH=/nonexistent /bin/hello", HELLO)
    prints("PATH=/nonexistent bin/hello", HELLO)
    prints("PATH=/nonexistent /tmp/pbin/tool", HELLO)
    not_found("PATH=/bin nosuch", status=127)
    check("a missing path is 127 too", s.run_status("PATH=/bin tmp/nosuch"),
          ("PATH=/bin tmp/nosuch\ntmp/nosuch: No such file or directory\n", 127))

    # --- the command word can itself come from a variable ---
    s.run("export PATH=/tmp/pbin")
    s.run("T=tool")
    prints("$T", HELLO)
    s.run("export PATH=/bin")
    check("the shell is alive", s.run("echo ok"), "echo ok\nok\n")
