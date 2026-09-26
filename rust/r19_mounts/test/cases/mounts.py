"""Last updated: Stage 19, Step 2.

Mounting a volume on a directory, by hand: the `mount` and `umount` programs over the `mount` and `umount` syscalls, and
the kernel's path resolution through the mount table. Three extra disks are attached -- `HOME` (ID `5E6F-7A8B`), `DATA`
(`0BAD-F00D`) and a second volume also labelled `HOME` (`1111-2222`, which lands in the lowest slot, so it is the first `HOME`
by device order and the one `LABEL=HOME` finds) -- beside the system image, which stays the root throughout.

What is pinned down: `mount` listing; a mount hiding what was under the directory and uncovering it again; paths through
a mount (`..` out of it, relative paths from inside, `find`); files created, copied, moved and run on a mounted volume;
the rules of a mount point (busy to remove, rename or mount over) and of `umount` (not a mount point, the root, another
mount inside, a working directory inside); every way `mount` can fail; and that a rename across two volumes is refused
(`EXDEV`) rather than corrupting either. `verify_disk` then reads the extra images from the host: what was written to a
mounted volume reached its own disk, and every image passes `fsck.fat`.
"""

import os
import subprocess

from harness import file_hash, lsblk_table, mcopy_out

EXTRA_DISKS = [
    {"label": "HOME", "volume_id": "5E6F7A8B", "size_kib": 2048},
    {"label": "DATA", "volume_id": "0BADF00D", "size_kib": 2048},
    {"label": "HOME", "volume_id": "11112222", "size_kib": 2048},
]
ENVIRONMENT = "HOME=/\n"

BUSY = "Device or resource busy"


