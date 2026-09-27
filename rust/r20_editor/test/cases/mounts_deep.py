"""Last updated: Stage 19, cleanup.

Mounts three deep (the system volume, then three more, each inside the last: `/a`, `/a/b`, `/a/b/c` -- the kernel drives four
devices), what resolves where at each level, `..` across all three boundaries, the order they can come off (innermost first,
each `EBUSY` while another is inside), `umount` refusing while a program holds a file open on the volume (`probe umount-open`) and
succeeding once it is closed, and that the kernel's open-file limit counts files on every volume together (`probe fds` given
paths on three volumes).
"""

import subprocess

ENVIRONMENT = "HOME=/\n"
EXTRA_DISKS = [
    {"label": "AAA", "volume_id": "0000000A", "size_kib": 1024},
    {"label": "BBB", "volume_id": "0000000B", "size_kib": 1024},
    {"label": "CCC", "volume_id": "0000000C", "size_kib": 1024},
]

BUSY = "Device or resource busy"


def run(ctx):
    s, check = ctx.s, ctx.check
    s.run("chmod +x /tests/probe")

    def cmd(text, want_out, want_status):
        check(text, s.run_status(text), (f"{text}\n{want_out}", want_status))

    # ================================================================= three levels
    s.run("mkdir /a")
    cmd("mount LABEL=AAA /a", "", 0)
    s.run("mkdir /a/b")
    cmd("mount LABEL=BBB /a/b", "", 0)
    s.run("mkdir /a/b/c")
    cmd("mount LABEL=CCC /a/b/c", "", 0)
    check("mount lists all four, outermost last", s.run("mount"),
          "mount\nLABEL=CCC on /a/b/c type vfat\nLABEL=BBB on /a/b type vfat\nLABEL=AAA on /a type vfat\nLABEL=SYSTEM on / type vfat\n")
    check("a file at each level", (s.run("echo 1 > /a/one"), s.run("echo 2 > /a/b/two"), s.run("echo 3 > /a/b/c/three")),
          ("echo 1 > /a/one\n", "echo 2 > /a/b/two\n", "echo 3 > /a/b/c/three\n"))
    check("each level lists its own (and the mount point below it)", (s.run("ls /a"), s.run("ls /a/b"), s.run("ls /a/b/c")),
          ("ls /a\nb\none\n", "ls /a/b\nc\ntwo\n", "ls /a/b/c\nthree\n"))
    check("find walks across all three boundaries", s.run("find /a"), "find /a\n/a\n/a/b\n/a/b/c\n/a/b/c/three\n/a/b/two\n/a/one\n")
    check("cd to the deepest and up through each", (s.run("cd /a/b/c"), s.run("pwd"), s.run("cat ../two"), s.run("cat ../../one"),
                                                    s.run("cd ../.."), s.run("pwd")),
          ("cd /a/b/c\n", "pwd\n/a/b/c\n", "cat ../two\n2\n", "cat ../../one\n1\n", "cd ../..\n", "pwd\n/a\n"))
    check("a path that goes down, up and down again", s.run("cat /a/b/c/../../one"), "cat /a/b/c/../../one\n1\n")
    s.run("cd /")

    # ================================================================= they come off innermost first
    cmd("umount /a", f"umount: /a: {BUSY}\n", 1)
    cmd("umount /a/b", f"umount: /a/b: {BUSY}\n", 1)
    check("the working directory in the deepest blocks it", (s.run("cd /a/b/c"), s.run_status("umount /a/b/c")),
          ("cd /a/b/c\n", ("umount /a/b/c\numount: /a/b/c: " + BUSY + "\n", 1)))
    s.run("cd /")

    # ================================================================= an open file blocks umount (a program does the opening)
    check("a program holding a file open on the volume: refused; closed: allowed",
          s.run("/tests/probe umount-open /a/b/c /a/b/c/three"),
          "/tests/probe umount-open /a/b/c /a/b/c/three\nopen ok, umount while open: -16, umount after close: 0\n")
    check("...the volume is gone, and the directory it hid is there again, empty", (s.run("ls /a/b/c"), s.run("mount").count("\n")),
          ("ls /a/b/c\n", 4))
    check("a directory held open blocks it too, and goes once it is closed", s.run("/tests/probe umount-open /a/b /a/b"),
          "/tests/probe umount-open /a/b /a/b\nopen ok, umount while open: -16, umount after close: 0\n")
    cmd("mount LABEL=BBB /a/b", "", 0)
    cmd("mount LABEL=CCC /a/b/c", "", 0)
    check("...and both are back, with what was on them", s.run("cat /a/b/two /a/b/c/three"), "cat /a/b/two /a/b/c/three\n2\n3\n")
    check("a file open on the outer volume with another mount inside it: the mount inside still blocks, closed or not",
          s.run("/tests/probe umount-open /a/b /a/b/two"),
          "/tests/probe umount-open /a/b /a/b/two\nopen ok, umount while open: -16, umount after close: -16\n")

    # ================================================================= the open-file limit is one, not one per volume
    one = s.run("/tests/probe fds /tests/notes.txt").split("\n")[1]
    check("the limit with one path", one.startswith("opened ") and one.endswith(", then -24"), True)
    mixed = s.run("/tests/probe fds /tests/notes.txt /a/one /a/b/two /a/b/c/three").split("\n")[1]
    check("the same limit with files taken in turn from four volumes", mixed, one)
    check("...and after closing them all a file can be opened", s.run("/tests/probe fds /a/b/two /a/b/c/three").split("\n")[2], "after closing all: open ok")

    # ================================================================= and then off, innermost first
    cmd("umount /a/b/c", "", 0)
    cmd("umount /a/b", "", 0)
    cmd("umount /a", "", 0)
    check("only the root is left", s.run("mount"), "mount\nLABEL=SYSTEM on / type vfat\n")
    check("...and the hidden directories are empty again", (s.run("ls /a"), s.run("ls /a/b")), ("ls /a\n", "ls /a/b\nls: cannot access '/a/b': No such file or directory\n"))
    check("the shell is alive", s.run("echo ok"), "echo ok\nok\n")


def verify_disk(ctx):
    for label, img in zip(("AAA", "BBB", "CCC"), ctx.extra_imgs):
        fsck = subprocess.run(["fsck.fat", "-n", img], capture_output=True, text=True)
        ctx.check(f"disk ({label}): fsck.fat -n is clean", (fsck.returncode, fsck.stdout + fsck.stderr) if fsck.returncode else 0, 0)
