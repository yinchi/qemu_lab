# System calls

The interface between an EL0 program and the kernel. The definitions are in the shared `abi` crate
(`rust/user/abi/`), which both the kernel and `userlib` depend on, so the two sides cannot disagree about a
number or a constant. The kernel's side is `rust/r12_shell/src/syscall/`; programs normally reach it through
`userlib` (`rust/user/userlib/`) rather than issuing `svc` themselves.

## Calling convention

The standard AArch64 Linux one:

- The program executes `svc #0` with the syscall **number in `x8`** and up to six **arguments in `x0`&ndash;`x5`**
  (the kernel reads at most four today).
- The **result comes back in `x0`**. A non-negative value is a success (a count, an fd, or 0); **any negative
  value is an error, the negated Linux `errno`** (`-ENOENT` is `-2`). `abi::errno::errmsg` turns one into
  `strerror`'s wording, which is what every program prints after `name: path: `.
- An unassigned number returns `ENOSYS`. `exit` never returns.

The **numbers are Linux's real aarch64 values**, borrowed for familiarity only; the shapes are simplified:
a path is a `(pointer, length)` pair, not a NUL-terminated string, and there is no `dirfd` &mdash; a relative
path is resolved against the shell's working directory ([`shell.md`](shell.md)).

## Passing pointers safely

A user pointer is never trusted. Before the kernel reads or writes through one it checks that the whole
`ptr..ptr+len` range lies inside the user window **and** in pages the loader actually mapped with the needed
permission (`exec/usermem.rs`): not the guard below the stack, not the gap after the image, not the program's
own read-only code when the kernel is about to write. The end is computed with a checked add, so a pointer near
`usize::MAX` cannot wrap around to pass. A bad range is `EFAULT`; a path that isn't valid UTF-8 is `EINVAL`.
The kernel would otherwise fault on such an access, and a kernel fault is a panic. The check is also what makes
the `PAN` hardening usable: each syscall that touches user memory holds a `mmu::user_access()` guard for just
that long (see [`mmu.md`](mmu.md)).

## File descriptors

A program starts with three descriptors, bound by the shell's current frame ([`shell.md`](shell.md)): **0**
(stdin) is the keyboard, **1** (stdout) and **2** (stderr) are the console, unless a redirection or pipe bound
one to a file. `open` returns the lowest free number from 3 up. At most **13** files can be open at once, so
the table holds 16 descriptors in all; the fourteenth `open` is `EMFILE`. The 13 are shared with the handles the shell itself holds open for redirections and pipes, so a program running under a redirect has one fewer. The rest of the model (what an open
file is, what `close` commits) is in [`filesystem.md`](filesystem.md). Console output written to fd 1 or 2 is
decoded as UTF-8 (a character may be split across `write` calls) and mirrored to the serial port; reading from
fd 0 delivers one finished line at a time ([`console.md`](console.md)).

`userlib` buffers stdout (`write_stdout`/`flush_stdout`, 512 bytes) so a formatted print costs one display flush,
not one per fragment. The buffer is sent when it fills, when a fragment contains a newline, before any other
`write` (so output to different fds stays in order), before a `read` from fd 0, and at `exit`. Stderr is
unbuffered.

## The calls

| # | Name (Linux) | Arguments | Returns / notes |
|---|---|---|---|
| 93 | `exit` | `status` | Never returns. Only the low 8 bits are the status (0&ndash;255). |
| 63 | `read` | `fd, buf, len` | Bytes read; `0` at end of file (on stdin: `Ctrl+D` on an empty line). `EBADF` (including fds 1 and 2 when they are the console), `EFAULT`, `EISDIR` (a directory), `EIO`. |
| 64 | `write` | `fd, buf, len` | Bytes written. `EBADF`, `EFAULT`, `EIO`. |
| 56 | `open` | `path, path_len, flags` | An fd. Flags: `O_RDONLY` (0, also opens a directory), `O_WRONLY` (1: creates, and empties), `O_WRONLY \| O_APPEND` (`0o2000`: creates, starts at the end); anything else is `EINVAL`. `O_APPEND` alone is accepted as a plain read-only open (the append is ignored). Also `ENOENT`, `ENOTDIR`, `EISDIR`, `EACCES` (a read-only file, for write), `EMFILE`, `ENAMETOOLONG`, `EFAULT` (bad path pointer), `EINVAL` (path not UTF-8), `EIO`. |
| 57 | `close` | `fd` | `0`. For a writer this is what commits its size to disk, so it can return `EIO` if that fails. `EBADF` for an fd that isn't open. Closing a standard fd is allowed, but if the shell redirected it to a file, that closes the shell's own handle for the redirect, which the shell also closes later, so a program should leave its standard fds alone. |
| 61 | `getdents` | `fd, buf, len` | Bytes of directory records written, `0` when there are none left. Each record is 261 bytes (`DIRENT_SIZE`): see [`filesystem.md`](filesystem.md); `len` must be at least that, or the call returns `0` as if the listing were finished. `ENOTDIR` if `fd` is an open file but not a directory; `EBADF` for an fd that isn't a file. |
| 53 | `chmod` | `path, path_len, set, clear` | `0`. Only the executable and read-only bits may be named; anything else, or the root, is `EINVAL`. |
| 79 | `newfstatat` (`stat`) | `path, path_len, out` | `0`, after writing `STAT_SIZE` (16) bytes to `out`: `size: u32`, `attrs: u8`, then FAT's packed created date, time and time-tenth, modified date and time, and accessed date. |
| 34 | `mkdirat` (`mkdir`) | `path, path_len` | `0`. `EEXIST` if it exists, `ENOENT` if the parent doesn't. |
| 35 | `unlinkat` (`unlink`) | `path, path_len, flags` | `0`. Removes a file, or with `flags & AT_REMOVEDIR` (`0x200`) an empty directory. The path is normalized first, so `.` and `..` mean the current and parent directories. `EISDIR`, `ENOTDIR`, `ENOTEMPTY`; `EINVAL` for a path that is `/`, or for flags other than 0 and `AT_REMOVEDIR`. |
| 38 | `renameat` (`rename`) | `old, old_len, new, new_len` | `0`. Refuses an existing destination (`EEXIST`) and moving a directory into itself (`EINVAL`). |
| 17 | `getcwd` | `buf, len` | The length of the working directory's absolute path, copied to `buf` **without a NUL** (unlike Linux's). `ERANGE` if `len` is too small. |
| 29 | `ioctl` | `fd, request, arg` | Out-of-band control; only the console understands any request. `CONSOLE_CLEAR` (1) clears the screen and homes the cursor. `ENOTTY` for any other request or a non-console fd; `EBADF`. |
| 142 | `reboot` | `cmd` | Never returns on success. `LINUX_REBOOT_CMD_POWER_OFF` (`0x4321FEDC`) powers off, `LINUX_REBOOT_CMD_RESTART` (`0x01234567`) restarts, both through PSCI. Any other `cmd` is `EINVAL`. No `magic1`/`magic2`/`arg`. |
| 49 | `chdir` | | **Reserved, not implemented** (`ENOSYS`). Its number is held so a later stage doesn't have to pick one; see below. |

