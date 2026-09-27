"""Last updated: Stage 19, cleanup.

The property the whole stage exists for: what is written to the home disk outlives a rebuilt system image. The first boot
writes under `/root` (the mount point of `HOME`, from `/etc/fstab`) and, for contrast, on the system volume; QEMU is then stopped and
booted again -- on a **fresh copy of the freshly built system image**, as `just run` would after `just disk`, but the *same* home
disk -- and the files written to `HOME` are there, the ones written to the system volume are gone, and the volume is the same
one (same ID). A third boot with no home disk attached shows an empty `/root` and a note, and leaves the disk alone. The
home disk is the group's own extra image, never the real `home.img`.
"""

import os
import shutil
import subprocess

from harness import DEFAULT_FSTAB, Session, file_hash, lsblk_table, set_environment, set_fstab

ENVIRONMENT = "HOME=/\n"
FSTAB = "LABEL=HOME /root vfat defaults\n"
EXTRA_DISKS = [{"label": "HOME", "volume_id": "5E6F7A8B", "size_kib": 2048}]


def boot_again(ctx, name, with_home):
    """A new QEMU on a fresh copy of the freshly built system image (the same environment and `fstab` as the group's),
    with or without the home disk; returns the session."""
    workdir = os.path.join(ctx.workdir, name)
    os.makedirs(workdir, exist_ok=True)
    img = os.path.join(workdir, "disk.img")
    subprocess.run(["cp", "--sparse=always", ctx.orig_img, img], check=True)
    set_environment(img, workdir, ENVIRONMENT)
    set_fstab(img, workdir, FSTAB)
    return Session(ctx.elf, img, workdir, [(path, False) for path in ctx.extra_imgs] if with_home else [])


def run(ctx):
    s, check = ctx.s, ctx.check

    # ================================================================= first boot: write to both volumes
    check("boot 1: HOME is mounted on /root", s.run("mount"), "mount\nLABEL=HOME on /root type vfat\nLABEL=SYSTEM on / type vfat\n")
    check("boot 1: /root starts empty", s.run("ls /root"), "ls /root\n")
    check("write a file on the home disk", s.run("echo kept > /root/note"), "echo kept > /root/note\n")
    check("...a directory and a file in it", (s.run("mkdir /root/dir"), s.run("echo deep > /root/dir/f")), ("mkdir /root/dir\n", "echo deep > /root/dir/f\n"))
    check("...and one on the system volume", s.run("echo throwaway > /gone"), "echo throwaway > /gone\n")
    check("both are there", (s.run("cat /root/note"), s.run("cat /gone")), ("cat /root/note\nkept\n", "cat /gone\nthrowaway\n"))
    home_id = [r["UUID"] for r in lsblk_table(s.run("lsblk")) if r["LABEL"] == "HOME"]
    s.close()

    # ================================================================= second boot: a rebuilt system image, the same home disk
    s2 = boot_again(ctx, "second", with_home=True)
    try:
        check("boot 2: the home disk is mounted again", s2.run("mount"), "mount\nLABEL=HOME on /root type vfat\nLABEL=SYSTEM on / type vfat\n")
        check("...it is the same volume", [r["UUID"] for r in lsblk_table(s2.run("lsblk")) if r["LABEL"] == "HOME"], home_id)
        check("what was written to it is there", (s2.run("cat /root/note"), s2.run("cat /root/dir/f"), s2.run("ls /root")),
              ("cat /root/note\nkept\n", "cat /root/dir/f\ndeep\n", "ls /root\ndir\nnote\n"))
        check("what was written to the system volume is gone", s2.run_status("cat /gone"), ("cat /gone\ncat: /gone: No such file or directory\n", 1))
        check("more can be added", s2.run("echo more >> /root/note"), "echo more >> /root/note\n")
    finally:
        s2.close()

    # ================================================================= third boot: no home disk
    s3 = boot_again(ctx, "third", with_home=False)
    try:
        notes = [l for l in s3.log().split("\n") if l.startswith("Fstab:")]
        check("boot 3: the missing disk is a note", notes, [
            "Fstab: /etc/fstab: line 1: LABEL=HOME on /root: no volume has that label or ID -- skipped.",
            "Fstab: 0 mount(s) from /etc/fstab.",
        ])
        check("...and /root is an empty directory on the system volume", s3.run("ls /root"), "ls /root\n")
    finally:
        s3.close()

    # ================================================================= a fourth, with the disk back: everything, in order
    s4 = boot_again(ctx, "fourth", with_home=True)
    try:
        check("boot 4: the file, with everything appended to it", s4.run("cat /root/note"), "cat /root/note\nkept\nmore\n")
    finally:
        s4.close()


def verify_disk(ctx):
    home = ctx.extra_imgs[0]
    fsck = subprocess.run(["fsck.fat", "-n", home], capture_output=True, text=True)
    ctx.check("disk (HOME): fsck.fat -n is clean", (fsck.returncode, fsck.stdout + fsck.stderr) if fsck.returncode else 0, 0)
    out = os.path.join(ctx.workdir, "note.out")
    subprocess.run(["mcopy", "-i", home, "::/note", out], check=True)
    with open(out) as f:
        ctx.check("the host sees the file the guest wrote, across four boots", f.read(), "kept\nmore\n")
