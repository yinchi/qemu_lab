"""Last updated: Stage 19, Step 4b.

Setting times: the `utimensat` syscall (through `probe utimens`), `touch -d`/`-t`/`-r` and `cp -p`. The zone is
`America/Toronto` (so a summer time shows `EDT` and a winter one `EST`, and `TZ=UTC` in front of a command changes how it
reads a wall-clock time); the kernel stores UTC and `stat` converts for display.

FAT keeps the modify time to 2 seconds, holds 1980 to 2107, and cannot set a creation time (nor is the access time set: FAT has only
an access *date*, so the syscall accepts an access time for Linux's shape and ignores it):
each of those is a check below, as are the ways a time can be given (`@N`, a date, a date and clock, `T`, `Z`, POSIX's `-t`), the
ones GNU accepts that this refuses, a daylight-saving gap and an ambiguous hour, `-r`, that `-d`, `-t` and `-r` exclude one
another, a directory, and that nothing about a file's contents changes. `cp -p` keeps the modify time and the executable and
read-only bits for a file, a tree (each directory's own time too) and onto another volume, and a plain `cp` does not.
"""

import subprocess

ENVIRONMENT = "HOME=/\nTZ=America/Toronto\n"
EXTRA_DISKS = [{"label": "HOME", "volume_id": "5E6F7A8B", "size_kib": 2048}]

EINVAL, ENOENT, EFAULT = -22, -2, -14


