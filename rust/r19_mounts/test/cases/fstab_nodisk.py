"""Last updated: Stage 19, Step 3.

The image's own arrangement with no home disk attached: `LABEL=HOME /root` finds no volume, so the shell says so on the
serial log and starts in `/root` on the system volume -- an empty directory -- instead of failing. What is written there
lands on the system image (which is rebuilt every run), so a home directory with no home disk is as disposable as the
rest of the image.
"""

ENVIRONMENT = "HOME=/root\n"
FSTAB = "LABEL=HOME /root vfat defaults\n"


def run(ctx):
    s, check = ctx.s, ctx.check
    boot = s.log()
    notes = [l for l in boot.split("\n") if l.startswith("Fstab:")]
    check("boot: the missing disk is noted, nothing is fatal", notes, [
        "Fstab: /etc/fstab: line 1: LABEL=HOME on /root: no volume has that label or ID -- skipped.",
        "Fstab: 0 mount(s) from /etc/fstab.",
    ])
    check("boot: no complaint about HOME", "cannot enter" not in boot, True)
    check("the shell starts in /root", s.run("pwd"), "pwd\n/root\n")
    check("...an empty directory on the system volume", s.run("ls"), "ls\n")
    check("mount: only the root", s.run("mount"), "mount\nLABEL=SYSTEM on / type vfat\n")
    check("it can be written to", s.run("echo scratch > note.txt"), "echo scratch > note.txt\n")
    check("...and read back", s.run("cat /root/note.txt"), "cat /root/note.txt\nscratch\n")
    check("the shell is alive", s.run("echo ok"), "echo ok\nok\n")
