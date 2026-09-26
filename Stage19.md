# Stage 19: persistent storage -- a second disk, mounts, `/etc/fstab` -- `r19_mounts` (plan)

`ROADMAP.md` carries the summary of this stage; this file is the plan: the decisions, and the steps in the order they are built and committed. It is updated as each step lands (an "As built" note per step, as `Stage17.md` and `Stage18.md` do). Unlike Stage 18 this stage changes the kernel (block driver, filesystem layer, syscalls) and the shell (start-up); the programs are a new tier `user/progs_r19` holding `mv`, `lsblk`, `mount` and `umount` -- the latter three thin wrappers over new syscalls, as on Linux (not builtins: they change no shell state).

| Step | What | Status |
|---|---|---|
| R | Roadmap: the mounts / `fstab` design written into the Stage 19 section (labels `SYSTEM`, `HOME`) | done |
| 0 | Plain copy of `r18_utils` as `r19_mounts` | done |
| 1 | Block driver: find every virtio-blk device, one `BlkIo` each; probe each boot sector for label and volume ID; choose the root by label; `blkinfo` syscall and `lsblk`; the host side (`SYSTEM` label, `home.img`, `just home-disk`/`home-reset`, `disk-home-seed/`) so `just run` shows both disks | done |
| 2 | Mount table and path resolution; open files remember their volume; `blkinfo` gains the mount point | |
| 3 | Pure `fstab` parser (host-tested) and the start-up order: `/etc/fstab`, `/etc/environment`, `$HOME`, `~/.profile`, prompt | |
| 4 | `EXDEV` from a cross-volume `rename`; the tier `progs_r19` with `mv` falling back to copy-then-remove | |
| 5 | `mount` and `umount` syscalls and programs (`EBUSY`); `mount` alone lists the table | |
| 6 | *(folded into Step 1: the host side -- `SYSTEM` label, `home.img` recipes, `disk-home-seed/`)* | done |
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
- **`lsblk`, `mount`, `umount` are programs over syscalls, not builtins** (decided after listing what the shell has: `cd`, `source`/`.`, `sh`, `export`, `unset` are builtins because they change the shell's own state; these change none). One syscall per Step where needed: `blkinfo` in Step 1, `mount`/`umount` in Step 5.
- **Cross-volume `rename` is `EXDEV`;** `mv` (in the new tier, extending Stage 18's) copies and removes for a file, and reports a directory it cannot move.
- **Host side:** `just run` rebuilds the system image every time and attaches `home.img`, which is created once and never rebuilt; the host must not touch it while QEMU runs.
- **Documentation stays stage-agnostic** (`<stage>` for what changes per stage; name the stage for a change attributable to one): new rows say "From Stage 19".

## Not doing
Other filesystem types, hot-plug, `/dev`, read-only mounts and further options, bind mounts, `root=` on a command line, moving a directory across volumes, naming a disk by its virtio serial.

## Step 0 -- plain copy `rust/r19_mounts`
As Stage 18's Step 0: `rsync` `r18_utils` -> `r19_mounts` (excluding `target`, `disk.img`, `*.elf`, `__pycache__` and the generated files), rename in `Cargo.toml`, `Cargo.lock`, `justfile` `BIN`, `hosttests/src/lib.rs`, `test/check_docs.py`, `test/run_tests.py` (docstring and temp prefix), `test/README.md`, `disk/tests/notes.txt`, the stage's `.gitignore`. `just test` = 1468 checks and 260 host tests unchanged.

**As built (Step 0).** Copied the 156 tracked files of `r18_utils` (so no build artifacts or generated fixtures came along) and renamed in `Cargo.toml`, `Cargo.lock`, `justfile` `BIN`, `hosttests/src/lib.rs`, `test/check_docs.py`, `test/run_tests.py` (docstring and the `r19-` temp prefix), `test/README.md`, `disk/tests/notes.txt` and `disk/fonts/NOTICE`. `just test` = 1468 checks, 260 host tests, `just lint` and `just check-docs` (35 programs) clean: identical to Stage 18.

## Step 1 -- every block device, identified by its boot sector
Nothing user-visible changes except the serial log and which disk is the root; the second disk is *found and named*, never read as a filesystem. (This moves "root = the volume labelled `SYSTEM`, else the first device" here from Step 2: without it, a second disk in a lower virtio-mmio slot than the system image would be taken for the root, so the step could not be tested with a second disk attached.)

- **Driver (`drivers/virtio/blk.rs`, `drivers/virtio/mod.rs`).** `Blk::find` (first device) becomes `Blk::find_all`, returning every virtio-blk device with its SPI, in device-tree order, up to a constant `MAX_BLK` (extra ones ignored, with a serial note). `find_mmio_transport` stays as the per-slot helper the GPU and keyboard use; the block search walks the slots itself.
- **Globals (`platform/globals.rs`, `main.rs`).** `BLK: Option<Blk>` / `BLK_SPI` become arrays of `MAX_BLK`; every found SPI is enabled, and `irq_handler` acknowledges whichever device's line fired (a short loop). The populate-before-enable ordering that `main.rs` documents holds per device.
- **`BlkIo` (`fs/blkio.rs`).** Gains a device index (`BlkIo::new(dev)`); its `Read`/`Write`/`Seek` go to `BLK[dev]`. Still one `VOL` and one filesystem: only the root is opened.
- **Probe (new `fs/bootsector.rs`).** A pure, host-tested parser (`no_std`, pulled into `hosttests` like `environment.rs`) turning sector 0 into `{label, volume_id}` or "not FAT": `0x55AA` signature, sane bytes-per-sector, the extended-boot-signature byte (`0x29`), FAT32 told from FAT12/16 by `FATSz16 == 0` and `RootEntCnt == 0`, and so the label (11 bytes, trailing spaces trimmed; `NO NAME` counts as no label, as `blkid` does) and volume ID at their offsets. Own parser rather than opening a `hadris` `FatVolume` per device: a probe must not need a valid filesystem, and a `FatVolume` would also keep a second `BlkIo` alive for nothing. After `DAIF` is cleared (the read is interrupt-driven) each device's sector 0 is read once into a small table `(index, capacity, label, volume_id)`.
- **Root choice (`main.rs`).** The volume labelled `SYSTEM` (the first, if several), else device 0; `BlkIo::new(root)` then mounts as today. Serial log, before `FAT filesystem mounted.`:
  ```
  Block devices: 2
    0: 16 MiB  SYSTEM  1A2B-3C4D  (root)
    1:  1 MiB  HOME    5E6F-7A8B
  ```
  and for a skipped device `1: 1 MiB  not a FAT volume -- ignored`. With one unlabelled device (every image before Stage 19's Step 6) the line says `(root, no SYSTEM volume: using the first device)`.
- **Syscall and program.** `blkinfo(index, *mut BlkInfo) -> 0 | -ENODEV` past the last device (additive to `abi`, wrapper in `userlib`, a row in `docs/syscalls.md`); `BlkInfo` is a fixed `#[repr(C)]` struct: capacity in bytes, `flags` (bit 0: FAT volume, bit 1: is the root), 11-byte label plus length, volume ID. New tier `user/progs_r19` (copy of `progs_r18`'s crate scaffolding) with `lsblk`: `NAME SIZE LABEL UUID MOUNTPOINT`, names `vda`, `vdb`, ... by device order, sizes in the `ls -h` style (`16M`), a non-FAT device shown with empty label/UUID, MOUNTPOINT `/` for the root and empty otherwise until Step 2.
- **Tests (new group `disks`, then more as needed).** The harness learns extra images: a module attribute (`EXTRA_DISKS`, in the manner of `ENVIRONMENT`) making each extra image from a small description -- label and volume ID for a FAT one made by `mkfs.fat -n -i`, or "blank" / "not FAT" -- and attaching it *before* or *after* the system image on the QEMU command line (a second attribute), since which one gets the lower slot is the very thing that must not matter. Cases:
  - system + `HOME` attached after, then again attached before: the serial log and `lsblk` list both volumes with the right label and ID, the `SYSTEM` one is the root (`/`), and both boots reach the same prompt with the same `ls /`;
  - a blank disk and a non-FAT (random bytes) disk: noted, ignored, boot unaffected; two `HOME` volumes: both listed (the duplicate matters only when mounting, Step 3);
  - the system image labelled `SYSTEM` is chosen even when the *other* disk is in slot 0;
  - `verify_disk` checks the extra image is **byte-identical** afterwards (nothing wrote to it) and the system image is `fsck.fat -n` clean.
  The existing groups (one disk) boot the image `just disk` builds, now labelled `SYSTEM`; `disks_odd` relabels it `R12SH` to stand for every image before Stage 19.
- **Docs.** `docs/` kernel/filesystem pages: one paragraph on multiple devices and probing (stage-agnostic wording, "From Stage 19"), the `tests.md` row.
- **Not here:** opening a second `FatVolume`, mount table, paths, `fstab`, the guest reading anything from the extra disk. `just run` still attaches one disk.

Open point for the step: the virtio-mmio slot order QEMU produces for two `-device virtio-blk-device` lines is checked empirically first (the plan does not assume it); `MAX_BLK` = 4 unless that shows a reason.

**As built (Step 1).**
- **Driver and IRQ.** `Blk::find_all` (over a new `mmio_transports` iterator that `find_mmio_transport` now uses too) fills `BLK`/`BLK_SPI` arrays of `MAX_BLK` = 4 and `BLK_COUNT`; `blk::get(dev)` reaches one; a device past the fourth is left alone with a serial note. `irq_handler` finds which line fired with a short loop. QEMU's slot order was checked first, as planned: **it is the reverse of the command line** (the device listed last is device 0), so the tests attach extra disks both ways.
- **`fs/bootsector.rs`** (pure; 11 host tests): `parse` (FAT12/16 and FAT32 layouts, signature byte `0x29`, or `0x28` for an ID and no label; anything else is not FAT), `label_is` (ASCII case-insensitive, an empty name matches nothing), `VolumeId`'s `XXXX-XXXX` display, and `pick_root`: the first `SYSTEM` volume, else the first FAT volume (not merely device 0: a blank device 0 must not become the root).
- **`fs/devices.rs`:** `probe` reads sector 0 of each device after IRQs are unmasked, logs the table (`Block devices: N`, one line each, `(root)` or the reason it is the root without the label) and stores it; `blkinfo(index)` reads it. `BlkIo::new(dev)` takes the device index; still one `VOL`.
- **Syscall 1000, `blkinfo(index, out)`** (not a Linux number; `abi::blk` holds the 32-byte record with `encode`/`decode` and its own tests; `ENODEV` added to `abi::errno`, `userlib::blkinfo`). **`lsblk [-b]`** in the new tier `user/progs_r19` (`NAME SIZE LABEL UUID MOUNTPOINT`; `table.rs` holds the column layout and `vda`, `vdb`, ... names, 5 host tests); `MOUNTPOINT` is `/` for the root, empty otherwise until Step 2.
- **Harness:** `EXTRA_DISKS` (FAT with label/ID, `blank`, `noise`; `before` picks the command-line side), `SYSTEM_LABEL`/`SYSTEM_ID` (`mlabel`), `Context.extra_imgs`/`extra_hashes`, `lsblk_table`. Four groups: `disks` (extra after the system image), `disks_first` (before), `disks_odd` (blank + noise disks, system image still `R12SH`: the root is the first FAT volume although it is device 2), `disks_many` (four extras, the fifth device ignored, duplicate labels told apart by ID); each checks the extra disks are byte-identical afterwards.
- **Host side, folded in from Step 6** (so the second disk is there to see in `just run`, still unmountable): `just disk` now labels the system image `SYSTEM` (volume serial still fixed, `0000-0000`). **`just home-disk`** creates `home.img` (32 MiB FAT16, label `HOME`, a random volume ID, gitignored) with `folder_to_img.sh` **only if it is missing** and says so when it is not; **`just home-reset`** deletes and recreates it; `just run` depends on `home-disk`, attaches `home.img` and `disk.img`, listing `home.img` first so the system disk is `vda` (QEMU numbers against the command line). The seed folder is **`disk-home-seed/`** -- named so because it is only the starting content of a new `home.img`, never kept in step with it -- and holds just a `README.txt` for now: `.profile` and `utf8-demo.txt` move there in Step 3, when `/root` becomes the mount (moving them now would leave the interactive shell with no profile, the disk being unmountable). Checked by booting `just run`'s exact QEMU line headless: `0: 64 MiB SYSTEM 0000-0000 (root)`, `1: 32 MiB HOME <id>`.
- `just test`: **1513 checks**, 279 host tests in the kernel crate plus 20 in `abi`, `just lint` and `just check-docs` (36 programs, 18 syscalls) clean. Docs: `syscalls.md`, `progs.md`, `filesystem.md`, `virtio.md`, `tests.md`.

## Steps 2-8
Detailed when reached; the scope of each is the row above and the ROADMAP section's feature list. Open point: where the volume handle lives in the open-file table (Step 2).
