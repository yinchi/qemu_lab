//! The mounted volumes: which devices hold an open FAT filesystem, and where in the tree each is mounted.
//!
//! `TABLE` (`mounttable.rs`, pure) says which device answers for a path; `VOLS` holds the `FatVolume` for each device that
//! is mounted, indexed by device. The root is mounted once at boot (`install_root`); every other mount comes from the
//! `mount` syscall (`mount`), which a program reaches -- and, from the next Step, the shell's start-up does the same for
//! the lines of `/etc/fstab`. A volume stays open for as long as it is mounted, and a mounted volume is never dropped
//! while a file on it is open (`umount` checks), which is what lets `files.rs` hold `'static` references into it.
//!
//! SAFETY of the statics (every access): single core; every syscall runs with IRQs masked and the shell runs outside
//! interrupts, so nothing else touches them while a call is in progress; and `irq_handler` never does.

use abi::blk::BLK_MOUNT_MAX;
use abi::errno::{EBUSY, EINVAL, ENAMETOOLONG, ENODEV};
use hadris_fat::sync::{FatVolume, FatVolumeBuilder};

use super::blkio::BlkIo;
use super::mounttable::{MountError, MountTable, Source, is_within};
use super::{devices, files};
use crate::drivers::virtio::blk::MAX_BLK;
use crate::exec::shell_state;

static mut VOLS: [Option<FatVolume<BlkIo>>; MAX_BLK] = [const { None }; MAX_BLK];
static mut TABLE: Option<MountTable> = None;

/// The open volumes, by device.
#[allow(clippy::deref_addrof)]
fn vols() -> &'static mut [Option<FatVolume<BlkIo>>; MAX_BLK] {
    // SAFETY: see the module doc.
    unsafe { &mut *(&raw mut VOLS) }
}

/// The mount table. Before `install_root` it is empty (everything resolves to device 0).
#[allow(clippy::deref_addrof)]
pub fn table() -> &'static MountTable {
    // SAFETY: see the module doc.
    unsafe { (*(&raw mut TABLE)).get_or_insert_with(MountTable::new) }
}

#[allow(clippy::deref_addrof)]
fn table_mut() -> &'static mut MountTable {
    // SAFETY: see the module doc.
    unsafe { (*(&raw mut TABLE)).get_or_insert_with(MountTable::new) }
}

/// The open filesystem on device `dev`. Panics if it is not mounted -- which the mount table never lets a path
/// resolve to.
pub fn volume(dev: usize) -> &'static FatVolume<BlkIo> {
    vols()[dev].as_ref().expect("a path resolved to a device that is not mounted")
}

/// Opens the filesystem on device `dev`, stamping new and changed entries from the real-time clock (UTC, not the FAT
/// epoch). `None` if it is not a filesystem `hadris-fat` can open.
///
/// SAFETY: `dev` is an index `BLK` is populated at, with its interrupt enabled (see `BlkIo::new`).
pub unsafe fn open_volume(dev: usize) -> Option<FatVolume<BlkIo>> {
    // SAFETY: the contract above.
    let blk_io = unsafe { BlkIo::new(dev) };
    FatVolumeBuilder::new(blk_io).time_provider(&super::rtc_time::RTC_TIME).open().ok()
}

/// Mounts the boot-time root, already opened, at `/`.
pub fn install_root(dev: usize, vol: FatVolume<BlkIo>) {
    vols()[dev] = Some(vol);
    table_mut().add("/", dev).expect("the root is the first mount");
}

/// Mounts the volume `source` names (`LABEL=name` or `UUID=XXXX-XXXX`) on the directory `target`, an absolute,
/// normalized path. `0`, or: `EINVAL` (not a source, or not a filesystem this kernel can open), `ENODEV` (no device
/// has that label or ID), `EBUSY` (that volume is mounted already, `target` is a mount point, or another mount is inside
/// it), `ENAMETOOLONG`, and
/// what `check_directory` says of the target (`ENOENT`, `ENOTDIR`).
pub fn mount(source: &str, target: &str) -> isize {
    let Some(source) = Source::parse(source) else {
        return EINVAL;
    };
    if target.len() > BLK_MOUNT_MAX {
        return ENAMETOOLONG;
    }
    let Some(dev) = source.find(&devices::identities()) else {
        return ENODEV;
    };
    if table().point_of(dev).is_some() || table().covers_a_mount(target) {
        return EBUSY;
    }
    if let Err(e) = files::check_directory(target) {
        return e;
    }
    // SAFETY: `dev` came from the device table, so `BLK` is populated at it with its interrupt enabled.
    let Some(vol) = (unsafe { open_volume(dev) }) else {
        return EINVAL;
    };
    // The volume is in place before the table can send a path to it.
    vols()[dev] = Some(vol);
    match table_mut().add(target, dev) {
        Ok(()) => 0,
        Err(_) => {
            // Cannot happen (both were checked above); undo rather than leave a volume nothing refers to.
            vols()[dev] = None;
            EBUSY
        }
    }
}

/// Unmounts the volume mounted exactly at `target`. `0`, or `EINVAL` (nothing is mounted there) and `EBUSY` (the root,
/// a mount with another inside it, or a volume with an open file or a working directory in it).
pub fn umount(target: &str) -> isize {
    if !table().is_mount_point(target) {
        return EINVAL;
    }
    let dev = table().resolve(target).0;
    if target == "/" || shell_state::frames().cwds().any(|cwd| is_within(target, cwd)) || files::open_on(dev) > 0 {
        return EBUSY;
    }
    match table_mut().remove(target) {
        Ok(dev) => {
            // No file is open on it (checked above), so nothing borrows the volume. Nothing is cached in memory
            // (`BlkIo` writes through), so dropping the volume loses no data.
            vols()[dev] = None;
            0
        }
        Err(MountError::NotMounted) => EINVAL,
        Err(_) => EBUSY,
    }
}