`chdir` is deliberately missing: with one global shell state, a program's `chdir` would change the *shell's*
directory too, which real Unix's per-process isolation prevents. The `cd` builtin does it instead, and the
syscall arrives when state is per-process.

## Faults are not syscalls

A data or instruction abort from EL0 is caught by the same handler (`sync_el0_handler`, reached from
`sync_el0_64` in `arch/vectors.s`). The kernel prints `Segmentation fault (address 0x..., ESR_EL1 0x...)` on
the console and the serial port, and the program ends with status **139** (`128 + SIGSEGV`, the shell
convention), which the shell reports as `exit 139`. Any other kind of exception from EL0 is treated as a
kernel-level unexpected exception.

## Error values

| `errno` | Value | `errmsg` text |
|---|---|---|
| `ENOENT` | -2 | No such file or directory |
| `EIO` | -5 | I/O error |
| `E2BIG` | -7 | Argument list too long |
| `ENOEXEC` | -8 | Exec format error |
| `EBADF` | -9 | Bad file descriptor |
| `EACCES` | -13 | Permission denied |
| `EFAULT` | -14 | Bad address |
| `EEXIST` | -17 | File exists |
| `ENOTDIR` | -20 | Not a directory |
| `EISDIR` | -21 | Is a directory |
| `EINVAL` | -22 | Invalid argument |
| `EMFILE` | -24 | Too many open files |
| `ENOTTY` | -25 | Inappropriate ioctl for device |
| `ENOSPC` | -28 | No space left on device |
| `ERANGE` | -34 | Numerical result out of range |
| `ENAMETOOLONG` | -36 | File name too long |
| `ENOSYS` | -38 | Function not implemented |
| `ENOTEMPTY` | -39 | Directory not empty |

The values are Linux's. Some errors never come from a syscall: `E2BIG` and `ENOEXEC` are what a *launch* reports
([`launching_programs.md`](launching_programs.md)).

## Adding a syscall

1. Add its number to `abi/src/syscall.rs` (Linux's aarch64 value, if it has one) and pin it in that file's
   test, along with any new constants or errno values, in `abi`'s own tests (`just test-host` runs them).
2. Import the number in `syscall/mod.rs`, add a dispatch arm in `sync_el0_handler` and its handler, validating
   every user pointer with the checks above.
3. Add a wrapper to `userlib` (`user/userlib/src/io.rs`, importing the number there too), re-exporting the number
   from `userlib/src/lib.rs`.
4. Test it from a program (`test/progs/src/bin/probe.rs` is where syscall-surface tests live) and add a row above.

## Where the code lives

| File | What it holds |
|---|---|
| `user/abi/src/syscall.rs` | The numbers |
| `user/abi/src/errno.rs`, `fs.rs`, `ioctl.rs`, `reboot.rs` | Errors and their text, the file constants, `ioctl` requests, `reboot` commands |
| `syscall/mod.rs` | The dispatcher, the trap frame, the fault path |
| `syscall/fd.rs` | The fd table and the file, `ioctl` and `getcwd` handlers; pointer validation |
| `syscall/power.rs` | `reboot` |
| `user/userlib/src/io.rs`, `syscall.rs` | The user-side wrappers and the raw `svc` |
