"""Last updated: Stage 12, Step 9.

Launching by path, files that aren't programs, the syscall error values, the fd limit, and how
`argv` is laid out on the new program's stack.

Test programs and fixtures live under `/tests/` (see `../README.md`); the kernel marks only `bin/`
executable at boot, so this module starts by giving the ones it needs the exec bit with `chmod +x`
(which also exercises `chmod` on paths).
"""

MALFORMED_ELFS = [
    "elf-trunc",      # cut off inside the program header table
    "elf-badseg",     # a segment far outside the user window
    "elf-badphoff",   # program header table offset nowhere near the file
    "elf-noload",     # no loadable segment at all
    "elf-badentry",   # entry point in no segment
    "elf-badoffset",  # segment bytes beyond the end of the file
    "elf-memlt",      # p_memsz < p_filesz
    "elf-toolargefile",  # a valid program in a file bigger than half the kernel heap
]

# `notes.txt` (plain text) isn't here: since Step 9, a non-ELF exec-bit file whose first bytes look
# like text is bash's `ENOEXEC` fallback -- run as a script -- not an error; see scripts.py for that.
# `data.bin` has a NUL early on, so it's still refused outright, just with Step 9's more specific
# wording ("binary file").
NOT_PROGRAMS = ["tests/data.bin"]


def cannot_execute(path):
    """`MALFORMED_ELFS`' wording: these do have ELF magic (they're broken further in), so `launch`
    never treats them as the Step 9 `ENOEXEC` fallback -- `process::run_program` itself refuses them."""
    return f"{path}\n{path}: cannot execute: Exec format error\n"


def cannot_execute_binary(path):
    """`NOT_PROGRAMS`' wording: no ELF magic at all, and not text either -- Step 9's `ENOEXEC`
    fallback's own "genuinely not a script" case, bash's wording."""
    return f"{path}\n{path}: cannot execute binary file: Exec format error\n"


