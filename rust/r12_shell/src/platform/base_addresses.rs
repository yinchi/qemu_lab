//! Discovers hardware base addresses from the device tree blob (DTB) the
//! ARM64 boot protocol places a pointer to in `x0` at entry, rather than
//! hardcoding them per-platform. Confirmed empirically before writing this:
//! `x0` at boot genuinely holds a valid FDT pointer -- reading the header's
//! `magic`, `totalsize`, `off_dt_struct`, and `version` fields all agree
//! with a genuine, well-formed FDT.

use core::fmt::Write;
use core::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

use fdt::Fdt;

/// Hard-coded UART0 base address, since it needs to be usable even if discovery fails
pub const UART0_BASE: usize = 0x0900_0000;
/// Size of the UART0 memory-mapped I/O region.
pub const UART0_SIZE: usize = 0x1000;

/// Size of the GIC Distributor memory-mapped I/O region.
pub const GICD_SIZE: usize = 0x10000;
/// Size of the GIC CPU Interface memory-mapped I/O region.
pub const GICC_SIZE: usize = 0x10000;

// The FDT (Flattened Device Tree) reports each virtio device (slot) separately, but we map them as
// one block instead, which works because the slots are contiguous in physical memory: 32 slots of
// 512 bytes each, 16 KiB, or four 4 KiB pages. Hard-coding the window keeps `arch/mmu.rs` to a
// single mapping. Empty slots are still valid MMIO addresses; they just read back a device ID of
// 0, which is how probing skips them.

/// Base address of the first `virtio,mmio` slot.
pub const VIRTIO_MMIO_BASE: usize = 0x0A00_0000;
/// Number of `virtio,mmio` slots this platform exposes.
pub const MAX_VIRTIO_MMIO_SLOTS: usize = 32;
/// Size of the entire `virtio,mmio` window, covering all slots (512 bytes each)
pub const VIRTIO_MMIO_SIZE: usize = MAX_VIRTIO_MMIO_SLOTS * 0x200;

/// Start of the fixed EL0-accessible user window in virtual memory.
pub const USER_BASE: usize = 0x4400_0000;
/// Size of the fixed EL0-accessible user window.
pub const USER_SIZE: usize = 0x0020_0000; // 2 MiB

// The window's layout, from the bottom: the program image (its segments, wherever it links them,
// page-aligned), a gap, the guard, then the stack up to the top of the window --
//
//   USER_BASE                 USER_IMAGE_END      USER_STACK_BOTTOM               USER_STACK_TOP
//   |  image (up to ~960 KiB) |  guard, unmapped  |        stack, 1 MiB           |
//                                  (64 KiB)
//
// Nothing is mapped between the last segment and the stack, so a stack that overflows -- or a
// wild pointer into the gap -- faults instead of silently running into the program's own data.
// (Stage 15 makes the window variable-sized; until then the split is fixed.)

/// The stack's size: generous, per the project's rule of thumb for MiB-scale stacks and buffers.
pub const USER_STACK_SIZE: usize = 0x0010_0000; // 1 MiB
/// The unmapped guard below the stack. Larger than any frame a sane program has, so a single
/// large frame can't step over it.
pub const USER_GUARD_SIZE: usize = 0x0001_0000; // 64 KiB
/// One past the highest stack address: the initial stack pointer's starting point.
pub const USER_STACK_TOP: usize = USER_BASE + USER_SIZE;
/// The lowest stack address; the guard lies just below it.
pub const USER_STACK_BOTTOM: usize = USER_STACK_TOP - USER_STACK_SIZE;
/// The highest address (exclusive) a program's image may occupy.
pub const USER_IMAGE_END: usize = USER_STACK_BOTTOM - USER_GUARD_SIZE;

/// One `virtio,mmio` slot's base address and SPI (Shared Peripheral Interrupt) number, as a pair
/// of atomics so the whole table can live in a plain `static` (see this module's doc comment).
struct VirtioMmioSlot {
    /// The base address of this `virtio,mmio` slot.
    base: AtomicUsize,

    /// The device-tree SPI number -- mapping to an interrupt ID of 32 + spi.
    spi: AtomicU32,
}

impl VirtioMmioSlot {
    const fn empty() -> Self {
        Self {
            base: AtomicUsize::new(0),
            spi: AtomicU32::new(0),
        }
    }
}

/// Global instance of the atomic base addresses -- GIC and every `virtio,mmio` slot.
pub static BASE_ADDRESSES: AtomicBaseAddresses = AtomicBaseAddresses {
    gicd: AtomicUsize::new(0),
    gicc: AtomicUsize::new(0),
    virtio_mmio: [const { VirtioMmioSlot::empty() }; MAX_VIRTIO_MMIO_SLOTS],
    virtio_mmio_count: AtomicUsize::new(0),
};

/// Atomic-field struct for storing discovered base addresses.  Hard-coded base addresses
/// are excluded from this struct and declared as separate constants.
pub struct AtomicBaseAddresses {
    /// GIC Distributor base address
    gicd: AtomicUsize,
    /// GIC CPU Interface base address
    gicc: AtomicUsize,
    /// Every discovered `virtio,mmio` slot, in device-tree order. Only the first
    /// `virtio_mmio_count` entries are valid.
    virtio_mmio: [VirtioMmioSlot; MAX_VIRTIO_MMIO_SLOTS],
    /// How many of `virtio_mmio`'s entries were actually populated by `init_base_addresses`.
    virtio_mmio_count: AtomicUsize,
}

