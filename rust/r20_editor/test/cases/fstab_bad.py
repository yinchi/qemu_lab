"""Last updated: Stage 19, Step 3.

A `/etc/fstab` full of lines that are not usable, and two that are (through a `..` and a trailing slash, which are
normalized like any path): every bad line is one note with its line number and never fatal; a line for `/` is only
checked -- the root is chosen at boot, so one naming another volume is noted and ignored, and one naming the root volume
is silent.
"""

ENVIRONMENT = "HOME=/\n"
FSTAB = """LABEL=SYSTEM / vfat defaults
LABEL=HOME / vfat defaults
label=HOME /root vfat defaults
LABEL=HOME root vfat defaults
LABEL=HOME /root ext4 defaults
LABEL=HOME /root vfat ro
LABEL=HOME /root vfat sync
LABEL=HOME /root
LABEL=HOME /root vfat defaults 0 1 2
LABEL=HOME /tests/../root vfat defaults
UUID=0BAD-F00D /fonts/ vfat defaults
"""
EXTRA_DISKS = [
    {"label": "HOME", "volume_id": "5E6F7A8B", "size_kib": 2048},
    {"label": "DATA", "volume_id": "0BADF00D", "size_kib": 2048},
]


def run(ctx):
    s, check = ctx.s, ctx.check
    boot = s.log()
    notes = [l for l in boot.split("\n") if l.startswith("Fstab:")]
    check("boot: one note per bad line, then the root line, then the count", notes, [
        "Fstab: /etc/fstab: line 3: source must be LABEL=name or UUID=XXXX-XXXX -- ignored.",
        "Fstab: /etc/fstab: line 4: mount point must be an absolute path -- ignored.",
        "Fstab: /etc/fstab: line 5: unsupported filesystem type (only vfat) -- ignored.",
        "Fstab: /etc/fstab: line 6: read-only mounts are not supported -- ignored.",
        "Fstab: /etc/fstab: line 7: unsupported option -- ignored.",
        "Fstab: /etc/fstab: line 8: expected <source> <mount point> <type> <options> -- ignored.",
        "Fstab: /etc/fstab: line 9: too many fields -- ignored.",
        "Fstab: /etc/fstab: line 2: LABEL=HOME on /: the root is chosen at boot (the volume labelled SYSTEM), not from this file -- ignored.",
        "Fstab: 2 mount(s) from /etc/fstab.",
    ])
    check("the two good lines were mounted, at normalized points", s.run("mount"),
          "mount\nLABEL=DATA on /fonts type vfat\nLABEL=HOME on /root type vfat\nLABEL=SYSTEM on / type vfat\n")
    check("the root is still the system volume", "bin" in s.run("ls /").split("\n"), True)
    check("the shell is alive", s.run("echo ok"), "echo ok\nok\n")
