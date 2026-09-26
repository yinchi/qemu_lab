"""Last updated: Stage 19, Step 1.

More disks than the kernel drives (four): four extra disks, all labelled `HOME` (with different volume IDs -- two volumes
may share a label, and are still told apart by their IDs), are attached *before* the system image, which is therefore
device 0, and the fifth device (the last extra disk) is left alone, with a note on the serial log. The others are listed.
"""

import re

from harness import file_hash, lsblk_table

EXTRA_DISKS = [{"label": "HOME", "volume_id": f"0000000{n}", "size_kib": 1024, "before": True} for n in range(1, 5)]
ENVIRONMENT = "HOME=/\n"


def run(ctx):
    s, check = ctx.s, ctx.check
    boot = s.log()
    check("boot: four devices driven", "Block devices: 4\n" in boot, True)
    check("boot: the fifth is reported", "Ignoring 1 block device(s) past the first 4.\n" in boot, True)
    rows = lsblk_table(s.run("lsblk"))
    check("lsblk: the system volume and three HOME volumes",
          sorted((r["LABEL"], r["MOUNTPOINT"]) for r in rows), [("HOME", ""), ("HOME", ""), ("HOME", ""), ("SYSTEM", "/")])
    check("lsblk: the volumes are told apart by their IDs", len({r["UUID"] for r in rows}), 4)
    check("lsblk: the system volume is device 0", [r["LABEL"] for r in rows][:1], ["SYSTEM"])
    check("the shell is alive", s.run("echo ok"), "echo ok\nok\n")


def verify_disk(ctx):
    for path, before in zip(ctx.extra_imgs, ctx.extra_hashes):
        ctx.check("the extra disk is byte-for-byte as it was", file_hash(path) == before, True)
