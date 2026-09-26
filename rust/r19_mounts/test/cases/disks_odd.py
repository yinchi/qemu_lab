"""Last updated: Stage 19, Step 1.

Devices that are not FAT volumes, and an image with no `SYSTEM` label. The system image keeps its own label (`R12SH`, what
every image before Stage 19 has) and two extra disks are attached after it, so they get the lower slots (the later on the command line, the lower): a blank one
(device 0) and one of random bytes (device 1). Neither is a filesystem: each is listed and ignored, with no label and no
volume ID; and with no volume labelled `SYSTEM` the root is the first FAT volume -- the system image, though it is device 2
-- not simply device 0.
"""

import re

from harness import file_hash, lsblk_table

# Listed noise first: the disk last on the command line gets the lowest slot, so the blank one is device 0.
# The image as every stage before Stage 19 built it: a different label, not SYSTEM.
SYSTEM_LABEL = "R12SH"
EXTRA_DISKS = [{"kind": "noise", "size_kib": 2048}, {"kind": "blank", "size_kib": 512}]
ENVIRONMENT = "HOME=/\n"


def run(ctx):
    s, check = ctx.s, ctx.check
    boot = s.log()
    check("boot: three devices", "Block devices: 3\n" in boot, True)
    lines = re.findall(r"^  (\d): (.*)$", boot, re.MULTILINE)
    check("boot: the two that are not FAT are ignored", [(n, text) for n, text in lines if "ignored" in text],
          [("0", "512 KiB  not a FAT volume -- ignored"), ("1", "  2 MiB  not a FAT volume -- ignored")])
    check("boot: the root is the first FAT volume, and says why",
          [text for n, text in lines if "(root" in text],
          [" 64 MiB  R12SH        0000-0000  (root: no SYSTEM volume, using the first FAT volume)"])
    rows = lsblk_table(s.run("lsblk"))
    check("lsblk: all three, in device order",
          [(r["NAME"], r["SIZE"], r["LABEL"], r["UUID"], r["MOUNTPOINT"]) for r in rows],
          [("vda", "512K", "", "", ""), ("vdb", "2M", "", "", ""), ("vdc", "64M", "R12SH", "0000-0000", "/")])
    check("ls /: the system volume", "bin" in s.run("ls /").split("\n"), True)
    check("the shell is alive", s.run("echo ok"), "echo ok\nok\n")


def verify_disk(ctx):
    for path, before in zip(ctx.extra_imgs, ctx.extra_hashes):
        ctx.check("the extra disk is byte-for-byte as it was", file_hash(path) == before, True)
