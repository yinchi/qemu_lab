//! The block devices the kernel found, and what is on each: every device's first sector is read once at boot and
//! parsed by `bootsector` into a label and a volume ID, so a device is named by its contents (never by which
//! virtio-mmio slot it sits in -- QEMU's order is not the command line's). One of them is the root of the file
//! tree, chosen by `bootsector::pick_root`; the rest are found and described but not opened as filesystems (that is
//! what mounting is, `Stage19.md`). `blkinfo` reports a device to a program.

use alloc::vec::Vec;
use core::fmt::Write;
use core::sync::atomic::Ordering;

use abi::blk::{BLK_FAT, BLK_LABEL_MAX, BLK_ROOT, BlkInfo};

use super::bootsector::{self, BootInfo};
use crate::drivers::virtio::blk;
use crate::platform::globals::BLK_COUNT;

/// A block device: its size, and its boot-sector identity if it holds a FAT volume.
pub struct Device {
    pub capacity: u64,
    pub boot: Option<BootInfo>,
}

pub struct Devices {
    list: Vec<Device>,
    root: usize,
}

/// Every device found, and the root among them; set once by `probe` at boot.
static mut DEVICES: Option<Devices> = None;

/// The device table, or `None` before `probe` has run.
fn table() -> Option<&'static Devices> {
    // SAFETY: written once by `probe` at boot, before anything that reads it can run, and never after.
    #[allow(clippy::deref_addrof, reason = "`&raw const` is what the 2024 edition's `static_mut_refs` lint asks for")]
    unsafe { (*(&raw const DEVICES)).as_ref() }
}

/// Reads sector 0 of every device `BLK` holds (`BLK_COUNT` of them), works out the root and stores the table.
/// Returns the root's index, or `None` if no device holds a FAT volume. Reads are interrupt-driven, so this runs
/// after the devices' SPIs are enabled and IRQs unmasked. The table is written to `log` (the serial log).
///
/// SAFETY: `BLK` is populated for `BLK_COUNT` devices, their SPIs enabled, and IRQs unmasked -- as
/// `kernel_main` arranges before calling this.
pub unsafe fn probe(log: &mut impl Write) -> Option<usize> {
    let count = BLK_COUNT.load(Ordering::Relaxed);
    let mut list = Vec::with_capacity(count);
    for dev in 0..count {
        // SAFETY: `dev < count`; see this function's contract.
        let device = unsafe { blk::get(dev) };
        let capacity = device.capacity_bytes();
        let mut sector = [0u8; 512];
        // A device too small to have a boot sector, or one whose read fails, is simply "not FAT".
        let boot = if capacity >= 512 && device.read_blocks_irq(0, &mut sector).is_ok() {
            bootsector::parse(&sector)
        } else {
            None
        };
        list.push(Device { capacity, boot });
    }

    let identities: Vec<Option<BootInfo>> = list.iter().map(|d| d.boot).collect();
    let root = bootsector::pick_root(&identities);

    let _ = writeln!(log, "Block devices: {count}\r");
    for (index, device) in list.iter().enumerate() {
        let size = size_text(device.capacity);
        match device.boot {
            Some(info) => {
                let label = core::str::from_utf8(info.label()).unwrap_or("?");
                let note = match root {
                    Some((r, true)) if r == index => "  (root)",
                    Some((r, false)) if r == index => "  (root: no SYSTEM volume, using the first FAT volume)",
                    _ => "",
                };
                let _ = writeln!(log, "  {index}: {size:>7}  {label:<width$}  {}{note}\r", info.volume_id, width = BLK_LABEL_MAX);
            }
            None => {
                let _ = writeln!(log, "  {index}: {size:>7}  not a FAT volume -- ignored\r");
            }
        }
    }

    let root_index = root.map(|(index, _)| index);
    // SAFETY: sole write to DEVICES, at boot (see `table`).
    unsafe {
        DEVICES = Some(Devices { list, root: root_index.unwrap_or(0) });
    }
    root_index
}

/// A size in the binary units a disk image is made in: `64 MiB`, `512 KiB`, or plain bytes for the odd size.
fn size_text(bytes: u64) -> alloc::string::String {
    use alloc::format;
    if bytes >= 1 << 20 && bytes.is_multiple_of(1 << 20) {
        format!("{} MiB", bytes >> 20)
    } else if bytes >= 1 << 10 && bytes.is_multiple_of(1 << 10) {
        format!("{} KiB", bytes >> 10)
    } else {
        format!("{bytes} B")
    }
}

/// What `SYS_BLKINFO` reports for device `index`: `None` past the last device.
pub fn blkinfo(index: usize) -> Option<BlkInfo> {
    let devices = table()?;
    let device = devices.list.get(index)?;
    let mut info = BlkInfo { capacity: device.capacity, flags: 0, volume_id: 0, label_len: 0, label: [0; BLK_LABEL_MAX] };
    if let Some(boot) = device.boot {
        info.flags |= BLK_FAT;
        info.volume_id = boot.volume_id.0;
        info.label_len = u8::try_from(boot.label().len()).unwrap_or(0);
        info.label[..boot.label().len()].copy_from_slice(boot.label());
    }
    if index == devices.root {
        info.flags |= BLK_ROOT;
    }
    Some(info)
}