def run(ctx):
    s, check = ctx.s, ctx.check

    def cmd(text, want_out, want_status):
        check(text, s.run_status(text), (f"{text}\n{want_out}", want_status))

    def rows():
        return lsblk_table(s.run("lsblk"))

    # ---------------------------------------------------------------- what is mounted at boot: the root only
    def listing(table):
        """What `mount` prints for a table from `lsblk`: each mounted volume, in device order, spelled as `mount` takes it
        (`LABEL=` if that names it alone, else `UUID=`)."""
        lines = ""
        for r in table:
            if r["MOUNTPOINT"]:
                shared = sum(1 for o in table if o["LABEL"] == r["LABEL"]) > 1
                source = f"UUID={r['UUID']}" if (not r["LABEL"] or shared) else f"LABEL={r['LABEL']}"
                lines += f"{source} on {r['MOUNTPOINT']} type vfat\n"
        return "mount\n" + lines

    table = rows()
    check("boot: only the root is mounted", [(r["UUID"], r["MOUNTPOINT"]) for r in table if r["MOUNTPOINT"]], [("0000-0000", "/")])
    check("mount: lists the root", s.run("mount"), "mount\nLABEL=SYSTEM on / type vfat\n")
    check("mount --help", s.run("mount --help"), "mount --help\nusage: mount [SOURCE TARGET]\n")
    check("umount --help", s.run("umount --help"), "umount --help\nusage: umount TARGET\n")

    # ---------------------------------------------------------------- a mount hides what was under the directory
    check("mkdir /mnt", s.run("mkdir /mnt"), "mkdir /mnt\n")
    check("a file on the system volume, under the mount point", s.run("echo under > /mnt/under.txt"), "echo under > /mnt/under.txt\n")
    check("mount by label", s.run_status("mount LABEL=HOME /mnt"), ("mount LABEL=HOME /mnt\n", 0))
    table = rows()
    mounted = [(r["UUID"], r["MOUNTPOINT"]) for r in table if r["MOUNTPOINT"]]
    # `LABEL=HOME` is two volumes: the first by device order is the one mounted (the other is told apart by its ID).
    first_home = next(r for r in table if r["LABEL"] == "HOME")  # `lsblk` lists devices in device order
    check("lsblk: the first HOME volume is mounted at /mnt", sorted(mounted), sorted([("0000-0000", "/"), (first_home["UUID"], "/mnt")]))
    check("the first HOME by device order is the second-attached one", first_home["UUID"], "1111-2222")
    check("mount: lists both, the shared label spelled as an ID", s.run("mount"), "mount\nUUID=1111-2222 on /mnt type vfat\nLABEL=SYSTEM on / type vfat\n")
    check("ls /mnt: the mounted volume is empty, what was under it is hidden", s.run("ls /mnt"), "ls /mnt\n")
    check("cat: the hidden file cannot be reached", s.run_status("cat /mnt/under.txt"), ("cat /mnt/under.txt\ncat: /mnt/under.txt: No such file or directory\n", 1))
    check("umount", s.run_status("umount /mnt"), ("umount /mnt\n", 0))
    check("umount uncovers it again", s.run("cat /mnt/under.txt"), "cat /mnt/under.txt\nunder\n")
    check("lsblk: nothing but the root is mounted again", [(r["UUID"], r["MOUNTPOINT"]) for r in rows() if r["MOUNTPOINT"]], [("0000-0000", "/")])
    check("rm the file that was under it", s.run("rm /mnt/under.txt"), "rm /mnt/under.txt\n")

    # ---------------------------------------------------------------- UUID mounts, and files on a mounted volume
    check("mount by UUID", s.run_status("mount UUID=5E6F-7A8B /mnt"), ("mount UUID=5E6F-7A8B /mnt\n", 0))
    check("mount: the UUID picked that one of the two HOME volumes", s.run("mount"), "mount\nUUID=5E6F-7A8B on /mnt type vfat\nLABEL=SYSTEM on / type vfat\n")
    check("write a file on the mounted volume", s.run("echo one > /mnt/a.txt"), "echo one > /mnt/a.txt\n")
    check("append to it", s.run("echo two >> /mnt/a.txt"), "echo two >> /mnt/a.txt\n")
    check("read it back", s.run("cat /mnt/a.txt"), "cat /mnt/a.txt\none\ntwo\n")
    check("ls /mnt", s.run("ls /mnt"), "ls /mnt\na.txt\n")
    check("ls /mnt/ (trailing slash)", s.run("ls /mnt/"), "ls /mnt/\na.txt\n")
    check("stat /mnt/a.txt", s.run("stat /mnt/a.txt").split("\n")[1:3], ["  File: /mnt/a.txt", "  Size: 8            Type: regular file"])
    check("stat /mnt: a directory", s.run("stat /mnt").split("\n")[1:3], ["  File: /mnt", "  Size: 0            Type: directory"])
    check("mkdir on the mounted volume", s.run("mkdir /mnt/d /mnt/d/e"), "mkdir /mnt/d /mnt/d/e\n")
    check("find /mnt", s.run("find /mnt"), "find /mnt\n/mnt\n/mnt/a.txt\n/mnt/d\n/mnt/d/e\n")
    check("mkdir /mnt: the mount point exists", s.run_status("mkdir /mnt"), ("mkdir /mnt\nmkdir: cannot create directory '/mnt': File exists\n", 1))

    # ---------------------------------------------------------------- paths through a mount
    check("cd into the mount", s.run("cd /mnt/d"), "cd /mnt/d\n")
    check("pwd inside it", s.run("pwd"), "pwd\n/mnt/d\n")
    check("a relative path inside it", s.run("cat ../a.txt"), "cat ../a.txt\none\ntwo\n")
    check("up out of the mount, to the system volume", s.run("cat ../../bin/../tests/hello.txt").split("\n")[0], "cat ../../bin/../tests/hello.txt")
    check("cd .. twice: out of the mount", s.run("cd ../.."), "cd ../..\n")
    check("pwd is on the system volume again", s.run("pwd"), "pwd\n/\n")
    check("cd /mnt/..", s.run("cd /mnt/.."), "cd /mnt/..\n")
    check("...is /", s.run("pwd"), "pwd\n/\n")
    check("/mnt/../mnt/a.txt", s.run("cat /mnt/../mnt/a.txt"), "cat /mnt/../mnt/a.txt\none\ntwo\n")
    check("find / crosses into the mount", "/mnt/d/e" in s.run("find /").split("\n"), True)

    # ---------------------------------------------------------------- programs, copies and moves
    check("cp a program onto the mounted volume", s.run("cp /bin/echo /mnt/echo"), "cp /bin/echo /mnt/echo\n")
    check("the copy is the same bytes", s.run_status("cmp /bin/echo /mnt/echo"), ("cmp /bin/echo /mnt/echo\n", 0))
    check("it does not run yet: no exec bit", s.run_status("/mnt/echo hi"), ("/mnt/echo hi\n/mnt/echo: Permission denied\n", 126))
    check("chmod +x on the mounted volume", s.run("chmod +x /mnt/echo"), "chmod +x /mnt/echo\n")
    check("a program run from the mounted volume", s.run("/mnt/echo hi there"), "/mnt/echo hi there\nhi there\n")
    check("cp -r a tree across", s.run("cp -r /tests/tree /mnt/tree"), "cp -r /tests/tree /mnt/tree\n")
    check("the tree is there", s.run("find /mnt/tree"), "find /mnt/tree\n/mnt/tree\n/mnt/tree/a.txt\n/mnt/tree/sub1\n/mnt/tree/sub1/b.txt\n/mnt/tree/sub1/sub2\n/mnt/tree/sub1/sub2/c.txt\n")
    check("a copied file is equal", s.run_status("cmp /tests/tree/sub1/sub2/c.txt /mnt/tree/sub1/sub2/c.txt"), ("cmp /tests/tree/sub1/sub2/c.txt /mnt/tree/sub1/sub2/c.txt\n", 0))
    check("mv within the mounted volume", s.run("mv /mnt/a.txt /mnt/d/a.txt"), "mv /mnt/a.txt /mnt/d/a.txt\n")
    check("mv a directory within it", s.run("mv /mnt/d /mnt/dd"), "mv /mnt/d /mnt/dd\n")
    check("...and its contents came along", s.run("cat /mnt/dd/a.txt"), "cat /mnt/dd/a.txt\none\ntwo\n")
    check("mv across volumes: a file", s.run_status("mv /tests/hello.txt /mnt/hello.txt"),
          ("mv /tests/hello.txt /mnt/hello.txt\nmv: cannot move '/tests/hello.txt' to '/mnt/hello.txt': Invalid cross-device link\n", 1))
    check("mv across volumes: the other way", s.run_status("mv /mnt/dd/a.txt /a.txt"),
          ("mv /mnt/dd/a.txt /a.txt\nmv: cannot move '/mnt/dd/a.txt' to '/a.txt': Invalid cross-device link\n", 1))
    check("mv across volumes: a directory", s.run_status("mv /mnt/dd /dd"),
          ("mv /mnt/dd /dd\nmv: cannot move '/mnt/dd' to '/dd': Invalid cross-device link\n", 1))
    check("nothing moved, nothing lost", s.run("find /mnt/dd"), "find /mnt/dd\n/mnt/dd\n/mnt/dd/a.txt\n/mnt/dd/e\n")
    check("rm on the mounted volume", s.run("rm /mnt/echo"), "rm /mnt/echo\n")
    check("rm -r a tree there", s.run("rm -r /mnt/tree"), "rm -r /mnt/tree\n")
    check("what is left", s.run("ls /mnt"), "ls /mnt\ndd\n")

    # ---------------------------------------------------------------- a mount point cannot be removed, renamed or covered
    cmd("rmdir /mnt", f"rmdir: failed to remove '/mnt': {BUSY}\n", 1)
    cmd("rm -d /mnt", f"rm: cannot remove '/mnt': {BUSY}\n", 1)
    cmd("mv /mnt /elsewhere", f"mv: cannot move '/mnt' to '/elsewhere': {BUSY}\n", 1)
    cmd("mv /tests /mnt", "mv: cannot move '/tests' to '/mnt/tests': Invalid cross-device link\n", 1)  # a directory: it goes *into* /mnt
    check("ls /: the mount point is still an entry", "mnt" in s.run("ls /").split("\n"), True)

    # ---------------------------------------------------------------- every way `mount` can fail
    cmd("mount UUID=0BAD-F00D /elsewhere", "mount: UUID=0BAD-F00D on /elsewhere: No such file or directory\n", 1)  # no such directory
    check("mkdir /other", s.run("mkdir /other"), "mkdir /other\n")
    cmd("mount UUID=5E6F-7A8B /other", f"mount: UUID=5E6F-7A8B on /other: {BUSY}\n", 1)  # already mounted, at /mnt
    cmd("mount UUID=0BAD-F00D /mnt", f"mount: UUID=0BAD-F00D on /mnt: {BUSY}\n", 1)  # something is mounted there
    cmd("mount LABEL=NOPE /other", "mount: LABEL=NOPE on /other: no volume has that label or ID\n", 1)
    cmd("mount UUID=DEAD-BEEF /other", "mount: UUID=DEAD-BEEF on /other: no volume has that label or ID\n", 1)
    cmd("mount /dev/vda /other", "mount: /dev/vda on /other: not a LABEL= or UUID= source, or not a FAT filesystem\n", 1)
    cmd("mount UUID=0BADF00D /other", "mount: UUID=0BADF00D on /other: not a LABEL= or UUID= source, or not a FAT filesystem\n", 1)
    cmd("mount label=DATA /other", "mount: label=DATA on /other: not a LABEL= or UUID= source, or not a FAT filesystem\n", 1)
    cmd("mount LABEL=SYSTEM /other", f"mount: LABEL=SYSTEM on /other: {BUSY}\n", 1)  # the root volume is mounted already
    cmd("mount LABEL=DATA /bin/cat", "mount: LABEL=DATA on /bin/cat: Not a directory\n", 1)
    cmd("mount LABEL=DATA /", f"mount: LABEL=DATA on /: {BUSY}\n", 1)
    cmd("mount LABEL=DATA", "mount: missing operand after 'LABEL=DATA'\nTry 'mount --help' for more information.\n", 1)
    cmd("mount a b c", "mount: extra operand 'c'\nTry 'mount --help' for more information.\n", 1)
    cmd("mount -x", "mount: invalid option -- 'x'\nTry 'mount --help' for more information.\n", 1)
    check("nothing of that was mounted", s.run("mount"), "mount\nUUID=5E6F-7A8B on /mnt type vfat\nLABEL=SYSTEM on / type vfat\n")

    # ---------------------------------------------------------------- a mount cannot cover another mount
    check("mkdir /p /p/q", s.run("mkdir /p /p/q"), "mkdir /p /p/q\n")
    check("mount DATA at /p/q", s.run_status("mount LABEL=DATA /p/q"), ("mount LABEL=DATA /p/q\n", 0))
    cmd("mount UUID=1111-2222 /p", f"mount: UUID=1111-2222 on /p: {BUSY}\n", 1)  # it would cover /p/q
    cmd("mount UUID=1111-2222 /p/q", f"mount: UUID=1111-2222 on /p/q: {BUSY}\n", 1)  # and /p/q itself is taken
    cmd("mount UUID=1111-2222 /p/q/nosuch", "mount: UUID=1111-2222 on /p/q/nosuch: No such file or directory\n", 1)  # inside DATA, where there is no such directory
    cmd("umount /p/q", "", 0)
    cmd("mount UUID=1111-2222 /p", "", 0)  # with nothing inside /p, it is allowed
    cmd("umount /p", "", 0)
    check("rmdir /p/q /p", s.run("rmdir /p/q /p"), "rmdir /p/q /p\n")

    # ---------------------------------------------------------------- nesting
    check("mkdir /mnt/sub, on the mounted volume", s.run("mkdir /mnt/sub"), "mkdir /mnt/sub\n")
    check("mount a second volume inside the first", s.run_status("mount LABEL=DATA /mnt/sub"), ("mount LABEL=DATA /mnt/sub\n", 0))
    check("write to it", s.run("echo nested > /mnt/sub/n.txt"), "echo nested > /mnt/sub/n.txt\n")
    check("ls /mnt/sub", s.run("ls /mnt/sub"), "ls /mnt/sub\nn.txt\n")
    check("ls /mnt: the mount point is an entry of the volume it is on", s.run("ls /mnt").split("\n")[1:], ["dd", "sub", ""])
    check("mount: all three", s.run("mount"), listing(rows()))
    check("mount: spelled the way `mount` takes them", listing(rows()), "mount\nLABEL=DATA on /mnt/sub type vfat\nUUID=5E6F-7A8B on /mnt type vfat\nLABEL=SYSTEM on / type vfat\n")  # device order
    cmd("umount /mnt", f"umount: /mnt: {BUSY}\n", 1)  # with another mount inside
    check("cd into the inner mount", s.run("cd /mnt/sub"), "cd /mnt/sub\n")
    cmd("umount /mnt/sub", f"umount: /mnt/sub: {BUSY}\n", 1)  # the shell's working directory is in it
    check("cd ..: now on the outer volume", s.run("cd .."), "cd ..\n")
    check("...at /mnt", s.run("pwd"), "pwd\n/mnt\n")
    cmd("umount /mnt/sub", "", 0)
    cmd("umount /mnt/sub", "umount: /mnt/sub: not a mount point\n", 1)
    check("the inner directory is empty again", s.run("ls /mnt/sub"), "ls /mnt/sub\n")
    cmd("umount /mnt", f"umount: /mnt: {BUSY}\n", 1)  # the working directory is /mnt itself
    check("cd /", s.run("cd /"), "cd /\n")
    cmd("umount /", f"umount: /: {BUSY}\n", 1)
    cmd("umount /other", "umount: /other: not a mount point\n", 1)
    cmd("umount /mnt/dd", "umount: /mnt/dd: not a mount point\n", 1)
    cmd("umount", "umount: missing operand\nTry 'umount --help' for more information.\n", 1)
    cmd("umount /mnt /other", "umount: extra operand '/other'\nTry 'umount --help' for more information.\n", 1)
    cmd("umount /nowhere", "umount: /nowhere: not a mount point\n", 1)
    cmd("umount /mnt", "", 0)
    check("nothing but the root is mounted", s.run("mount"), "mount\nLABEL=SYSTEM on / type vfat\n")
    check("lsblk: no mount points but the root", [(r["UUID"], r["MOUNTPOINT"]) for r in rows() if r["MOUNTPOINT"]], [("0000-0000", "/")])

    # ---------------------------------------------------------------- what was written stays on its own disk
    check("mount again: the file is still there", s.run_status("mount UUID=5E6F-7A8B /mnt"), ("mount UUID=5E6F-7A8B /mnt\n", 0))
    check("...and so is the directory", s.run("find /mnt"), "find /mnt\n/mnt\n/mnt/dd\n/mnt/dd/a.txt\n/mnt/dd/e\n/mnt/sub\n")
    check("the other HOME was never touched: mount it and look", s.run_status("mount UUID=1111-2222 /other"), ("mount UUID=1111-2222 /other\n", 0))
    check("...it is empty", s.run("ls /other"), "ls /other\n")
    check("one more file, on the second HOME", s.run("echo second > /other/s.txt"), "echo second > /other/s.txt\n")
    check("the shell is alive", s.run("echo ok"), "echo ok\nok\n")