impl AtomicBaseAddresses {
    /// Returns the GIC Distributor base address.
    pub fn get_gicd(&self) -> usize {
        self.gicd.load(Ordering::Relaxed)
    }

    /// Returns the GIC CPU Interface base address.
    pub fn get_gicc(&self) -> usize {
        self.gicc.load(Ordering::Relaxed)
    }

    /// Sets the GIC Distributor base address.
    fn set_gicd(&self, addr: usize) {
        self.gicd.store(addr, Ordering::Relaxed);
    }

    /// Sets the GIC CPU Interface base address.
    fn set_gicc(&self, addr: usize) {
        self.gicc.store(addr, Ordering::Relaxed);
    }

    /// Records one discovered `virtio,mmio` slot. Only ever called from `init_base_addresses`,
    /// which runs once to completion before anything else can observe this table.
    fn push_virtio_mmio(&self, base: usize, irq: u32) -> bool {
        // Get the currently used count of virtio_mmio slots, i.e. the next available index.
        let i = self.virtio_mmio_count.load(Ordering::Relaxed);

        // If we've used up all available slots, return false.
        let Some(slot) = self.virtio_mmio.get(i) else {
            return false;
        };

        // Populate the slot with the provided base address and IRQ number.
        slot.base.store(base, Ordering::Relaxed);
        slot.spi.store(irq, Ordering::Relaxed);

        // Increment the count of populated virtio_mmio slots.
        self.virtio_mmio_count.store(i + 1, Ordering::Relaxed);

        // Return true to indicate the slot was successfully recorded.
        true
    }

    /// Returns every discovered `virtio,mmio` slot as `(base, irq)`, in device-tree order.
    pub fn virtio_mmio_slots(&self) -> impl Iterator<Item = (usize, u32)> + '_ {
        let count = self.virtio_mmio_count.load(Ordering::Relaxed);
        self.virtio_mmio[..count].iter().map(|slot| {
            (
                slot.base.load(Ordering::Relaxed),
                slot.spi.load(Ordering::Relaxed),
            )
        })
    }
}

/// Maximum device tree blob size this module can relocate. 2 MiB comfortably covers the
/// 1 MiB blob this platform produces, with headroom to spare.
const DTB_BUF_SIZE: usize = 1 << 21;

/// Buffer for our own copy of the device tree blob, relocated out of low memory (0x0) to avoid
/// "null pointer" issues.
static mut DTB_BUFFER: [u8; DTB_BUF_SIZE] = [0; DTB_BUF_SIZE];

/// Returns the first SPI number found in a FDT node's `interrupts` property, if any, else `None`.
/// Used to find the SPI number for each `virtio,mmio` device.
fn first_spi(node: fdt::node::FdtNode) -> Option<u32> {
    // Get the raw bytes of the `interrupts` property. Return `None` if the property is missing.
    let bytes = node.property("interrupts")?.value;

    // Define a closure to extract the i-th 4-byte cell as a big-endian u32.
    //Returns `None` if out of bounds.
    let cell = |i: usize| -> Option<u32> {
        let b = bytes.get(i * 4..i * 4 + 4)?;
        Some(u32::from_be_bytes(b.try_into().unwrap()))
    };

    // If the first cell (`type`) is 0 (GIC_SPI), return the second cell (SPI number).
    //Otherwise, return `None`.
    (cell(0)? == 0).then(|| cell(1)).flatten()
}

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
    // so both are sound as long as nothing else touches DTB_BUFFER concurrently -- true here,
    // since this whole function only ever runs once.
    let dst = &raw mut DTB_BUFFER as *mut u8;
    let data = unsafe {
        core::ptr::copy_nonoverlapping(src, dst, totalsize);
        core::slice::from_raw_parts(dst, totalsize)
    };

    let fdt = Fdt::new(data).expect("failed to parse device tree blob");

    // The GIC node's `reg` property has two regions: distributor, then
    // CPU interface, in that order (confirmed via the same dtc dump
    // used to originally hardcode GICD_BASE/GICC_BASE).
    let mut gic_regions = fdt
        .find_compatible(&["arm,cortex-a15-gic"])
        .and_then(|node| node.reg())
        .expect("no arm,cortex-a15-gic node found in device tree");

    let gicd = gic_regions
        .next()
        .expect("GIC node has no distributor region")
        .starting_address as usize;
    let gicc = gic_regions
        .next()
        .expect("GIC node has no CPU interface region")
        .starting_address as usize;

    BASE_ADDRESSES.set_gicd(gicd);
    BASE_ADDRESSES.set_gicc(gicc);
    write!(writer, "Discovered: GICD={:#x} GICC={:#x}\r\n", gicd, gicc).unwrap_or(());

    // Find all virtio-mmio nodes and record their base addresses and SPI numbers.
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

        // Every virtio,mmio node on this platform specifies exactly one SPI -- if that's
        // somehow not true, skip the slot rather than record a bogus IRQ number for it.
        let Some(irq) = first_spi(node) else {
            continue;
        };

        if BASE_ADDRESSES.push_virtio_mmio(base.starting_address as usize, irq) {
            count += 1;
        } else {
            // Already found all available virtio-mmio slots: stop searching.
            break;
        }
    }

    write!(writer, "Discovered: {count} virtio-mmio slot(s)\r\n").unwrap_or(());
}
