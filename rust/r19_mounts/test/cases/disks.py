"""Last updated: Stage 19, Step 1.

More than one block device. A second disk (label `HOME`, volume ID `5E6F-7A8B`) is attached after the system image, which
is the image `just disk` builds, labelled `SYSTEM` (ID `0000-0000`): the kernel finds both, tells them apart by what their boot sectors say, chooses
the `SYSTEM` one as the root, and lists both -- on the serial log at boot and through `lsblk` (the `blkinfo` syscall).
The second disk is found, not read as a filesystem: nothing of it shows in the file tree, and nothing writes to it.

QEMU gives the virtio-mmio slots to devices in the opposite order to the command line; `disks_first` attaches the
extra disk the other way round, so that both orders are exercised.
"""

import re

from harness import lsblk_table

EXTRA_DISKS = [{"label": "HOME", "volume_id": "5E6F7A8B", "size_kib": 1024}]
ENVIRONMENT = "HOME=/\n"


def table_lines(boot):
    """The device lines of the boot log's `Block devices:` table."""
    return re.findall(r"^  \d: .*$", boot, re.MULTILINE)


def run(ctx):
    s, check = ctx.s, ctx.check
    boot = s.log()

    # --- the serial log: what was found, and which one is the root ---
    check("boot: two devices", "Block devices: 2\n" in boot, True)
    lines = table_lines(boot)
    check("boot: the SYSTEM volume is the root", sorted(re.sub(r"^  \d: ", "", l) for l in lines),
          ["  1 MiB  HOME         5E6F-7A8B", " 64 MiB  SYSTEM       0000-0000  (root)"])
    check("boot: the root was chosen by its label", "no SYSTEM volume" in boot, False)
    check("boot: the filesystem is mounted after the table", boot.index("Block devices") < boot.index("FAT filesystem mounted."), True)

    # --- lsblk ---
    rows = lsblk_table(s.run("lsblk"))
    by_label = {r["LABEL"]: r for r in rows}
    check("lsblk: both devices", sorted(by_label), ["HOME", "SYSTEM"])
    check("lsblk: SYSTEM", (by_label["SYSTEM"]["SIZE"], by_label["SYSTEM"]["UUID"], by_label["SYSTEM"]["MOUNTPOINT"]), ("64M", "0000-0000", "/"))
    check("lsblk: HOME", (by_label["HOME"]["SIZE"], by_label["HOME"]["UUID"], by_label["HOME"]["MOUNTPOINT"]), ("1M", "5E6F-7A8B", ""))
    # Precondition of `disks_first`: QEMU numbers the slots against the command line, so a disk attached after the
    # system image is device 0, and `lsblk` lists devices in device order. If this ever fails, the two groups no longer
    # cover both orders.
    check("the extra disk attached after the system image is listed first", [r["LABEL"] for r in rows], ["HOME", "SYSTEM"])
    check("lsblk: the whole table", s.run("lsblk"), "lsblk\nSIZE  LABEL   UUID       MOUNTPOINT\n1M    HOME    5E6F-7A8B\n64M   SYSTEM  0000-0000  /\n")
    check("lsblk -b: sizes in bytes", [(r["LABEL"], r["SIZE"]) for r in sorted(lsblk_table(s.run("lsblk -b")), key=lambda r: r["LABEL"])],
          [("HOME", "1048576"), ("SYSTEM", "67108864")])
    check("lsblk --help", s.run("lsblk --help"), "lsblk --help\nusage: lsblk [-b]\n  -b  print sizes in bytes instead of `ls -h` units\n")
    check("lsblk: an unknown option", s.run_status("lsblk -x"),
          ("lsblk -x\nlsblk: invalid option -- 'x'\nTry 'lsblk --help' for more information.\n", 1))

    # --- the second disk is not part of the file tree ---
    listing = s.run("ls /").split("\n")
    check("ls /: the system volume, no trace of the other", ("bin" in listing, "HOME" in listing, "home" in listing), (True, False, False))
    check("the shell is alive", s.run("echo ok"), "echo ok\nok\n")


def verify_disk(ctx):
    from harness import file_hash
    for path, before in zip(ctx.extra_imgs, ctx.extra_hashes):
        ctx.check("the extra disk is byte-for-byte as it was", file_hash(path) == before, True)
