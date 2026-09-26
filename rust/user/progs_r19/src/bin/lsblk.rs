//! `lsblk [-b]` -- see `docs/progs.md`. Lists the block devices the kernel found (`blkinfo`, asked for device 0, 1, ...
//! until `ENODEV`): a name (`vda`, `vdb`, ... in the kernel's device order), the size, and for a FAT volume its label
//! and its volume ID (what `blkid` calls the UUID). `MOUNTPOINT` is `/` for the root and empty for the rest until
//! there are mounts to show. A device that holds no FAT volume has an empty label and UUID.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;
use core::fmt::Write;

use abi::blk::{BLK_FAT, BLK_ROOT};
use abi::errno::ENODEV;
use progs::{Fd, fail};
use progs_r12::cli;
use progs_r18::human::human_size;
use progs_r19::table::{device_name, render};
use userlib::{ExitCode, blkinfo};

userlib::entry_with_args!(run);

const USAGE: &str = "lsblk [-b]";
const FLAGS: &[(&str, &str)] = &[("-b", "print sizes in bytes instead of `ls -h` units")];

/// `1M` rather than `1.0M`: a size that is exact needs no decimal.
fn size_text(bytes: u64) -> String {
    let text = human_size(bytes);
    match text.find(".0") {
        Some(at) if text[at + 2..].chars().all(|c| c.is_ascii_alphabetic()) => {
            let mut text = text;
            text.replace_range(at..at + 2, "");
            text
        }
        _ => text,
    }
}

fn run(args: userlib::Args) -> ExitCode {
    let mut bytes = false;
    let mut opts = cli::opts(args);
    loop {
        match cli::next("lsblk", &mut opts) {
            Ok(None) => break,
            Ok(Some(getargs::Arg::Long("help"))) => return progs::help(USAGE, FLAGS),
            Ok(Some(getargs::Arg::Short('b') | getargs::Arg::Long("bytes"))) => bytes = true,
            Ok(Some(other)) => return cli::invalid("lsblk", other),
            Err(status) => return status,
        }
    }

    let mut rows: Vec<Vec<String>> = Vec::new();
    for index in 0.. {
        let info = match blkinfo(index) {
            Ok(info) => info,
            Err(ENODEV) => break,
            Err(e) => {
                fail("lsblk", "cannot read the block devices", e);
                return ExitCode(1);
            }
        };
        let fat = info.flags & BLK_FAT != 0;
        let size = if bytes { alloc::format!("{}", info.capacity) } else { size_text(info.capacity) };
        let label = if fat { String::from_utf8_lossy(info.label()).into_owned() } else { String::new() };
        let uuid = if fat {
            let id = info.volume_id;
            alloc::format!("{:04X}-{:04X}", id >> 16, id & 0xffff)
        } else {
            String::new()
        };
        let mount = if info.flags & BLK_ROOT != 0 { "/" } else { "" };
        rows.push(alloc::vec![device_name(index), size, label, uuid, String::from(mount)]);
    }

    let _ = write!(Fd(1), "{}", render(&["NAME", "SIZE", "LABEL", "UUID", "MOUNTPOINT"], &rows));
    ExitCode(0)
}
