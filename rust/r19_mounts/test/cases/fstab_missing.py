"""Last updated: Stage 19, Step 3.

No `/etc/fstab` at all: one note, nothing mounted, and the shell starts as usual.
"""

ENVIRONMENT = "HOME=/\n"
FSTAB = None  # the harness removes the file from this group's image
EXTRA_DISKS = [{"label": "HOME", "volume_id": "5E6F7A8B", "size_kib": 1024}]


def run(ctx):
    s, check = ctx.s, ctx.check
    notes = [l for l in s.log().split("\n") if l.startswith("Fstab:")]
    check("boot: the missing file is one note", notes, ["Fstab: /etc/fstab: No such file or directory -- nothing to mount."])
    check("mount: only the root", s.run("mount"), "mount\nLABEL=SYSTEM on / type vfat\n")
    check("the shell is alive", s.run("echo ok"), "echo ok\nok\n")
