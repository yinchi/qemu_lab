//! Discovers hardware base addresses from the device tree blob (DTB) the
//! ARM64 boot protocol places a pointer to in `x0` at entry, rather than
//! hardcoding them per-platform. Confirmed empirically before writing this:
//! `x0` at boot genuinely holds a valid FDT pointer -- reading the header's
//! `magic`, `totalsize`, `off_dt_struct`, and `version` fields all agree
//! with a genuine, well-formed FDT.
//!
//! This stage doesn't touch the GIC at all (no interrupt-driven I/O -- see
//! vectors.s), so unlike every earlier stage's copy of this file, what's
//! discovered here is every `virtio,mmio` slot's base address -- confirmed
//! via `dtc` against a real dumped DTB from this exact QEMU build before
//! being trusted: 32 `virtio,mmio` nodes at 0x0a000000..0x0a003e00, 0x200
//! apart.

use core::fmt::Write;
use core::sync::atomic::{AtomicUsize, Ordering};

use fdt::Fdt;

/// Hard-coded UART0 base address, since it needs to be usable even if discovery fails
pub const UART0_BASE: usize = 0x0900_0000;

/// Maximum number of `virtio,mmio` (memory-mapped I/O) slots this platform exposes.
pub const MAX_VIRTIO_MMIO_SLOTS: usize = 32;

/// Every discovered `virtio,mmio` slot's base address, in device-tree order, up to
/// `MAX_VIRTIO_MMIO_SLOTS`. Only the first `VIRTIO_MMIO_COUNT` entries are valid.
pub static mut VIRTIO_MMIO_BASES: [usize; MAX_VIRTIO_MMIO_SLOTS] = [0; MAX_VIRTIO_MMIO_SLOTS];

/// How many of `VIRTIO_MMIO_BASES`'s entries were actually populated by `init_base_addresses`.
pub static VIRTIO_MMIO_COUNT: AtomicUsize = AtomicUsize::new(0);

/// Returns every discovered `virtio,mmio` slot base address.
///
/// SAFETY: `init_base_addresses` runs once, to completion, before any other code (including this
/// function) can observe `VIRTIO_MMIO_BASES` -- there is no concurrent writer once it returns.
pub fn virtio_mmio_bases() -> &'static [usize] {
    let count = VIRTIO_MMIO_COUNT.load(Ordering::Relaxed);
    let base = &raw const VIRTIO_MMIO_BASES as *const usize;
    unsafe { core::slice::from_raw_parts(base, count) }
}

/// Maximum device tree blob size this module can relocate. 2 MiB comfortably covers the
/// 1 MiB blob this platform produces, with headroom to spare.
const DTB_BUF_SIZE: usize = 1 << 21;

/// Buffer for our own copy of the device tree blob, relocated out of low memory (0x0)
/// to avoid "null pointer" issues.
static mut DTB_BUFFER: [u8; DTB_BUF_SIZE] = [0; DTB_BUF_SIZE];

/// Initialize the device base addresses.
pub fn init_base_addresses(dtb_ptr: usize, mut writer: impl Write) {
    write!(writer, "raw dtb_ptr: {:#x}\r\n", dtb_ptr).unwrap_or(());

    let src = dtb_ptr as *const u8;

    // Read just the `totalsize` header field (offset 4, big-endian) via a raw pointer read --
    // not a reference -- so this works even though `dtb_ptr` is 0x0. The DTB format is always
    // big-endian regardless of host endianness, so we use `from_be` to interpret it correctly.
    let totalsize = u32::from_be(unsafe { (src.add(4) as *const u32).read_unaligned() }) as usize;
    assert!(
        totalsize <= DTB_BUF_SIZE,
        "device tree blob ({totalsize} bytes) exceeds DTB_BUF_SIZE ({DTB_BUF_SIZE} bytes)"
    );

    // SAFETY: `&raw mut` forms a pointer without creating a reference, so this alone needs no
    // unsafe. The copy and slice below do need it: `dst` is non-null (a real static's address),
    // so both are sound as long as nothing else touches DTB_BUFFER concurrently.
    let dst = &raw mut DTB_BUFFER as *mut u8;
    let data = unsafe {
        core::ptr::copy_nonoverlapping(src, dst, totalsize);
        core::slice::from_raw_parts(dst, totalsize)
    };

    let fdt = Fdt::new(data).expect("failed to parse device tree blob");

    // Find all virtio-mmio nodes and record their base addresses.
    let mut count = 0usize;
    for node in fdt.all_nodes() {
        let is_virtio_mmio = node
            .compatible()
            .is_some_and(|c| c.all().any(|s| s == "virtio,mmio"));

        // Not a virtio-mmio node: skip it.
        if !is_virtio_mmio {
            continue;
        }

        // Get the base address of the MMIO region for this virtio-mmio node.
        // If the node has no `reg` property or it's empty, skip it.
        let Some(base) = node.reg().and_then(|mut r| r.next()) else {
            continue;
        };

        // Already found all available virtio-mmio slots: stop searching.
        if count >= MAX_VIRTIO_MMIO_SLOTS {
            break;
        }

        // SAFETY: nothing else touches VIRTIO_MMIO_BASES until init_base_addresses returns.
        unsafe { VIRTIO_MMIO_BASES[count] = base.starting_address as usize };
        count += 1;
    }
    VIRTIO_MMIO_COUNT.store(count, Ordering::Relaxed);

    write!(writer, "Discovered: {count} virtio-mmio slot(s)\r\n").unwrap_or(());
}
