"""Last updated: Stage 19, Step 1.

`disks`, with the second disk attached *before* the system image on the QEMU command line, so it gets the higher
virtio-mmio slot and the system image is device 0 (and the other way round from `disks`): the root is still the volume
labelled `SYSTEM`, wherever it is, and both disks are listed as before.
"""

import re

from harness import file_hash, lsblk_table

EXTRA_DISKS = [{"label": "HOME", "volume_id": "5E6F7A8B", "size_kib": 1024, "before": True}]
ENVIRONMENT = "HOME=/\n"


def run(ctx):
    s, check = ctx.s, ctx.check
    boot = s.log()
    check("boot: two devices", "Block devices: 2\n" in boot, True)
    check("boot: the SYSTEM volume is the root", sorted(re.sub(r"^  \d: ", "", l) for l in re.findall(r"^  \d: .*$", boot, re.MULTILINE)),
          ["  1 MiB  HOME         5E6F-7A8B", " 64 MiB  SYSTEM       0000-0000  (root)"])
    rows = lsblk_table(s.run("lsblk"))
    by_label = {r["LABEL"]: r for r in rows}
    check("lsblk: SYSTEM is the root", (by_label["SYSTEM"]["UUID"], by_label["SYSTEM"]["MOUNTPOINT"]), ("0000-0000", "/"))
    check("lsblk: HOME is not mounted", (by_label["HOME"]["UUID"], by_label["HOME"]["MOUNTPOINT"]), ("5E6F-7A8B", ""))
    check("the extra disk attached before the system image is listed last", [r["LABEL"] for r in rows], ["SYSTEM", "HOME"])
    check("ls /: the system volume", "bin" in s.run("ls /").split("\n"), True)
    check("the shell is alive", s.run("echo ok"), "echo ok\nok\n")


def verify_disk(ctx):
    for path, before in zip(ctx.extra_imgs, ctx.extra_hashes):
        ctx.check("the extra disk is byte-for-byte as it was", file_hash(path) == before, True)
