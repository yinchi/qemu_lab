"""Step 1 of `Stage12.md`: launching by path, files that aren't programs, the syscall error values, the
fd limit, and how `argv` is laid out on the new program's stack.

Test programs and fixtures live under `/tests/` (see `../README.md`); the kernel marks only `bin/`
executable at boot, so this module starts by giving the ones it needs the exec bit with `chmod +x`
(which also exercises `chmod` on paths).
"""

MALFORMED_ELFS = [
    "elf-trunc.exe",      # cut off inside the program header table
    "elf-badseg.exe",     # a segment far outside the user window
    "elf-badphoff.exe",   # program header table offset nowhere near the file
    "elf-noload.exe",     # no loadable segment at all
    "elf-badentry.exe",   # entry point in no segment
    "elf-badoffset.exe",  # segment bytes beyond the end of the file
    "elf-memlt.exe",      # p_memsz < p_filesz
]

NOT_PROGRAMS = ["tests/notes.txt", "tests/data.bin"]


def cannot_execute(path):
    return f"{path}\n{path}: cannot execute: Exec format error\n"


def run(ctx):
    s, check = ctx.s, ctx.check

    # --- the exec bit is enforced for a path, as for a bare name ---
    check("path without the exec bit", s.run("tests/probe.exe"),
          "tests/probe.exe\ntests/probe.exe: not executable\n")

    for name in ["probe.exe", "bigpad.exe"] + MALFORMED_ELFS:
        s.run(f"chmod +x tests/{name}")
    for path in NOT_PROGRAMS:
        s.run(f"chmod +x {path}")

    # --- lookup errors ---
    check("bare name unchanged", s.run("nosuch"), "nosuch\nnosuch: not found\n")
    check("path: missing file", s.run("tests/nosuch.exe"),
          "tests/nosuch.exe\ntests/nosuch.exe: No such file or directory\n")
    check("path: missing directory", s.run("nosuch/x"), "nosuch/x\nnosuch/x: No such file or directory\n")
    check("path: through a file", s.run("tests/notes.txt/x"),
          "tests/notes.txt/x\ntests/notes.txt/x: Not a directory\n")
    check("path: a directory", s.run("tests/docs"), "tests/docs\ntests/docs: Is a directory\n")
    check("absolute path", s.run("/bin/echo.exe absolute"), "/bin/echo.exe absolute\nabsolute\n")

    # --- files that are not programs: refused, never a panic ---
    for path in NOT_PROGRAMS:
        check(f"not a program: {path}", s.run(path), cannot_execute(path))
    for name in MALFORMED_ELFS:
        path = f"tests/{name}"
        check(f"malformed ELF: {name}", s.run(path), cannot_execute(path))
    check("shell still alive after all that", s.run("echo alive"), "echo alive\nalive\n")

    # --- an executable far bigger than the old 1 MiB kernel heap (it is read whole into memory) ---
    check("3 MiB executable runs", s.run("tests/bigpad.exe"), "tests/bigpad.exe\nhello from userspace\n")

    # --- syscall error values ---
    check("probe without a subcommand", s.run("tests/probe.exe"),
          "tests/probe.exe\nusage: probe sys-unknown|bad-ptr|fds|args|exit|poke|poke-w|user-ptrs|ioctl|getcwd|sp|stack|frag|frag-raw|bs-wide|interleave ...\nexit 2\n")
    check("unknown syscall is ENOSYS", s.run("tests/probe.exe sys-unknown"),
          "tests/probe.exe sys-unknown\nunknown syscall: -38\n")
    check("ioctl on a closed fd is EBADF", s.run("tests/probe.exe ioctl 3 1"),
          "tests/probe.exe ioctl 3 1\nioctl(3, 1): -9\n")
    check("ioctl on something that is not the console is ENOTTY", s.run("tests/probe.exe ioctl 0 1"),
          "tests/probe.exe ioctl 0 1\nioctl(0, 1): -25\n")
    check("an ioctl request the console doesn't know is ENOTTY", s.run("tests/probe.exe ioctl 1 999"),
          "tests/probe.exe ioctl 1 999\nioctl(1, 999): -25\n")
    check("bad pointers are EFAULT", s.run("tests/probe.exe bad-ptr"),
          "tests/probe.exe bad-ptr\n"
          "write, pointer in kernel memory: -14\n"
          "write, pointer at the top of the address space: -14\n"
          "write, length larger than the window: -14\n"
          "read, bad pointer: -14\n"
          "open, bad pointer: -14\n"
          "open, path is not UTF-8: -22\n"
          "chmod, bad pointer: -14\n"
          "getdents on a closed fd: -9\n")

    # --- exit status: passed back from the exit syscall to the launcher, masked to 8 bits ---
    check("exit status 7", s.run("tests/probe.exe exit 7"), "tests/probe.exe exit 7\nexit 7\n")
    check("exit status 255", s.run("tests/probe.exe exit 255"), "tests/probe.exe exit 255\nexit 255\n")
    check("exit status is masked to 8 bits (300 -> 44)", s.run("tests/probe.exe exit 300"),
          "tests/probe.exe exit 300\nexit 44\n")
    check("exit status 256 masks to 0: success, nothing reported", s.run("tests/probe.exe exit 256"),
          "tests/probe.exe exit 256\n")
    check("exit status 0 after a nonzero one is not stale", s.run("true"), "true\n")

    # --- the fd limit: 16 slots, 3 standard, so exactly 13 opens, and closing frees them ---
    check("fd limit is exactly 13 opens", s.run("tests/probe.exe fds"),
          "tests/probe.exe fds\nopened 13, then -24\nafter closing all: open ok\n")

    # --- argv layout ---
    check("argv, with an empty argument", s.run('tests/probe.exe args a "" b'),
          'tests/probe.exe args a "" b\n'
          "argc=5\n"
          'argv[0]="tests/probe.exe"\n'
          'argv[1]="args"\n'
          'argv[2]="a"\n'
          'argv[3]=""\n'
          'argv[4]="b"\n'
          "argv[argc] is NULL: yes\n"
          "argv is 16-byte aligned: yes\n"
          "sp is 16-byte aligned: yes\n")
    many = [f"a{i}" for i in range(1, 31)]
    out = s.run("tests/probe.exe args " + " ".join(many))
    lines = out.splitlines()
    check("argv, 30 arguments: argc", lines[1], "argc=32")
    check("argv, 30 arguments: last", lines[1 + 32], 'argv[31]="a30"')
    check("argv, 30 arguments: terminator and alignment", lines[-3:],
          ["argv[argc] is NULL: yes", "argv is 16-byte aligned: yes", "sp is 16-byte aligned: yes"])
