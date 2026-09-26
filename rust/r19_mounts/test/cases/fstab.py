"""Last updated: Stage 19, Step 3.

`/etc/fstab`: the volumes the init shell mounts at start-up, in file order, before it reads `/etc/environment`, enters
`$HOME` and runs `~/.profile` -- so the home directory can be on a mounted volume, and its profile is the one on that
volume. Four extra disks-worth of cases in one boot: `HOME` (with a `.profile` and a file of its own) mounted on `/root`,
`DATA` on `/fonts` (with the `fail` option, which is the default), a `noauto` line that is left for `mount` by hand, a `nofail` line for a volume
that is not there (no note), and lines that cannot be mounted for the reasons `mount` refuses by hand (no such volume,
volume already mounted, mount point in use, mount point missing), each one a note on the serial log and nothing more.
"""

import re

from harness import lsblk_table

ENVIRONMENT = "HOME=/root\n"
FSTAB = """# the home disk
LABEL=HOME /root vfat defaults
UUID=0BAD-F00D /fonts vfat defaults,fail
UUID=1111-2222 /tests vfat noauto
LABEL=GONE /bin vfat defaults,nofail
LABEL=NOPE /bin vfat defaults
LABEL=HOME /tmp vfat defaults
LABEL=SPARE /root vfat defaults
LABEL=SPARE /nosuchdir vfat defaults
"""
EXTRA_DISKS = [
    {"label": "HOME", "volume_id": "5E6F7A8B", "size_kib": 2048,
     "files": {"/.profile": "export FROM_HOME=yes\n", "/hello.txt": "hello from the home disk\n"}},
    {"label": "DATA", "volume_id": "0BADF00D", "size_kib": 2048},
    {"label": "SPARE", "volume_id": "11112222", "size_kib": 2048},
]


def run(ctx):
    s, check = ctx.s, ctx.check
    boot = s.log()

    # --- the serial log: one note per line that could not be mounted, then the count; all before the environment ---
    notes = [l for l in boot.split("\n") if l.startswith("Fstab:")]
    check("boot: what was mounted and what was not", notes, [
        "Fstab: /etc/fstab: line 6: LABEL=NOPE on /bin: no volume has that label or ID -- skipped.",
        "Fstab: /etc/fstab: line 7: LABEL=HOME on /tmp: Device or resource busy -- skipped.",
        "Fstab: /etc/fstab: line 8: LABEL=SPARE on /root: Device or resource busy -- skipped.",
        "Fstab: /etc/fstab: line 9: LABEL=SPARE on /nosuchdir: No such file or directory -- skipped.",
        "Fstab: 2 mount(s) from /etc/fstab.",
    ])
    check("boot: fstab comes before the environment", boot.index("Fstab: 2 mount(s)") < boot.index("Environment:"), True)

    # --- what is mounted: file order, and only those ---
    check("mount", s.run("mount"), "mount\nLABEL=DATA on /fonts type vfat\nLABEL=HOME on /root type vfat\nLABEL=SYSTEM on / type vfat\n")
    check("lsblk: the noauto volume is not mounted", [r["MOUNTPOINT"] for r in lsblk_table(s.run("lsblk"))], ["", "/fonts", "/root", "/"])

    # --- the shell entered $HOME on the mounted volume, and ran the profile that is there ---
    check("the shell starts in /root", s.run("pwd"), "pwd\n/root\n")
    check("...which is the HOME volume", s.run("ls"), "ls\nhello.txt\n")
    check("the profile on the HOME volume ran", s.run("printenv FROM_HOME"), "printenv FROM_HOME\nyes\n")
    check("a relative path is on it", s.run("cat hello.txt"), "cat hello.txt\nhello from the home disk\n")
    check("the mount hides the system volume's /root", s.run_status("cat /root/nothing"), ("cat /root/nothing\ncat: /root/nothing: No such file or directory\n", 1))

    # --- DATA is on /fonts; the noauto volume waited to be mounted by hand ---
    check("ls /fonts: DATA, not the system volume's", s.run("ls /fonts"), "ls /fonts\n")
    check("/tests is the system volume's", "hello.txt" in s.run("ls /tests").split("\n"), True)
    check("mount the noauto line by hand", s.run_status("mount UUID=1111-2222 /tests"), ("mount UUID=1111-2222 /tests\n", 0))
    check("ls /tests: now the SPARE volume", s.run("ls /tests"), "ls /tests\n")
    check("the shell is alive", s.run("echo ok"), "echo ok\nok\n")


def verify_disk(ctx):
    import subprocess
    for label, img in zip(("HOME", "DATA", "SPARE"), ctx.extra_imgs):
        fsck = subprocess.run(["fsck.fat", "-n", img], capture_output=True, text=True)
        ctx.check(f"disk ({label}): fsck.fat -n is clean", (fsck.returncode, fsck.stdout + fsck.stderr) if fsck.returncode else 0, 0)
