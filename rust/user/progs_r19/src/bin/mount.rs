//! `mount [SOURCE TARGET]` -- see `docs/progs.md`. With no operands, lists what is mounted, one `SOURCE on TARGET type vfat`
//! line per mounted device (`blkinfo`: the kernel's own table, which is all there is -- there is no `/proc/mounts`), the
//! source spelled the way `mount` takes one (`spell`). With
//! two, mounts the volume SOURCE names (`LABEL=name` or `UUID=XXXX-XXXX`) on the existing directory TARGET, through the
//! `mount` syscall, which is where every rule lives.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::vec::Vec;
use core::fmt::Write;

use abi::blk::{BLK_FAT, BlkInfo};
use abi::errno::{EINVAL, ENODEV};
use progs::{Fd, diag, errmsg};
use progs_r12::cli;
use progs_r19::spell::source_of;
use userlib::{ExitCode, blkinfo, mount};

userlib::entry_with_args!(run);

const USAGE: &str = "mount [SOURCE TARGET]";
const FLAGS: &[(&str, &str)] = &[];

fn list() -> ExitCode {
    let devices: Vec<BlkInfo> = (0..).map_while(|index| blkinfo(index).ok()).collect();
    for info in devices.iter().filter(|info| info.mount_len > 0) {
        let label = core::str::from_utf8(info.label()).unwrap_or("");
        // Another FAT volume with the same label: it would not name this one.
        let shared = devices.iter().filter(|d| d.flags & BLK_FAT != 0 && d.label() == info.label()).count() > 1;
        let _ = writeln!(
            Fd(1),
            "{} on {} type vfat",
            source_of(label, info.volume_id, shared),
            core::str::from_utf8(info.mount()).unwrap_or("?")
        );
    }
    ExitCode(0)
}

fn run(args: userlib::Args) -> ExitCode {
    let plain = match cli::plain("mount", USAGE, FLAGS, args) {
        Ok(plain) => plain,
        Err(status) => return status,
    };
    match plain.count {
        0 => list(),
        1 => diag::missing_operand_after("mount", plain.first.unwrap_or("")),
        2 => {
            let (source, target) = (plain.first.unwrap_or(""), plain.last.unwrap_or(""));
            let result = mount(source, target);
            if result >= 0 {
                return ExitCode(0);
            }
            let why = match result {
                EINVAL => "not a LABEL= or UUID= source, or not a FAT filesystem",
                ENODEV => "no volume has that label or ID",
                other => errmsg(other),
            };
            let _ = writeln!(Fd(2), "mount: {source} on {target}: {why}");
            ExitCode(1)
        }
        _ => {
            let extra = plain.last.unwrap_or("");
            diag::extra_operand("mount", extra)
        }
    }
}
