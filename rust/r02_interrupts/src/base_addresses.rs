//! Discovers hardware base addresses from the device tree blob (DTB) the
//! ARM64 boot protocol places a pointer to in `x0` at entry, rather than
//! hardcoding them per-platform. Confirmed empirically before writing this:
//! `x0` at boot genuinely holds a valid FDT pointer -- reading the header's
//! `magic`, `totalsize`, `off_dt_struct`, and `version` fields all agree
//! with a genuine, well-formed FDT.
//!
//! Designed to be copied unchanged into every future rNN_name crate, same
//! as build.rs -- the compatible strings below are the only thing that
//! would ever need to change per-project.

use core::fmt::Write;

use fdt::Fdt;

/// Hard-coded UART0 base address, since it needs to be usable even if discovery fails
pub const UART0_BASE: usize = 0x0900_0000;

/// GIC base addresses, discovered from the device tree in `kernel_main`.
/// `irq_handler` reads this because it has no way to receive them as
/// parameters -- boot.s branches to it directly with no arguments. Written
/// exactly once, before `DAIF.write(DAIF::I::CLEAR)` unmasks IRQs, so every
/// read is guaranteed to happen after that one write -- no concurrent
/// access is possible on this single core.
pub static mut BASE_ADDRESSES: BaseAddresses = BaseAddresses { gicd: 0, gicc: 0 };

/// Maximum device tree blob size this module can relocate. 2 MiB comfortably covers the
/// 1 MiB blob this platform produces, with headroom to spare.
const DTB_BUF_SIZE: usize = 1 << 21;

/// Buffer for our own copy of the device tree blob, relocated out of low memory (0x0)
/// to avoid "null pointer" issues.
static mut DTB_BUFFER: [u8; DTB_BUF_SIZE] = [0; DTB_BUF_SIZE];

/// Initialize the device base addresses.
pub fn init_base_addresses(dtb_ptr: usize, mut writer: impl Write) {
    write!(writer, "raw dtb_ptr: {:#x}\r\n", dtb_ptr).unwrap_or(());

    let addrs = BaseAddresses::new(dtb_ptr);
    // SAFETY: this is the only write to BASE_ADDRESSES, and it happens
    // before DAIF.write(DAIF::I::CLEAR) below unmasks IRQs -- so irq_handler
    // can never read this concurrently with this write.
    unsafe { BASE_ADDRESSES = addrs };
    write!(
        writer,
        "Discovered: GICD={:#x} GICC={:#x}\r\n",
        addrs.gicd, addrs.gicc
    )
    .unwrap_or(());
}

/// Hardware base addresses discovered from the device tree at boot.
/// Note that these are specific to our QEMU `virt` machine configuration (e.g. single
/// UART0 serial port via `-serial`)
#[derive(Clone, Copy)]
pub struct BaseAddresses {
    /// GIC distributor base address as discovered from the device tree. Found using
    /// the `"arm,cortex-a15-gic"` compatibility string.
    pub gicd: usize,

    /// GIC CPU interface base address as discovered from the device tree. Found using
    /// the `"arm,cortex-a15-gic"` compatibility string.
    pub gicc: usize,
}

impl BaseAddresses {
    /// Parses the device tree blob at `dtb_ptr` (the pointer the boot
    /// protocol places in `x0` at entry) and discovers the base addresses
    /// of the devices this project depends on.
    pub fn new(dtb_ptr: usize) -> Self {
        let src = dtb_ptr as *const u8;

        // Read just the `totalsize` header field (offset 4, big-endian) via a raw pointer read --
        // not a reference -- so this works even though `dtb_ptr` is 0x0. The DTB format is always
        // big-endian regardless of host endianness, so we use `from_be` to interpret it correctly.
        let totalsize =
            u32::from_be(unsafe { (src.add(4) as *const u32).read_unaligned() }) as usize;
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

        Self { gicd, gicc }
    }
}