def run(ctx):
    s, check = ctx.s, ctx.check

    # --- the exec bit is enforced for a path, as for a bare name ---
    check("path without the exec bit", s.run("tests/probe"),
          "tests/probe\ntests/probe: Permission denied\n")

    for name in ["probe", "bigpad"] + MALFORMED_ELFS:
        s.run(f"chmod +x tests/{name}")
    for path in NOT_PROGRAMS:
        s.run(f"chmod +x {path}")

    # --- lookup errors ---
    check("bare name unchanged", s.run("nosuch"), "nosuch\nnosuch: command not found\n")
    check("path: missing file", s.run("tests/nosuch"),
          "tests/nosuch\ntests/nosuch: No such file or directory\n")
    check("path: missing directory", s.run("nosuch/x"), "nosuch/x\nnosuch/x: No such file or directory\n")
    check("path: through a file", s.run("tests/notes.txt/x"),
          "tests/notes.txt/x\ntests/notes.txt/x: Not a directory\n")
    check("path: a directory", s.run("tests/docs"), "tests/docs\ntests/docs: Is a directory\n")
    check("absolute path", s.run("/bin/echo absolute"), "/bin/echo absolute\nabsolute\n")

    # --- files that are not programs: refused, never a panic ---
    for path in NOT_PROGRAMS:
        check(f"not a program: {path}", s.run(path), cannot_execute_binary(path))
    for name in MALFORMED_ELFS:
        path = f"tests/{name}"
        check(f"malformed ELF: {name}", s.run(path), cannot_execute(path))
    check("shell still alive after all that", s.run("echo alive"), "echo alive\nalive\n")

    # --- an executable far bigger than the old 1 MiB kernel heap (it is read whole into memory) ---
    check("3 MiB executable runs", s.run("tests/bigpad"), "tests/bigpad\nhello from userspace\n")

    # --- syscall error values ---
    check("probe without a subcommand", s.run_status("tests/probe"), ("tests/probe\nusage: probe sys-unknown|bad-ptr|fds|close-out|brk|clock|leak-write|reboot-wide|getdents-small|args|exit|poke|poke-w|user-ptrs|ioctl|getcwd|sp|stack|frag|frag-raw|bs-wide|interleave ...\n", 2))
    check("unknown syscall is ENOSYS", s.run("tests/probe sys-unknown"),
          "tests/probe sys-unknown\nunknown syscall: -38\n")
    check("ioctl on a closed fd is EBADF", s.run("tests/probe ioctl 3 1"),
          "tests/probe ioctl 3 1\nioctl(3, 1): -9\n")
    check("ioctl on something that is not the console is ENOTTY", s.run("tests/probe ioctl 0 1"),
          "tests/probe ioctl 0 1\nioctl(0, 1): -25\n")
    check("an ioctl request the console doesn't know is ENOTTY", s.run("tests/probe ioctl 1 999"),
          "tests/probe ioctl 1 999\nioctl(1, 999): -25\n")
    check("bad pointers are EFAULT", s.run("tests/probe bad-ptr"),
          "tests/probe bad-ptr\n"
          "write, pointer in kernel memory: -14\n"
          "write, pointer at the top of the address space: -14\n"
          "write, length larger than the window: -14\n"
          "read, bad pointer: -14\n"
          "open, bad pointer: -14\n"
          "open, path is not UTF-8: -22\n"
          "chmod, bad pointer: -14\n"
          "getdents on a closed fd: -9\n")

    # --- exit status: passed back from the exit syscall to the launcher, masked to 8 bits ---
    check("exit status 7", s.run_status("tests/probe exit 7"), ("tests/probe exit 7\n", 7))
    check("exit status 255", s.run_status("tests/probe exit 255"), ("tests/probe exit 255\n", 255))
    check("exit status is masked to 8 bits (300 -> 44)", s.run_status("tests/probe exit 300"), ("tests/probe exit 300\n", 44))
    check("exit status 256 masks to 0: success", s.run_status("tests/probe exit 256"), ("tests/probe exit 256\n", 0))
    check("exit status 0 after a nonzero one is not stale", s.run_status("true"), ("true\n", 0))

    # --- the fd limit: 16 slots, 3 standard, so exactly 13 opens, and closing frees them ---
    check("fd limit is exactly 13 opens", s.run("tests/probe fds"),
          "tests/probe fds\nopened 13, then -24\nafter closing all: open ok\n")

    # --- getdents: a buffer that cannot hold one record is an error, not an empty listing ---
    check("getdents with a buffer under one record is EINVAL; a file is still ENOTDIR",
          s.run("tests/probe getdents-small"),
          "tests/probe getdents-small\n"
          "empty buffer: -22\n"
          "one byte short: -22\n"
          "on a file, too small: -20\n"
          "exactly one record: 261\n")

    # --- a file left open when the program ends is closed, and its size committed, by the kernel ---
    check("a program that exits without closing its file", s.run("tests/probe leak-write tests/leaked.txt"),
          "tests/probe leak-write tests/leaked.txt\n")
    check("...still has the written size", s.run("cat tests/leaked.txt"), "cat tests/leaked.txt\nwritten\n")
    crashed = s.run("tests/probe leak-write tests/crashed.txt crash")
    check("a program that faults with a file open is stopped", "Segmentation fault" in crashed, True)
    check("...and its file was still committed", s.run("cat tests/crashed.txt"), "cat tests/crashed.txt\nwritten\n")

    # --- reboot's command is the whole register, not its low 32 bits ---
    check("reboot with a wide command is EINVAL and does not power off",
          s.run("tests/probe reboot-wide"),
          "tests/probe reboot-wide\nreboot(0x14321fedc): -22\n")

    # --- argv layout ---
    check("argv, with an empty argument", s.run('tests/probe args a "" b'),
          'tests/probe args a "" b\n'
          "argc=5\n"
          'argv[0]="tests/probe"\n'
          'argv[1]="args"\n'
          'argv[2]="a"\n'
          'argv[3]=""\n'
          'argv[4]="b"\n'
          "argv[argc] is NULL: yes\n"
          "argv is 16-byte aligned: yes\n"
          "sp is 16-byte aligned: yes\n")
    many = [f"a{i}" for i in range(1, 31)]
    out = s.run("tests/probe args " + " ".join(many))
    lines = out.splitlines()
    check("argv, 30 arguments: argc", lines[1], "argc=32")
    check("argv, 30 arguments: last", lines[1 + 32], 'argv[31]="a30"')
    check("argv, 30 arguments: terminator and alignment", lines[-3:],
          ["argv[argc] is NULL: yes", "argv is 16-byte aligned: yes", "sp is 16-byte aligned: yes"])
