"""Last updated: Stage 12, Step 6.

A working directory. `cd` is a builtin, `pwd` a program over the `getcwd` syscall, and every relative
path -- a program's `open`/`chmod` and a command word containing `/` -- is resolved against it. A
bare command name is still looked up in `/bin`, wherever the shell is.

Every case ends back at `/`, since the working directory is shell state that later cases inherit.
"""

NOT_FOUND = "No such file or directory"


def run(ctx):
    s, check = ctx.s, ctx.check
    s.run("chmod +x tests/probe.exe")  # self-sufficient: this module must not depend on an earlier
    # one (possibly in a different parallel group/session) having already done this.
    hello_txt = ctx.fixture("hello.txt")

    def cd(path, expect_error=None):
        """Runs `cd path`; a successful `cd` prints nothing."""
        want = f"cd {path}\n" + (f"cd: {expect_error}\n" if expect_error else "")
        return s.run(f"cd {path}"), want

    # --- T6.1: moving around ---
    check("pwd starts at the root", s.run("pwd"), "pwd\n/\n")
    check("cd bin", *cd("bin"))
    check("pwd after cd bin", s.run("pwd"), "pwd\n/bin\n")
    check("cd ..", *cd(".."))
    check("back at the root", s.run("pwd"), "pwd\n/\n")
    check("cd .. at the root stays put", *cd(".."))
    check("still the root", s.run("pwd"), "pwd\n/\n")
    check("cd with . and .. mixed", *cd("/bin/../fonts"))
    check("pwd is normalized", s.run("pwd"), "pwd\n/fonts\n")
    check("cd /", *cd("/"))
    check("cd ./bin/.", *cd("./bin/."))
    check("pwd after cd ./bin/.", s.run("pwd"), "pwd\n/bin\n")
    check("cd with no operand goes to the root", *cd(""))
    check("pwd after a bare cd", s.run("pwd"), "pwd\n/\n")
    check("cd -- dir", *cd("-- tests"))
    check("pwd after cd -- tests", s.run("pwd"), "pwd\n/tests\n")
    check("cd ../bin from /tests", *cd("../bin"))

    # --- T6.2: errors leave the working directory alone ---
    check("cd to a missing directory", *cd("nosuch", f"nosuch: {NOT_FOUND}"))
    check("cd to a file", *cd("/tests/notes.txt", "/tests/notes.txt: Not a directory"))
    check("cd through a file", *cd("/tests/notes.txt/x", "/tests/notes.txt/x: Not a directory"))
    check("cd with two operands", *cd("a b", "too many arguments"))
    check("cd -", *cd("-", "-: not supported (there is no $OLDPWD)"))
    check("cd -L", *cd("-L", "-L: not supported (there are no symbolic links)"))
    check("cd -x", *cd("-x", "-x: invalid option"))
    check("the working directory survived all of that", s.run("pwd"), "pwd\n/bin\n")
    check("cd with a name that is too long", *cd("x" * 256, "x" * 256 + ": File name too long"))

    # --- T6.3: relative paths are relative to the working directory ---
    check("a bare command name is found in /bin from anywhere", (s.run("cd /tests"), s.run("echo hi")),
          ("cd /tests\n", "echo hi\nhi\n"))
    check("a bare name is not looked up in the working directory", s.run("probe.exe"),
          "probe.exe\nprobe.exe: command not found\n")
    check("cat opens a relative path", s.run("cat hello.txt"), "cat hello.txt\n" + hello_txt)
    check("...and a path with ..", s.run("cat ../tests/hello.txt"), "cat ../tests/hello.txt\n" + hello_txt)
    check("cp creates its output relative to the working directory",
          s.run("cp hello.txt cwdtest.txt"), "cp hello.txt cwdtest.txt\n")
    check("chmod honors the working directory", s.run("chmod +x cwdtest.txt"), "chmod +x cwdtest.txt\n")
    check("...on that file", "cwdtest.txt*" in s.run("ls -F"), True)
    check("...and it is the one in /tests", "cwdtest.txt*" in s.run("ls -F /tests"), True)
    check("a program is launched by relative path", s.run("./probe.exe ioctl 3 1"),
          "./probe.exe ioctl 3 1\nioctl(3, 1): -9\n")
    check("...and by a path with ..", s.run("../tests/probe.exe ioctl 3 1"),
          "../tests/probe.exe ioctl 3 1\nioctl(3, 1): -9\n")
    check("ls with no operand lists the working directory", "cwdtest.txt*" in s.run("ls -F"), True)

    # --- T6.4: getcwd's buffer is honored ---
    check("getcwd with room", s.run("./probe.exe getcwd 100"), "./probe.exe getcwd 100\ngetcwd(100): 6 /tests\n")
    check("getcwd with exactly enough", s.run("./probe.exe getcwd 6"), "./probe.exe getcwd 6\ngetcwd(6): 6 /tests\n")
    check("getcwd with too little is ERANGE", s.run("./probe.exe getcwd 5"), "./probe.exe getcwd 5\ngetcwd(5): -34\n")
    check("getcwd with nothing is ERANGE", s.run("./probe.exe getcwd 0"), "./probe.exe getcwd 0\ngetcwd(0): -34\n")

    # --- pwd's own options ---
    check("pwd -L is refused", s.run("pwd -L"),
          "pwd -L\npwd: -L: not supported (there are no symbolic links)\nexit 1\n")
    check("pwd -x is refused", s.run("pwd -x"), "pwd -x\npwd: invalid option -- 'x'\nTry 'pwd --help' for more information.\nexit 1\n")
    check("pwd with an operand is refused", s.run("pwd a"), "pwd a\npwd: extra operand 'a'\nTry 'pwd --help' for more information.\nexit 1\n")

    check("cd back to the root", *cd("/"))
    check("pwd at the end", s.run("pwd"), "pwd\n/\n")
