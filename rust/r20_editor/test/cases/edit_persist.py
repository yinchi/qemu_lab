"""Last updated: Stage 20, Step 9.

Stage 20's own version of the property Stage 19's `persist.py` already demonstrated for plain shell
redirections: `edit` saves through the same `open`/`write`/`close` syscalls every other program uses,
so what it writes to the home disk outlives the kernel that wrote it. Boots twice -- the second time
on a fresh copy of the *freshly rebuilt* system image (`persist.py`'s own `boot_again` pattern), but
the same home disk -- with `edit` only ever run in the first boot and only `cat` used to check the
result in the second, so there is no way for the file to have merely lived on in one running kernel's
own memory rather than genuinely reaching the disk.

The home disk is this group's own scratch image (`EXTRA_DISKS`), never the repository's real
`home.img`.
"""

import os
import subprocess

from harness import Session, set_environment, set_fstab

ENVIRONMENT = "HOME=/root\n"
FSTAB = "LABEL=HOME /root vfat defaults\n"
EXTRA_DISKS = [{"label": "HOME", "volume_id": "5747A20E", "size_kib": 2048}]

TEXT = "edit wrote this before a reboot"


def boot_again(ctx, name):
    """A new QEMU on a fresh copy of the freshly built system image, with the same home disk attached
    -- `persist.py`'s own helper, not reused directly since test modules stay self-contained."""
    workdir = os.path.join(ctx.workdir, name)
    os.makedirs(workdir, exist_ok=True)
    img = os.path.join(workdir, "disk.img")
    subprocess.run(["cp", "--sparse=always", ctx.orig_img, img], check=True)
    set_environment(img, workdir, ENVIRONMENT)
    set_fstab(img, workdir, FSTAB)
    return Session(ctx.elf, img, workdir, [(path, False) for path in ctx.extra_imgs])


def run(ctx):
    s, check = ctx.s, ctx.check

    check("boot 1: HOME is mounted on /root", s.run("mount"),
          "mount\nLABEL=HOME on /root type vfat\nLABEL=SYSTEM on / type vfat\n")
    check("boot 1: the file doesn't exist yet", s.run("cat /root/notes.txt"),
          "cat /root/notes.txt\ncat: /root/notes.txt: No such file or directory\n")

    s.type("edit /root/notes.txt\n")
    s.screendump_settled(stable_for=0.5)  # let it draw its first frame before typing into it
    for c in TEXT:  # one key at a time, settling after each -- `edit` redraws the whole screen per key
        s.type(c)
        s.screendump_settled()
    s.keys(["ctrl-s"])
    s.screendump_settled(stable_for=0.5)
    s.keys(["ctrl-x"])
    s.wait_prompt()
    check("boot 1: the file is there before any reboot", s.run("cat /root/notes.txt"),
          f"cat /root/notes.txt\n{TEXT}\n")
    s.close()

    s2 = boot_again(ctx, "second")
    try:
        check("boot 2 (a freshly rebuilt system image, the same home disk): edit's save survived",
              s2.run("cat /root/notes.txt"), f"cat /root/notes.txt\n{TEXT}\n")
        check("...and it is a real file on the disk, not a fresh copy -- appending, then rereading, confirms it",
              s2.run("echo and this too >> /root/notes.txt"), "echo and this too >> /root/notes.txt\n")
        check("...", s2.run("cat /root/notes.txt"),
              f"cat /root/notes.txt\n{TEXT}\nand this too\n")
    finally:
        s2.close()


def verify_disk(ctx):
    home = ctx.extra_imgs[0]
    fsck = subprocess.run(["fsck.fat", "-n", home], capture_output=True, text=True)
    ctx.check("disk (HOME): fsck.fat -n is clean", (fsck.returncode, fsck.stdout + fsck.stderr) if fsck.returncode else 0, 0)
    out = os.path.join(ctx.workdir, "notes.out")
    subprocess.run(["mcopy", "-i", home, "::/notes.txt", out], check=True)
    with open(out) as f:
        ctx.check("the host sees the file edit wrote, across a reboot with a rebuilt system image",
                   f.read(), f"{TEXT}\nand this too\n")