def verify_disk(ctx):
    check = ctx.check
    home, data, home2 = ctx.extra_imgs
    for label, img in (("HOME", home), ("DATA", data), ("second HOME", home2)):
        fsck = subprocess.run(["fsck.fat", "-n", img], capture_output=True, text=True)
        check(f"disk ({label}): fsck.fat -n is clean", (fsck.returncode, fsck.stdout + fsck.stderr) if fsck.returncode else 0, 0)

    def read(img, name):
        out = os.path.join(ctx.workdir, "out.bin")
        if os.path.exists(out):
            os.remove(out)
        mcopy_out(img, name, out)
        with open(out) as f:
            return f.read()

    def listing(img, path="::/"):
        out = subprocess.run(["mdir", "-b", "-i", img, path], capture_output=True, text=True).stdout
        return sorted(line.strip() for line in out.splitlines() if line.strip())

    check("HOME: what was written through the mount", (read(home, "/dd/a.txt"), listing(home)), ("one\ntwo\n", ["::/dd/", "::/sub/"]))
    check("DATA: what was written to the nested mount", (read(data, "/n.txt"), listing(data)), ("nested\n", ["::/n.txt"]))
    check("second HOME: only its own file", (read(home2, "/s.txt"), listing(home2)), ("second\n", ["::/s.txt"]))
