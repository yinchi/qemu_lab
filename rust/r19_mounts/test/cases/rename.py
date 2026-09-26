"""Last updated: Stage 19, Step 4a.

The `rename` syscall on its own (through `probe rename OLD NEW`, which puts no `mv` rules on top): from Stage 19 it
replaces an existing destination as POSIX's does -- a file replaces a file, a directory replaces an empty directory --
and refuses the rest with the errors POSIX gives, leaving both sides untouched. Between two volumes it is `EXDEV`, and
for a mount point `EBUSY`, still without touching the destination. (`mv`, which decides what to do with an existing
directory or a cross-volume move, is `mv`'s own group.)
"""

import os
import subprocess

EXTRA_DISKS = [{"label": "HOME", "volume_id": "5E6F7A8B", "size_kib": 2048}]

ENOENT, EBUSY, EXDEV, ENOTDIR, EISDIR, EINVAL, ENOTEMPTY = -2, -16, -18, -20, -21, -22, -39


def run(ctx):
    s, check = ctx.s, ctx.check

    def rename(old, new):
        """The return value `probe rename` prints."""
        out = s.run(f"/tests/probe rename {old} {new}")
        return int(out.split("rename: ")[1])

    def sh(command):
        s.run(command)

    s.run("chmod +x /tests/probe")  # the kernel marks only /bin's programs executable at boot
    check("setup", s.run("mkdir /w"), "mkdir /w\n")
    sh("cd /w")

    # --- a file replaces a file, and the old one is gone ---
    sh("echo new > a")
    sh("echo old > b")
    check("file over file: replaced", rename("a", "b"), 0)
    check("...the source name is gone", s.run_status("cat a"), ("cat a\ncat: a: No such file or directory\n", 1))
    check("...the destination holds the source's content", s.run("cat b"), "cat b\nnew\n")
    check("...and nothing else was left behind", s.run("ls"), "ls\nb\n")

    # --- a read-only destination is replaced too (deletion is governed by the directory) ---
    sh("echo again > c")
    sh("chmod -w b")
    check("file over a read-only file", rename("c", "b"), 0)
    check("...replaced", s.run("cat b"), "cat b\nagain\n")

    # --- the same path: nothing happens ---
    check("a path onto itself", rename("b", "b"), 0)
    check("...and the file is as it was", s.run("cat b"), "cat b\nagain\n")

    # --- mixed kinds ---
    sh("mkdir d")
    sh("echo inside > d/f")
    check("file onto a directory", rename("b", "d"), EISDIR)
    check("directory onto a file", rename("d", "b"), ENOTDIR)
    check("...both are as they were", (s.run("cat b"), s.run("cat d/f")), ("cat b\nagain\n", "cat d/f\ninside\n"))

    # --- directories: onto an empty one replaces, onto one with contents does not ---
    sh("mkdir e")
    check("directory onto an empty directory", rename("d", "e"), 0)
    check("...the contents came along", s.run("cat e/f"), "cat e/f\ninside\n")
    check("...and the source name is gone", s.run("ls"), "ls\nb\ne\n")
    sh("mkdir g")
    sh("echo keep > g/k")
    check("directory onto a directory with contents", rename("e", "g"), ENOTEMPTY)
    check("...both intact", (s.run("ls e"), s.run("ls g")), ("ls e\nf\n", "ls g\nk\n"))

    # --- a directory into its own subtree: refused before anything is removed ---
    sh("mkdir -p h/i")
    check("directory into its own subtree", rename("h", "h/i/j"), EINVAL)
    check("directory onto its own descendant (which exists)", rename("h", "h/i"), EINVAL)
    check("...still there", s.run("ls h"), "ls h\ni\n")

    # --- missing pieces ---
    check("missing source", rename("nosuch", "x"), ENOENT)
    check("missing destination directory", rename("b", "nodir/x"), ENOENT)
    check("destination directory is a file", rename("b", "b/x"), ENOTDIR)
    check("...b is still there", s.run("cat b"), "cat b\nagain\n")

    # --- across volumes: EXDEV, and the destination is untouched ---
    sh("mkdir /mnt")
    check("mount the second volume", s.run_status("mount LABEL=HOME /mnt"), ("mount LABEL=HOME /mnt\n", 0))
    sh("echo precious > /mnt/p")
    check("rename across volumes", rename("b", "/mnt/p"), EXDEV)
    check("...the destination was not replaced, the source not moved", (s.run("cat /mnt/p"), s.run("cat b")), ("cat /mnt/p\nprecious\n", "cat b\nagain\n"))
    check("a mount point as the source", rename("/mnt", "m2"), EBUSY)
    check("a mount point as the destination", rename("b", "/mnt"), EBUSY)
    check("...and nothing changed", s.run("ls /mnt"), "ls /mnt\np\n")
    check("within the second volume it replaces", (s.run("echo q > /mnt/q"), rename("/mnt/q", "/mnt/p"), s.run("cat /mnt/p")),
          ("echo q > /mnt/q\n", 0, "cat /mnt/p\nq\n"))
    check("the shell is alive", s.run("echo ok"), "echo ok\nok\n")


def verify_disk(ctx):
    fsck = subprocess.run(["fsck.fat", "-n", ctx.extra_imgs[0]], capture_output=True, text=True)
    ctx.check("disk (HOME): fsck.fat -n is clean", (fsck.returncode, fsck.stdout + fsck.stderr) if fsck.returncode else 0, 0)
