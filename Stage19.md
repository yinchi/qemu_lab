# Stage 19: persistent storage -- a second disk, mounts, `/etc/fstab` -- `r19_mounts` (plan)

`ROADMAP.md` carries the summary of this stage; this file is the plan: the decisions, and the steps in the order they are built and committed. It is updated as each step lands (an "As built" note per step, as `Stage17.md` and `Stage18.md` do). Unlike Stage 18 this stage changes the kernel (block driver, filesystem layer, syscalls) and the shell (start-up, two builtins); the programs change only in a new tier `user/progs_r19` holding `mv`.

| Step | What | Status |
|---|---|---|
| R | Roadmap: the mounts / `fstab` design written into the Stage 19 section (labels `SYSTEM`, `HOME`) | done |
| 0 | Plain copy of `r18_utils` as `r19_mounts` | done |
| 1 | Block driver: find every virtio-blk device, one `BlkIo` each; probe each boot sector for label and volume ID | |
| 2 | Mount table and path resolution; root = volume labelled `SYSTEM`, else the first device; open files remember their volume | |
| 3 | Pure `fstab` parser (host-tested) and the start-up order: `/etc/fstab`, `/etc/environment`, `$HOME`, `~/.profile`, prompt | |
| 4 | `EXDEV` from a cross-volume `rename`; the tier `progs_r19` with `mv` falling back to copy-then-remove | |
| 5 | `mount` and `umount` builtins (`EBUSY`) | |
| 6 | Host side: `SYSTEM` label, `home.img` recipes (`just home-disk`, `just home-reset`), `disk-home/` seed, distinct volume ID | |
| 7 | Tests: second image per group, `FSTAB` attribute, `verify_disk`/`fsck.fat` on both | |
| 8 | Docs, roadmap "As built", regression sweep | |

Steps 1-2 and 6 depend on one another for a full boot: Step 1 must keep the one-disk boot working (nothing mounts a second volume yet), and Step 6's recipes arrive before Step 3's `fstab` can be tried by hand, so Step 3 is first tested with a hand-made second image.

## Decisions (settled)
- **Two labels, no `/home/root`.** The system image is labelled `SYSTEM` and is `/`; the persistent disk is labelled `HOME` and is mounted at `/root`, the directory Stage 17 already made the shell's `$HOME` (there is no login or user system, so `/home/<user>` would only add a level; `/home/root` is not the Linux convention either -- root's home is `/root`). Labels rather than `ROOT`/`DATA`, so that "ROOT on `/`, DATA on `/root`" is never read backwards.
- **The kernel mounts the root; the shell mounts the rest.** Linux's `root=` versus `mount -a`. The root is chosen before any file can be read: the volume labelled `SYSTEM`, else the first device, so every earlier image (label `R12SH`) still boots. A `root=LABEL=` on a kernel command line (QEMU `-append` becomes the DTB's `/chosen/bootargs`) is possible later and not needed while the label rule works.
- **`/etc/fstab`:** `<source> <mount point> <type> <options>`, whitespace-separated, `#` comments, `LABEL=`/`UUID=` sources (UUID is the FAT volume ID, `XXXX-XXXX`), type `vfat` or `fat`, options `defaults`, `noauto`, `nofail`, `noatime` accepted, `ro` refused with a note, `dump`/`pass` ignored. Mounted in file order. The image ships one line: `LABEL=HOME /root vfat defaults`. A `/` line is accepted but informational (checked against the volume that is the root; mismatch is a note); none is shipped.
- **Every problem is a serial-log note, never fatal:** missing file, bad line, no matching device, duplicate label (first wins), mount point not a directory. With no home disk, `/root` is an empty directory on the system volume and the shell starts there.
- **Identification by what is on the disk:** FAT volume label and volume ID from the boot sector (via `hadris`: `volume_label()`, `volume_id()`; `mlabel` changes both the boot sector and the root-directory label). Not by MMIO slot (QEMU's order is not the command-line order), not by the virtio serial, no partition table so no `PARTUUID`. All images so far have volume ID `0000-0000`; `just home-disk` stamps a distinct one.
- **Path resolution:** longest matching mount-point prefix on the normalized absolute path; the remainder goes to that volume. A mount point must exist as a directory on the volume beneath it; what it hid stays hidden while mounted.
- **Cross-volume `rename` is `EXDEV`;** `mv` (in the new tier, extending Stage 18's) copies and removes for a file, and reports a directory it cannot move.
- **Host side:** `just run` rebuilds the system image every time and attaches `home.img`, which is created once and never rebuilt; the host must not touch it while QEMU runs.
- **Documentation stays stage-agnostic** (`<stage>` for what changes per stage; name the stage for a change attributable to one): new rows say "From Stage 19".

## Not doing
Other filesystem types, hot-plug, `/dev`, read-only mounts and further options, bind mounts, `root=` on a command line, moving a directory across volumes, naming a disk by its virtio serial.

## Step 0 -- plain copy `rust/r19_mounts`
As Stage 18's Step 0: `rsync` `r18_utils` -> `r19_mounts` (excluding `target`, `disk.img`, `*.elf`, `__pycache__` and the generated files), rename in `Cargo.toml`, `Cargo.lock`, `justfile` `BIN`, `hosttests/src/lib.rs`, `test/check_docs.py`, `test/run_tests.py` (docstring and temp prefix), `test/README.md`, `disk/tests/notes.txt`, the stage's `.gitignore`. `just test` = 1468 checks and 260 host tests unchanged.

**As built (Step 0).** Copied the 156 tracked files of `r18_utils` (so no build artifacts or generated fixtures came along) and renamed in `Cargo.toml`, `Cargo.lock`, `justfile` `BIN`, `hosttests/src/lib.rs`, `test/check_docs.py`, `test/run_tests.py` (docstring and the `r19-` temp prefix), `test/README.md`, `disk/tests/notes.txt` and `disk/fonts/NOTICE`. `just test` = 1468 checks, 260 host tests, `just lint` and `just check-docs` (35 programs) clean: identical to Stage 18.

## Steps 1-8
Detailed when reached; the scope of each is the row above and the ROADMAP section's feature list. Open points to settle at the step: how interrupts are wired per device (Step 1: one PLIC/GIC line per MMIO slot, as today's single device), whether `BlkIo` stays a global or becomes a table entry (Step 1), and where the volume handle lives in the open-file table (Step 2).
