# The filesystem

Programs see a single FAT16 volume mounted at `/`. The kernel does not implement FAT itself: the
[`hadris-fat`](https://crates.io/crates/hadris-fat) crate does, and `rust/r12_shell/src/fs/` is the glue
between it, the block device below and the syscalls above.

```mermaid
flowchart TD
    prog["EL0 program<br/>open / read / write / getdents / ..."]
    fd["syscall/fd.rs<br/>fd numbers, the three standard streams"]
    files["fs/files.rs<br/>open-file table, path walk, mkdir / unlink / rename / chmod / stat"]
    path["fs/path.rs<br/>path arithmetic (pure)"]
    fat["hadris-fat<br/>FAT16: directories, clusters, long names"]
    blkio["fs/blkio.rs<br/>BlkIo: bytes over 512-byte sectors"]
    blk["drivers/virtio/blk.rs<br/>virtio-blk"]
    img["disk.img"]

    prog --> fd --> files
    files --> path
    files --> fat --> blkio --> blk --> img
```

## The volume

- **The image.** `just disk` builds a 64 MiB FAT16 image from the stage's `disk/` directory
  (`folder_to_img.sh`: `mkfs.fat` plus `mtools`), so `disk/bin/cat.exe` lands at `/bin/cat.exe`. The volume
  serial and file timestamps are pinned, so the same inputs give a byte-identical image. The layout the shell
  relies on is `/bin` (programs) and `/tmp` (pipeline temp files; the shell needs it to exist); the image also
  carries `/tests` (test programs and fixtures) and whatever else is in `disk/`.
- **Mounting.** `kernel_main` opens the volume once, over a `BlkIo` (below), into the static `VOL`, and it stays
  mounted for the kernel's whole life. Everything else re-derives directories and files from it on each lookup;
  nothing else is cached. There is one volume: no mount points, no other filesystems.
- **The executable bit.** FAT has no execute permission, so this project claims one of the attribute byte's
  unused bits, `ATTR_EXEC` (`0x40`), alongside the real FAT ones. At boot the kernel sets it on every file in
  `/bin`; nothing else is executable until `chmod +x`. See [`launching_programs.md`](launching_programs.md).

## Getting bytes to the disk: `BlkIo`

`fs/blkio.rs` presents the block device to `hadris-fat` as one flat, byte-addressable stream from byte 0 to
the end of the disk (`Read + Write + Seek`); `hadris-fat` decides for itself which byte ranges are the FAT
tables, directory entries or file data. The device only does whole 512-byte sectors, so every read fetches the
sector containing the position, and every write is a **read-modify-write** of a sector: read it, patch the
bytes, write it back. Nothing is buffered, so every write has reached the device when it returns (`flush` has
nothing to do). It goes through the shared `BLK` static, whose completion interrupt is handled in the IRQ
handler (see [`virtio.md`](virtio.md)).

## Paths

`fs/path.rs` (`abspath`) is pure string arithmetic, tested on the host. It turns what a user typed into the
absolute, normalized path the rest of the module resolves: relative paths start at the working directory,
`.` and empty components vanish, `..` removes the component before it (and stays at the root), and a trailing
`/` is ignored. It never touches the disk, so it never checks that anything exists.

- A component longer than `NAME_MAX` (255) or a path longer than `PATH_MAX` (4096) is `ENAMETOOLONG`; an empty
  path is `ENOENT`.
- `files.rs` then walks the components from the root, one directory at a time, matching each name **exactly
  and case-sensitively** &mdash; unlike FAT itself, which folds case, so `cat HELLO.TXT` does not find
  `hello.txt`. Long file names (up to 255 characters) work; the short 8.3 name every entry also has
  must still be unique in its directory.
- There are no symbolic links and no hard links, which is what makes purely lexical `..` correct. `.` and `..`
  entries in FAT directories are never listed.

## Open files

`syscall/fd.rs` owns the small file-descriptor numbers a program sees (0, 1 and 2 are the standard streams;
the rest refer to files). `fs/files.rs` owns what they refer to: a table of **13** slots (`MAX_OPEN_FILES`),
across everything open at once. Each slot is one of:

| Kind | Opened by | Used by |
|---|---|---|
| Reader | `open` for reading a file | `read` |
| Writer | `open` for writing a file | `write` |
| Directory | `open` for reading a *directory* | `getdents` |

`open` fails with `EMFILE` when all 13 are in use. The rules:

- **Reading.** A directory can be opened read-only, and its entries are **snapshotted** at that moment.
  `read` on a directory is `EISDIR`; `getdents` on an open file is `ENOTDIR`.
- **Writing.** Opening for write creates the file if it is missing and empties it, unless `append` (`O_WRONLY |
  O_APPEND`) starts at the current end. A directory is `EISDIR`; a read-only file is `EACCES`. `hadris-fat`
  refuses a second writer on the same file.
- **`close` is what commits a write.** A writer's final size reaches the directory entry only when it is closed
  (`FileWriter::finish`), which is why the shell closes each redirect target and pipe end before the next
  command reads it. There is no `seek`, and no separate truncate.
- **Ending a program.** When a program exits or faults, `close_all` closes whatever handles it left open, so a
  crashed program cannot leak them. Handles the *shell* opened for a redirection are marked and skipped: they
  outlive the one program that ran under them, and the shell closes them when the redirected command is done
  (see [`shell.md`](shell.md)).

## Directory listings

`getdents` fills the caller's buffer with fixed-size records, as many as fit:

| Bytes | Field |
|---|---|
| 0&ndash;3 | file size, `u32`, little-endian |
| 4 | attribute byte |
| 5 | name length |
| 6&ndash;260 | the name, NUL-padded to `NAME_MAX` (255) |

That is `DIRENT_SIZE` = 261 bytes per entry. Entries come in **on-disk order, not sorted**; the volume-label
pseudo-entry, `.` and `..` are skipped.

## Attributes and permissions

The attribute byte is FAT's own, plus the one claimed bit. The table lists the bits the kernel gives meaning to; the
raw byte is passed through to `getdents` and `stat`, so others (hidden `0x02`, system `0x04`, archive `0x20`, which
`mtools` sets on copied files) can appear too:

| Bit | Name | Meaning |
|---|---|---|
| `0x01` | `ATTR_READ_ONLY` | Refuses opening for write (`EACCES`) |
| `0x08` | `ATTR_VOLUME_LABEL` | The volume-label pseudo-entry; never listed |
| `0x10` | `ATTR_DIRECTORY` | A directory |
| `0x40` | `ATTR_EXEC` | Executable (not standard FAT) |

`chmod` may change only `ATTR_EXEC` and `ATTR_READ_ONLY`, since the other bits say what an entry *is*, not what
it permits; anything else is `EINVAL`. That is the whole permission model: there are no users, groups or read
bits. The read-only bit gates *opening for write* but deliberately not deleting or renaming: those are governed
by the containing directory alone, as for a privileged process on Unix.

## Other operations

| Operation | Behavior |
|---|---|
| `mkdir` | The parent must exist (`ENOENT`, or `ENOTDIR` if a component is a file) and the path must not (`EEXIST`); an invalid name is `EINVAL`. |
| `unlink` | Removes a file, or with `AT_REMOVEDIR` an empty directory (`EISDIR`/`ENOTDIR` if the kind doesn't match, `ENOTEMPTY` if it isn't empty). Paths are normalized first, so `.` and `..` name the current and parent directory; a path that normalizes to `/` is `EINVAL`. |
| `rename` | A literal rename or move within the volume: refuses if the destination already exists (`EEXIST`), and refuses moving a directory into itself or a descendant (`EINVAL`). `/` as either path is `EISDIR`. "Move into an existing directory" and "replace a file" are `mv`'s job, built in userspace on `stat` + `rename` + `unlink`. |
| `stat` | Every field a FAT entry stores: size, attributes, created/modified date-time, accessed date. Dates are FAT's packed encoding, not calendar values; the root has no entry of its own and reports size 0, directory, and the FAT epoch. |
| `chmod` | As above. |

Timestamps are what is stored: the fixed build-time stamp for files bundled into the image, and the FAT epoch
(1980-01-01) for anything the kernel itself creates or writes, since there is no real-time clock until Stage 14.

`mkdir`, `unlink` and `rename` map `hadris-fat`'s errors through one function (`map_fat_err`): `AlreadyExists` to
`EEXIST`, `DirectoryNotEmpty` to `ENOTEMPTY`, `InvalidPath`/`InvalidFilename` to `EINVAL`, `NoFreeSpace`/
`DirectoryFull` to `ENOSPC`, and everything else to `EIO`. Every other operation (`open`, including creating a
file, `write`, `close`, `chmod`) reports any failure as plain `EIO`, so a full disk during a write or a create
shows up as `EIO`, not `ENOSPC`.

## Not supported

Symbolic and hard links, permissions beyond the two bits above, `seek` on an open file, more than one volume,
and case-insensitive names.

## Where the code lives

| File | What it holds |
|---|---|
| `fs/blkio.rs` | `BlkIo`, and the `VOL` static (the mounted volume) |
| `fs/files.rs` | The open-file table and every operation above |
| `fs/path.rs` | `abspath` (pure, host-tested) |
| `fs/mod.rs` | `find_entry_checked` (look up one name in a directory) and `read_file_checked` (read a whole file into a `Vec`) |
| `syscall/fd.rs` | The fd table on top of the handles above; where `read`/`write`/`open` enter |
| `user/abi/src/fs.rs` | The shared constants: open flags, `NAME_MAX`/`PATH_MAX`, `DIRENT_SIZE`, the attribute bits |
