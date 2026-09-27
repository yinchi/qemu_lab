"""Last updated: Stage 16, the audit.

Programs that used a fixed-size stand-in because EL0 had no heap now use one (the Stage 16 tier, `user/progs_r16`):

- `ls` lists a directory sorted by name, bytewise (it needed the whole listing in memory to sort it);
- `tail` reading stdin keeps only the last N lines or bytes, however much input goes by (it buffered the whole of stdin in
  a fixed 512 KiB and refused more);
- `tee` writes to as many files as the kernel lets it open (it held at most 8 in a fixed array);
- `chmod -R` and `rm -r` list each directory whole and close it before recursing, so how deep a tree can be is not limited by
  the kernel's open-file limit (`chmod -R` kept every ancestor directory open while descending);
- `cp`, `mv`, `rm`, `chmod` build paths in `String`s, with no `PATH_MAX` buffer of their own.
"""

BIGPAD_MIN = 3 * 1024 * 1024  # the fixture is a valid program plus 3 MiB of zeros


def run(ctx):
    s, check = ctx.s, ctx.check

    # --- ls: sorted by name, bytewise (so `Mid` comes before `alpha`), whatever order the entries were made in ---
    s.run("mkdir tests/sortdir")
    for name in ["zeta", "alpha", "Mid", "beta"]:
        s.run(f"echo {name} > tests/sortdir/{name}")
    s.run("mkdir tests/sortdir/gamma")
    check("ls sorts by name, bytewise", s.run("ls tests/sortdir"), "ls tests/sortdir\nMid\nalpha\nbeta\ngamma\nzeta\n")
    check("ls -F sorts the same", s.run("ls -F tests/sortdir"), "ls -F tests/sortdir\nMid\nalpha\nbeta\ngamma/\nzeta\n")
    long = s.run("ls -l tests/sortdir").split("\n")[1:-1]
    check("ls -l too", [line.split()[-1] for line in long], ["Mid", "alpha", "beta", "gamma", "zeta"])

    # --- tail on stdin: past the old 512 KiB buffer, keeping only what it could still print ---
    check("tail -n 3 of 100000 lines (1 MB) on stdin", s.run("tail -n 3 < tests/biglines.txt"),
          "tail -n 3 < tests/biglines.txt\nline 99998\nline 99999\nline 100000\n")
    check("tail -c 10 of the same", s.run("tail -c 10 < tests/biglines.txt"), "tail -c 10 < tests/biglines.txt\nne 100000\n")
    check("tail -n 0 prints nothing", s.run("tail -n 0 < tests/biglines.txt"), "tail -n 0 < tests/biglines.txt\n")
    check("tail -n 100000 keeps every line", s.run("tail -n 100000 < tests/biglines.txt | wc -l"),
          "tail -n 100000 < tests/biglines.txt | wc -l\n100000\n")
    check("tail -n 200000 (more than the input) too", s.run("tail -n 200000 < tests/biglines.txt | wc -l"),
          "tail -n 200000 < tests/biglines.txt | wc -l\n100000\n")
    check("tail -c 4 of a 3 MiB binary on stdin", s.run("tail -c 4 < tests/bigpad | wc -c"),
          "tail -c 4 < tests/bigpad | wc -c\n4\n")
    whole = int(s.run("wc -c < tests/bigpad").split("\n")[1])
    check("tail -c 10000000 of it returns all of it", s.run("tail -c 10000000 < tests/bigpad | wc -c"),
          f"tail -c 10000000 < tests/bigpad | wc -c\n{whole}\n")
    check("...which is past the old 512 KiB limit", whole > 512 * 1024 and whole >= BIGPAD_MIN, True)

    # --- tee: not capped at 8 files, only by the kernel's open-file limit (13 files at a time) ---
    names = " ".join(f"tests/tee{i}" for i in range(1, 11))
    s.run(f"echo ten | tee {names}")
    check("tee writes ten files (the old cap was eight)", s.run("cat tests/tee9 tests/tee10"), "cat tests/tee9 tests/tee10\nten\nten\n")
    # In a pipeline the shell holds the pipe's temp file open for `tee`'s stdin, one of the kernel's 13 open files, so 12 remain.
    names = " ".join(f"tests/tf{i}" for i in range(1, 15))
    check("tee with 14 files: the kernel's limit reports the ones that do not open, and the rest are written",
          s.run_status(f"echo many | tee {names}"),
          (f"echo many | tee {names}\ntee: tests/tf13: Too many open files\n"
           "tee: tests/tf14: Too many open files\nmany\n", 1))
    check("...the twelfth was written", s.run("cat tests/tf12"), "cat tests/tf12\nmany\n")
    check("...the thirteenth was not", s.run_status("cat tests/tf13"), ("cat tests/tf13\ncat: tests/tf13: No such file or directory\n", 1))

    # --- chmod -R and rm -r on a tree deeper than the open-file limit ---
    path = "tests/deep"
    s.run(f"mkdir {path}")
    for level in range(1, 15):
        path += f"/d{level}"
        s.run(f"mkdir {path}")
    s.run(f"echo leaf > {path}/leaf.txt")
    check("chmod -R reaches a tree 14 levels deep", s.run("chmod -w -R tests/deep"), "chmod -w -R tests/deep\n")
    parent = "tests/deep" + "".join(f"/d{level}" for level in range(1, 14))
    check("...it changed the deepest directory and its file", [line.split()[0] for line in s.run(f"ls -l {parent}").split("\n")[1:-1]],
          ["d--"])
    check("...and the file at the bottom", s.run(f"ls -l {path}").split("\n")[1].split()[0], "---")
    check("rm -r removes it", s.run("rm -r tests/deep"), "rm -r tests/deep\n")
    check("...all of it", s.run_status("ls tests/deep"), ("ls tests/deep\nls: cannot access 'tests/deep': No such file or directory\n", 1))

    check("shell alive", s.run("echo ok"), "echo ok\nok\n")
