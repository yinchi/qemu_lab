"""Last updated: Stage 19, Step 4c.

`mv` between two volumes, where the kernel's `rename` says `EXDEV` and `mv` copies and removes instead: a file, a file
onto an existing one, into a directory, several at once, and a whole tree, in both directions, keeping modify times and the
executable and read-only bits; the order (everything is copied and checked before anything is removed, under a hidden
`.mv-partial` name that is renamed into place); and the failures -- a destination disk too small, for a file, for a tree
and for a file that would replace another -- which leave the source whole, the existing destination untouched and no
`.mv-partial` behind. (Before Stage 19's `mv` a cross-volume move onto an existing file deleted that file first and then
failed.) Same-volume moves are `user_progs`' and `flags`'.
"""

import subprocess

ENVIRONMENT = "HOME=/\nTZ=UTC\n"
EXTRA_DISKS = [
    {"label": "BIG", "volume_id": "5E6F7A8B", "size_kib": 2048},
    {"label": "SMALL", "volume_id": "0BADF00D", "size_kib": 128},
]

NOSPACE = "No space left on device"


def run(ctx):
    s, check = ctx.s, ctx.check

    def modify(path):
        for line in s.run(f"stat {path}").split("\n"):
            if line.startswith("Modify: "):
                return line[len("Modify: "):]

    def status(cmd, want_out, want_status):
        check(cmd, s.run_status(cmd), (f"{cmd}\n{want_out}", want_status))

    s.run("mkdir /w /mnt /small")
    check("mount BIG", s.run_status("mount LABEL=BIG /mnt"), ("mount LABEL=BIG /mnt\n", 0))
    check("mount SMALL", s.run_status("mount LABEL=SMALL /small"), ("mount LABEL=SMALL /small\n", 0))
    s.run("cd /w")

    # ================================================================= a file
    s.run("echo payload > f")
    s.run("touch -d '2011-11-11 11:11' f")
    check("mv a file to the other volume", s.run_status("mv f /mnt/f"), ("mv f /mnt/f\n", 0))
    check("...it is there, whole", s.run("cat /mnt/f"), "cat /mnt/f\npayload\n")
    check("...and no longer here", s.run_status("cat f"), ("cat f\ncat: f: No such file or directory\n", 1))
    check("...with its modify time", modify("/mnt/f"), "2011-11-11 11:11:00 UTC")
    check("nothing hidden was left behind", s.run("ls -a /mnt"), "ls -a /mnt\nf\n")
    check("mv back the other way", s.run_status("mv /mnt/f g"), ("mv /mnt/f g\n", 0))
    check("...still whole, still dated", (s.run("cat g"), modify("g")), ("cat g\npayload\n", "2011-11-11 11:11:00 UTC"))

    # ================================================================= attributes travel
    s.run("cp /bin/echo prog")
    s.run("chmod +x prog")
    s.run("echo locked > ro")
    s.run("chmod -w ro")
    s.run("mv prog ro /mnt")
    check("an executable stays executable", (s.run("/mnt/prog it works"), "exec=yes" in s.run("stat /mnt/prog")), ("/mnt/prog it works\nit works\n", True))
    check("a read-only file stays read-only", ("read-only=yes" in s.run("stat /mnt/ro"), s.run("cat /mnt/ro")), (True, "cat /mnt/ro\nlocked\n"))
    check("several sources into a directory on the other volume", s.run("ls /mnt"), "ls /mnt\nprog\nro\n")
    check("...none is left here", s.run("ls"), "ls\ng\n")

    # ================================================================= replacing an existing file
    s.run("echo new > /w/n")
    s.run("echo old > /mnt/n")
    check("mv onto an existing file replaces it", s.run_status("mv n /mnt/n"), ("mv n /mnt/n\n", 0))
    check("...the old content is gone", s.run("cat /mnt/n"), "cat /mnt/n\nnew\n")
    s.run("echo again > n2")
    s.run("echo there > /mnt/n2")
    status("mv -n n2 /mnt/n2", "mv: not replacing '/mnt/n2'\n", 1)
    check("-n: both untouched", (s.run("cat n2"), s.run("cat /mnt/n2")), ("cat n2\nagain\n", "cat /mnt/n2\nthere\n"))
    check("-v says what was moved", s.run("mv -v n2 /mnt/v"), "mv -v n2 /mnt/v\nrenamed 'n2' -> '/mnt/v'\n")

    # ================================================================= a tree
    s.run("cp -rp /tests/tree t")
    s.run("touch -d 2001-01-01 t/a.txt")
    s.run("touch -d 2002-02-02 t/sub1/b.txt")
    s.run("touch -d 2003-03-03 t/sub1")
    s.run("touch -d 2004-04-04 t")
    check("mv a directory tree to the other volume", s.run_status("mv t /mnt/t"), ("mv t /mnt/t\n", 0))
    check("...all of it is there", s.run("find /mnt/t"),
          "find /mnt/t\n/mnt/t\n/mnt/t/a.txt\n/mnt/t/sub1\n/mnt/t/sub1/b.txt\n/mnt/t/sub1/sub2\n/mnt/t/sub1/sub2/c.txt\n")
    check("...none of it is here", s.run_status("ls t"), ("ls t\nls: cannot access 't': No such file or directory\n", 1))
    check("...the directory and the files keep their times",
          [modify(p) for p in ("/mnt/t", "/mnt/t/sub1", "/mnt/t/sub1/b.txt")],
          ["2004-04-04 00:00:00 UTC", "2003-03-03 00:00:00 UTC", "2002-02-02 00:00:00 UTC"])
    check("...and the contents", s.run("cat /mnt/t/sub1/sub2/c.txt"), "cat /mnt/t/sub1/sub2/c.txt\n" + ctx.fixture("tree/sub1/sub2/c.txt"))
    check("mv the tree back", s.run_status("mv /mnt/t back"), ("mv /mnt/t back\n", 0))
    check("...intact", s.run("find back"), "find back\nback\nback/a.txt\nback/sub1\nback/sub1/b.txt\nback/sub1/sub2\nback/sub1/sub2/c.txt\n")
    check("...and nothing hidden left on either volume", (s.run("ls -a /mnt"), s.run("ls -a")), ("ls -a /mnt\nn\nn2\nprog\nro\nv\n", "ls -a\nback\ng\n"))
    status("mv back /mnt/prog", "mv: cannot move 'back' to '/mnt/prog': File exists\n", 1)  # a directory onto a file: refused, as on one volume

    # ================================================================= a leftover hidden name is not touched
    s.run("echo mine > /mnt/.mv-partial")
    check("a taken hidden name: the next one is used", s.run_status("mv g /mnt/g"), ("mv g /mnt/g\n", 0))
    check("...the leftover is as it was, the new file is there", (s.run("cat /mnt/.mv-partial"), s.run("cat /mnt/g")),
          ("cat /mnt/.mv-partial\nmine\n", "cat /mnt/g\npayload\n"))
    check("...and no other hidden name is left", s.run("ls -a /mnt"), "ls -a /mnt\n.mv-partial\ng\nn\nn2\nprog\nro\nv\n")
    s.run("rm /mnt/.mv-partial")

    # ================================================================= failures: the destination is too small
    s.run("chmod +x /tests/bigpad")
    s.run("cp /tests/bigpad big")
    s.run("echo keep > /small/keep")
    check("a file too big for the destination", s.run_status("mv big /small/big"),
          (f"mv big /small/big\nmv: error writing '/small/.mv-partial': {NOSPACE}\n", 1))
    check("...the source is whole", s.run_status("cmp big /tests/bigpad"), ("cmp big /tests/bigpad\n", 0))
    check("...nothing was left on the destination", s.run("ls -a /small"), "ls -a /small\nkeep\n")
    check("one that would replace an existing file: it fails and the old file survives", s.run_status("mv big /small/keep"),
          (f"mv big /small/keep\nmv: error writing '/small/.mv-partial': {NOSPACE}\n", 1))
    check("...the existing file is untouched", s.run("cat /small/keep"), "cat /small/keep\nkeep\n")
    s.run("mkdir tr")
    s.run("echo one > tr/a")
    s.run("cp /tests/bigpad tr/big")
    s.run("echo two > tr/z")
    out = s.run_status("mv tr /small/tr")
    check("a tree too big for the destination fails", (out[1], f"mv: error writing '/small/.mv-partial/big': {NOSPACE}" in out[0]), (1, True))
    check("...every file of the source is still there and whole", (s.run("find tr"), s.run_status("cmp tr/big /tests/bigpad")[1]),
          ("find tr\ntr\ntr/a\ntr/big\ntr/z\n", 0))
    check("...and the destination has no trace of it", s.run("ls -a /small"), "ls -a /small\nkeep\n")

    # ================================================================= what stays refused
    status("mv /mnt /elsewhere", "mv: cannot move '/mnt' to '/elsewhere': Device or resource busy\n", 1)
    s.run("echo payload > g")
    status("mv g /mnt/n/x", "mv: cannot stat '/mnt/n/x': Not a directory\n", 1)
    status("mv g /mnt/nodir/x", "mv: cannot move 'g' to '/mnt/nodir/x': No such file or directory\n", 1)
    check("...g is still here", s.run("cat g"), "cat g\npayload\n")
    check("the shell is alive", s.run("echo ok"), "echo ok\nok\n")


def verify_disk(ctx):
    for label, img in zip(("BIG", "SMALL"), ctx.extra_imgs):
        fsck = subprocess.run(["fsck.fat", "-n", img], capture_output=True, text=True)
        ctx.check(f"disk ({label}): fsck.fat -n is clean", (fsck.returncode, fsck.stdout + fsck.stderr) if fsck.returncode else 0, 0)