def run(ctx):
    s, check = ctx.s, ctx.check

    def modify(path):
        """The `Modify:` line of `stat`, without its label."""
        for line in s.run(f"stat {path}").split("\n"):
            if line.startswith("Modify: "):
                return line[len("Modify: "):]
        return None

    def create(path):
        for line in s.run(f"stat {path}").split("\n"):
            if line.startswith("Create: "):
                return line[len("Create: "):]

    def probe(args):
        out = s.run(f"/tests/probe {args}")
        return out.split("\n")[1]

    def status(cmd, want_out, want_status):
        check(cmd, s.run_status(cmd), (f"{cmd}\n{want_out}", want_status))

    s.run("chmod +x /tests/probe")
    s.run("mkdir /w")
    s.run("cd /w")
    s.run("echo data > f")
    created = create("f")

    # ================================================================= touch -d
    check("-d a date and a clock (summer: EDT)", (s.run("touch -d '2024-05-01 12:00' f"), modify("f")), ("touch -d '2024-05-01 12:00' f\n", "2024-05-01 12:00:00 EDT"))
    check("-d in winter: EST", (s.run("touch -d '2020-01-02 15:30' f"), modify("f")), ("touch -d '2020-01-02 15:30' f\n", "2020-01-02 15:30:00 EST"))
    check("-d with seconds", (s.run("touch -d '2024-05-01 12:00:30' f"), modify("f")), ("touch -d '2024-05-01 12:00:30' f\n", "2024-05-01 12:00:30 EDT"))
    check("...an odd second is dropped (FAT keeps 2 seconds)", (s.run("touch -d '2024-05-01 12:00:31' f"), modify("f")), ("touch -d '2024-05-01 12:00:31' f\n", "2024-05-01 12:00:30 EDT"))
    check("-d with T", (s.run("touch -d 2024-05-01T12:00:10 f"), modify("f")), ("touch -d 2024-05-01T12:00:10 f\n", "2024-05-01 12:00:10 EDT"))
    check("-d a bare date is midnight", (s.run("touch -d 2024-05-01 f"), modify("f")), ("touch -d 2024-05-01 f\n", "2024-05-01 00:00:00 EDT"))
    check("-d with Z is UTC", (s.run("touch -d 2001-09-09T01:46:40Z f"), modify("f")), ("touch -d 2001-09-09T01:46:40Z f\n", "2001-09-08 21:46:40 EDT"))
    check("-d @SECONDS", (s.run("touch -d @1000000000 f"), modify("f")), ("touch -d @1000000000 f\n", "2001-09-08 21:46:40 EDT"))
    check("--date=", (s.run("touch --date=2024-05-01 f"), modify("f")), ("touch --date=2024-05-01 f\n", "2024-05-01 00:00:00 EDT"))
    check("TZ=UTC reads the wall clock as UTC", (s.run("TZ=UTC touch -d '2024-05-01 12:00' f"), s.run("TZ=UTC stat f").split("\n")[4]),
          ("TZ=UTC touch -d '2024-05-01 12:00' f\n", "Modify: 2024-05-01 12:00:00 UTC"))
    check("the creation time never moves", create("f"), created)
    check("...and the data is untouched", s.run("cat f"), "cat f\ndata\n")

    # ================================================================= daylight saving
    status("touch -d '2024-03-10 02:30' f", "touch: invalid date format '2024-03-10 02:30'\n", 1)  # the hour that was skipped
    check("an ambiguous hour takes the earlier (EDT)", (s.run("touch -d '2024-11-03 01:30' f"), modify("f")), ("touch -d '2024-11-03 01:30' f\n", "2024-11-03 01:30:00 EDT"))

    # ================================================================= touch -t
    check("-t MMDDhhmm (this year)", "-05-01 13:45:00" in (s.run("touch -t 05011345 f") and modify("f")), True)
    check("-t CCYYMMDDhhmm", (s.run("touch -t 202405011345 f"), modify("f")), ("touch -t 202405011345 f\n", "2024-05-01 13:45:00 EDT"))
    check("-t with .ss", (s.run("touch -t 202405011345.20 f"), modify("f")), ("touch -t 202405011345.20 f\n", "2024-05-01 13:45:20 EDT"))
    check("-t YYMMDDhhmm: 00-68 are 20YY", (s.run("touch -t 2405011345 f"), modify("f")), ("touch -t 2405011345 f\n", "2024-05-01 13:45:00 EDT"))
    check("-t YYMMDDhhmm: 69-99 are 19YY (before 1980 here, so refused by FAT)", s.run_status("touch -t 7005011345 f"),
          ("touch -t 7005011345 f\ntouch: setting times of 'f': Invalid argument\n", 1))
    check("-t 80: 1980 is the first year FAT has", (s.run("touch -t 8001020000 f"), modify("f")), ("touch -t 8001020000 f\n", "1980-01-02 00:00:00 EST"))
    for bad in ("13", "0501134", "05011345.", "13011345", "abcdefgh"):
        status(f"touch -t {bad} f", f"touch: invalid date format '{bad}'\n", 1)

    # ================================================================= FAT's range
    status("touch -d 1970-01-01 f", "touch: setting times of 'f': Invalid argument\n", 1)
    status("touch -d 1979-12-31T23:59:58Z f", "touch: setting times of 'f': Invalid argument\n", 1)
    check("1980-01-01 00:00 UTC is the first moment", (s.run("touch -d 1980-01-01T00:00:00Z f"), modify("f")), ("touch -d 1980-01-01T00:00:00Z f\n", "1979-12-31 19:00:00 EST"))
    check("2107-12-31 23:58 is nearly the last", (s.run("touch -d 2107-12-31T23:58:00Z f"), modify("f")), ("touch -d 2107-12-31T23:58:00Z f\n", "2107-12-31 18:58:00 EST"))
    status("touch -d 2108-01-01T00:00:00Z f", "touch: setting times of 'f': Invalid argument\n", 1)
    check("a failed touch left the last good time", modify("f"), "2107-12-31 18:58:00 EST")

    # ================================================================= what -d does not read
    for bad in ("yesterday", "2024-5-1", "2024-02-30", "2024-05-01 25:00", "12:00", "", "@", "next week"):
        status(f"touch -d '{bad}' f", f"touch: invalid date format '{bad}'\n", 1)

    # ================================================================= -r, creating, -c, directories
    s.run("touch -d '2010-10-10 10:10:10' ref")
    check("-r takes the other file's modify time", (s.run("touch -r ref g"), modify("g")), ("touch -r ref g\n", "2010-10-10 10:10:10 EDT"))
    check("...and creates the file if it is missing", s.run("cat g"), "cat g\n")
    check("-d creates a missing file and sets its time", (s.run("touch -d 2015-06-06 h"), modify("h")), ("touch -d 2015-06-06 h\n", "2015-06-06 00:00:00 EDT"))
    check("-c with -d leaves a missing file missing", s.run_status("touch -c -d 2015-06-06 nothere"), ("touch -c -d 2015-06-06 nothere\n", 0))
    check("...", s.run_status("stat nothere")[1], 1)
    s.run("mkdir dd")
    check("a directory can be touched", (s.run("touch -d 2012-12-12 dd"), modify("dd")), ("touch -d 2012-12-12 dd\n", "2012-12-12 00:00:00 EST"))
    check("several files at once", (s.run("touch -d 2013-03-03 a1 a2"), modify("a1"), modify("a2")), ("touch -d 2013-03-03 a1 a2\n", "2013-03-03 00:00:00 EST", "2013-03-03 00:00:00 EST"))
    check("touch -d after the operands", (s.run("touch a3 -d 2014-04-04"), modify("a3")), ("touch a3 -d 2014-04-04\n", "2014-04-04 00:00:00 EDT"))
    check("-c and -d together in a group", (s.run("touch -cd 2016-06-06 a3"), modify("a3")), ("touch -cd 2016-06-06 a3\n", "2016-06-06 00:00:00 EDT"))

    # ================================================================= usage errors
    status("touch -d 2024-05-01 -t 202401011200 f", "touch: cannot specify times from more than one source\nTry 'touch --help' for more information.\n", 1)
    status("touch -d 2024-05-01 -r ref f", "touch: cannot specify times from more than one source\nTry 'touch --help' for more information.\n", 1)
    status("touch -r nosuch f", "touch: failed to get attributes of 'nosuch': No such file or directory\n", 1)
    status("touch -d", "touch: option requires an argument -- 'd'\nTry 'touch --help' for more information.\n", 1)
    status("touch -d 2024-05-01", "touch: missing file operand\nTry 'touch --help' for more information.\n", 1)
    status("touch -r ref", "touch: missing file operand\nTry 'touch --help' for more information.\n", 1)

    # ================================================================= the syscall itself
    s.run("touch -d 2000-01-01 p")
    old = modify("p")
    check("both omitted: a success that changes nothing", (probe("utimens p 0 omit 0 omit"), modify("p")), ("utimens: 0", old))
    check("an access time is ignored, even one FAT could not hold", (probe("utimens p 0 0 0 omit"), modify("p")), ("utimens: 0", old))
    check("modify only (access omitted)", (probe("utimens p 0 omit 946684800 0"), modify("p")), ("utimens: 0", "1999-12-31 19:00:00 EST"))
    check("modify omitted: nothing moves", (probe("utimens p 946684800 0 0 omit"), modify("p")), ("utimens: 0", "1999-12-31 19:00:00 EST"))
    check("a fraction of a second is accepted and dropped", (probe("utimens p 0 omit 978307200 999999999"), modify("p")), ("utimens: 0", "2000-12-31 19:00:00 EST"))
    check("a null array is now for both", probe("utimens-null p"), "utimens: 0")
    check("...the modify time is now: after 2020", int(modify("p")[:4]) >= 2026, True)
    check("a nanosecond field out of range", probe("utimens p 0 1000000000 0 0"), f"utimens: {EINVAL}")
    check("a negative nanosecond field", probe("utimens p 0 omit 0 -1"), f"utimens: {EINVAL}")
    check("a time before 1980", probe("utimens p 0 omit 0 0"), f"utimens: {EINVAL}")
    check("a missing file", probe("utimens nosuch 0 omit 946684800 0"), f"utimens: {ENOENT}")
    check("the root has no entry to hold a time", probe("utimens / 0 omit 946684800 0"), f"utimens: {EINVAL}")
    check("bad pointers", probe("utimens-bad p"), f"utimens: {EFAULT} {EFAULT}")
    check("a failed call changed nothing", int(modify("p")[:4]) >= 2026, True)
    check("access and modify both now: after the fixed date", probe("utimens p 0 now 0 now"), "utimens: 0")

    # ================================================================= cp -p
    s.run("echo body > src")
    s.run("touch -d '2011-11-11 11:11' src")
    s.run("chmod +x src")
    check("cp without -p: a new time, no exec bit", (s.run("cp src plain"), int(modify("plain")[:4]) >= 2026, "exec=no" in s.run("stat plain")),
          ("cp src plain\n", True, True))
    check("cp -p keeps the modify time", (s.run("cp -p src kept"), modify("kept")), ("cp -p src kept\n", "2011-11-11 11:11:00 EST"))
    check("...and the exec bit", "exec=yes" in s.run("stat kept"), True)
    check("...and the contents", s.run("cat kept"), "cat kept\nbody\n")
    s.run("chmod -x src")
    s.run("chmod -w src")
    check("cp -p keeps read-only", (s.run("cp -p src ro"), "read-only=yes" in s.run("stat ro"), "exec=no" in s.run("stat ro")), ("cp -p src ro\n", True, True))
    s.run("chmod +w src")
    check("cp -p onto an existing file (older, executable) takes the source's", (s.run("cp -p src kept"), modify("kept"), "exec=no" in s.run("stat kept")),
          ("cp -p src kept\n", "2011-11-11 11:11:00 EST", True))
    check("--preserve", (s.run("cp --preserve src kept2"), modify("kept2")), ("cp --preserve src kept2\n", "2011-11-11 11:11:00 EST"))
    check("cp -pn is accepted", s.run("cp -pn src kept2"), "cp -pn src kept2\n")

    # a tree: every file and every directory keeps its own time
    s.run("mkdir -p tree/sub")
    s.run("echo 1 > tree/a")
    s.run("echo 2 > tree/sub/b")
    s.run("touch -d 2001-01-01 tree/a")
    s.run("touch -d 2002-02-02 tree/sub/b")
    s.run("touch -d 2003-03-03 tree/sub")
    s.run("touch -d 2004-04-04 tree")  # (Toronto changed to daylight time at 02:00 that day: midnight is still EST)
    check("cp -rp a tree", s.run("cp -rp tree tree2"), "cp -rp tree tree2\n")
    check("...each file and directory has its source's time",
          [modify(p) for p in ("tree2", "tree2/a", "tree2/sub", "tree2/sub/b")],
          ["2004-04-04 00:00:00 EST", "2001-01-01 00:00:00 EST", "2003-03-03 00:00:00 EST", "2002-02-02 00:00:00 EST"])
    check("cp -pr (either order of the flags)", (s.run("cp -pr tree tree3"), modify("tree3/sub")), ("cp -pr tree tree3\n", "2003-03-03 00:00:00 EST"))
    check("cp -r without -p: new times", int(s.run("cp -r tree tree4") and modify("tree4/sub/b")[:4]) >= 2026, True)
    check("-p -v", s.run("cp -pv src v1"), "cp -pv src v1\n'src' -> 'v1'\n")

    # onto another volume
    check("mkdir /mnt", s.run("mkdir /mnt"), "mkdir /mnt\n")
    check("mount", s.run_status("mount LABEL=HOME /mnt"), ("mount LABEL=HOME /mnt\n", 0))
    check("cp -rp a tree to the other volume", s.run("cp -rp tree /mnt/tree"), "cp -rp tree /mnt/tree\n")
    check("...every time is the source's",
          [modify(f"/mnt/tree/{p}") for p in ("", "a", "sub", "sub/b")],
          ["2004-04-04 00:00:00 EST", "2001-01-01 00:00:00 EST", "2003-03-03 00:00:00 EST", "2002-02-02 00:00:00 EST"])
    check("the shell is alive", s.run("echo ok"), "echo ok\nok\n")


def verify_disk(ctx):
    fsck = subprocess.run(["fsck.fat", "-n", ctx.extra_imgs[0]], capture_output=True, text=True)
    ctx.check("disk (HOME): fsck.fat -n is clean", (fsck.returncode, fsck.stdout + fsck.stderr) if fsck.returncode else 0, 0)
