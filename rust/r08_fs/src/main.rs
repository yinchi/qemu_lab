#![no_std]
#![no_main]

extern crate alloc;

mod base_addresses;
mod blk;
mod console;
mod cp437;
mod devices;
mod fat_io;
mod font;
mod gpu;
mod uart;
mod utils;
mod virtio_hal;

use core::fmt::Write;
use core::panic::PanicInfo;
use core::sync::atomic::Ordering;

use crate::base_addresses::{BASE_ADDRESSES, UART0_BASE, init_base_addresses};
use crate::blk::Blk;
use crate::console::Console;
use crate::fat_io::BlkIo;
use crate::font::Font;
use crate::gpu::Gpu;
use crate::uart::{Uart, UartWriter};
use aarch64_cpu::registers::{DAIF, ELR_EL1, ESR_EL1, Readable, Writeable};
use arm_gic::{IntId, InterruptGroup, gicv2::GicV2};
use hadris_fat::sync::FatVolume;
use hadris_fat::sync::FatVolumeReadExt;
use linked_list_allocator::LockedHeap;

use crate::devices::{BLK, CONSOLE, GPU};

// ARGB colors (virtio-gpu's negotiated format -- see gpu.rs): alpha is a real channel here, not
// padding, so it must be opaque (0xFF) or the compositor may treat these pixels as transparent.
const FG: u32 = 0xFF55FF55;
const BG: u32 = 0xFF00_0000;

/// This project's own reserved-bit convention for "executable" (see `ROADMAP.md`'s Stage 8) --
/// one of the two bits the FAT spec leaves unused (`DirEntryAttrFlags` only names the other six),
/// following the same "claim spare bits for a new meaning" pattern FAT/VFAT itself used twice
/// already (the NT case-flags byte, the LFN attribute-combination trick). A claim, not a
/// guarantee -- same as real POSIX, where `chmod +x` on a file full of garbage still makes `ls
/// -F` show `*`; the actual failure only surfaces later, at `execve()`, as its own distinct error
/// (`ENOEXEC`).
const EXEC_BIT: u8 = 0x40;

static UART0: Uart = Uart::new(UART0_BASE, 1);

const HEAP_SIZE: usize = 128 * 1024;
static mut HEAP: [u8; HEAP_SIZE] = [0; HEAP_SIZE];

#[global_allocator]
static ALLOCATOR: LockedHeap = LockedHeap::empty();

/// The font, read off the filesystem once at boot. `Console`'s `Font` borrows this, so it needs
/// to be `'static` rather than a `kernel_main`-local array.
static mut FONT_DATA: [u8; 4096] = [0; 4096];

/// Writes one line to both the console, at its cursor, and the UART.
fn show_line(console: &mut Console, line: &str, uart: &mut UartWriter) {
    for ch in line.chars() {
        console.write_char(ch, FG, BG);
    }
    console.write_char('\n', FG, BG);
    write!(uart, "{line}\r\n").unwrap_or(());
}

/// Sets up the GIC distributor/CPU interface and the priority mask. Called once; `gic_enable`
/// (below) is what actually turns on individual interrupt sources, and is called once per device
/// instead, at the point it becomes safe to fire (see `kernel_main`).
fn gic_setup() {
    let mut gic = unsafe {
        GicV2::new(
            BASE_ADDRESSES.get_gicd() as *mut _,
            BASE_ADDRESSES.get_gicc() as *mut _,
        )
    };
    gic.setup();
    gic.set_priority_mask(0xff);
}

/// Enables one SPI at the GIC. See `gic_setup`'s doc comment on why this constructs its own
/// `GicV2` rather than sharing one from `gic_setup` via a static.
fn gic_enable(spi: u32) {
    let mut gic = unsafe {
        GicV2::new(
            BASE_ADDRESSES.get_gicd() as *mut _,
            BASE_ADDRESSES.get_gicc() as *mut _,
        )
    };
    let irq = IntId::spi(spi);
    gic.set_interrupt_priority(irq, 0xa0);
    gic.enable_interrupt(irq, true)
        .expect("failed to enable interrupt");
}

/// Reads a whole file into a freshly allocated `Vec<u8>` -- `FileReader::read_to_vec` already does
/// exactly this (and more carefully: it bounds its up-front allocation against the volume's actual
/// capacity, where a hand-rolled loop trusting `entry.len()` outright would let a corrupt directory
/// entry force an arbitrarily large allocation), so there's no reason to re-loop it ourselves.
fn read_file_to_vec(
    vol: &FatVolume<BlkIo>,
    entry: &hadris_fat::sync::FileEntry,
) -> alloc::vec::Vec<u8> {
    vol.read_file(entry)
        .expect("failed to open file for reading")
        .read_to_vec()
        .expect("failed to read file")
}

/// Finds a named entry in `dir`, case-sensitively matching the display name `hadris-fat` derives
/// from the on-disk 8.3/LFN entry (e.g. `"hello.txt"`, `"fonts"`). Deliberately not
/// case-insensitive, matching POSIX conventions.
fn find_entry<'a>(
    dir: &hadris_fat::sync::FatDir<'a, BlkIo>,
    name: &str,
) -> hadris_fat::sync::FileEntry {
    dir.entries()
        .find_map(|r| {
            let hadris_fat::sync::DirectoryEntry::Entry(entry) =
                r.expect("directory entry read failed");
            (entry.name() == name).then_some(entry)
        })
        .unwrap_or_else(|| panic!("{name} not found"))
}

#[unsafe(no_mangle)]
extern "C" fn kernel_main(dtb_ptr: usize) -> ! {
    // SAFETY: the only call to `init`, and it happens before anything else can possibly
    // allocate.
    unsafe { ALLOCATOR.lock().init(&raw mut HEAP as *mut u8, HEAP_SIZE) };

    let mut uart0_writer = UartWriter { uart: &UART0 };
    init_base_addresses(dtb_ptr, &mut uart0_writer);

    gic_setup();

    // Find the VirtIO block device and hand it over to BLK, then enable its SPI -- from this
    // point on, a real IRQ can call `static_mut_ref!(BLK)` inside `irq_handler`, so this write
    // must (and does) happen before `gic_enable(blk_spi)`. Unlike Stage 6/7, BLK stays live (and
    // its SPI enabled) for this program's entire remaining life: `fat_io.rs`'s `BlkIo` reaches
    // through it for every filesystem read, not just one early font load.
    let (blk, blk_spi) = Blk::find(BASE_ADDRESSES.virtio_mmio_slots())
        .expect("no virtio-blk device found among the virtio-mmio slots");
    // SAFETY: sole write to BLK, and it happens before BLK_SPI's GIC line is enabled below --
    // irq_handler can't observe BLK until then.
    unsafe {
        BLK = Some(blk);
    }
    devices::BLK_SPI.store(blk_spi, Ordering::Relaxed);
    gic_enable(blk_spi);

    // DAIF stays unmasked from here on -- every block-device read/write below blocks on a real
    // IRQ (see `Blk::read_blocks_irq`/`write_blocks_irq`).
    DAIF.write(DAIF::I::CLEAR);

    // Mount the FAT filesystem built by `just disk` (see justfile) -- BlkIo presents the whole
    // block device as one byte-addressable stream, so hadris-fat can find its own boot sector,
    // FAT tables, and directory entries without this code needing to know their layout.
    //
    // SAFETY: BLK is populated and its SPI enabled above.
    let blk_io = unsafe { BlkIo::new() };
    let vol = FatVolume::open(blk_io).expect("failed to mount the FAT filesystem");
    uart0_writer
        .write_str("FAT filesystem mounted.\r\n")
        .unwrap_or(());

    // Read the font through the filesystem, same as any other file -- no more special-cased raw
    // sector read the way Stage 6/7 needed before a real filesystem existed.
    let root = vol.root_dir();
    let fonts_dir_entry = find_entry(&root, "fonts");
    let fonts_dir = root
        .open_entry(&fonts_dir_entry)
        .expect("failed to open fonts directory");
    let font_entry = find_entry(&fonts_dir, "spleen.raw");
    let font_bytes = read_file_to_vec(&vol, &font_entry);
    // SAFETY: sole write to FONT_DATA, happening before anything else could read it.
    unsafe {
        (&raw mut FONT_DATA as *mut u8).copy_from_nonoverlapping(font_bytes.as_ptr(), 4096);
    }
    // SAFETY: FONT_DATA was just fully written above, and nothing else writes it afterward.
    #[allow(clippy::deref_addrof)]
    let font = Font::new(unsafe { &*(&raw const FONT_DATA) });

    // Mark hello.sh executable, per this project's own EXEC_BIT convention -- the first thing in
    // this program that actually writes to the filesystem (everything above only ever read).
    // Exercises hadris-fat's real write path (`FatVolume::set_attributes`), including `BlkIo`'s
    // read-modify-write logic, for the first time -- not just something that compiles unused.
    let hello_sh_entry = find_entry(&root, "hello.sh");
    let new_attrs = hadris_fat::raw::DirEntryAttrFlags::from_bits_retain(
        hello_sh_entry.attributes().bits() | EXEC_BIT,
    );
    vol.set_attributes(&hello_sh_entry, new_attrs)
        .expect("failed to set hello.sh's executable bit");

    // Find the VirtIO GPU device and set up the console -- polled, not interrupt-driven; see
    // gpu.rs's doc comment on why.
    let mut gpu_dev = Gpu::find(BASE_ADDRESSES.virtio_mmio_slots())
        .expect("no virtio-gpu device found among the virtio-mmio slots");
    let fb = gpu_dev.framebuffer();
    let mut console = Console::new(fb, font);
    console.clear(BG);

    // List the root directory.
    show_line(&mut console, "Root directory:", &mut uart0_writer);
    for entry in root.entries() {
        let hadris_fat::sync::DirectoryEntry::Entry(entry) =
            entry.expect("directory entry read failed");
        // POSIX `ls -F`'s convention: a trailing `/` marks a directory, `*` marks executable
        let suffix = if entry.is_directory() {
            "/"
        } else if entry.attributes().bits() & EXEC_BIT != 0 {
            "*"
        } else {
            ""
        };
        let mut line = alloc::string::String::new();
        write!(line, "  {}{suffix}", entry.name()).unwrap_or(());
        show_line(&mut console, &line, &mut uart0_writer);
    }

    // Read hello.txt and print its contents.
    let hello_entry = find_entry(&root, "hello.txt");
    let hello_bytes = read_file_to_vec(&vol, &hello_entry);
    let hello_text = core::str::from_utf8(&hello_bytes).unwrap_or("<invalid utf8>");
    show_line(&mut console, "", &mut uart0_writer);
    show_line(&mut console, "hello.txt contents:", &mut uart0_writer);
    show_line(&mut console, hello_text.trim_end(), &mut uart0_writer);

    gpu_dev.flush();

    // SAFETY: sole writes, and nothing outside kernel_main reads CONSOLE/GPU (irq_handler only
    // ever touches BLK).
    unsafe {
        CONSOLE = Some(console);
        GPU = Some(gpu_dev);
    }

    loop {
        unsafe { core::arch::asm!("wfe") };
    }
}

/// Handles IRQ (Interrupt Request) exceptions. The only possible source now is the block device
/// (every filesystem read/write, not just boot-time font loading -- see `devices.rs`).
///
/// See `gic_setup`'s doc comment for why this constructs its own `GicV2` rather than sharing one
/// via a static.
#[unsafe(no_mangle)]
extern "C" fn irq_handler() {
    let mut gic = unsafe {
        GicV2::new(
            BASE_ADDRESSES.get_gicd() as *mut _,
            BASE_ADDRESSES.get_gicc() as *mut _,
        )
    };

    // Group0: matches the C version's plain GICC_IAR/GICC_EOIR (offsets 0x00C/0x010), which is
    // what this QEMU config (no `secure=on`, no GIC Security Extensions) actually uses -- Group1
    // goes through the separate AIAR/AEOIR registers instead.
    if let Some(intid) = gic.get_and_acknowledge_interrupt(InterruptGroup::Group0) {
        if intid == IntId::spi(devices::BLK_SPI.load(Ordering::Relaxed)) {
            // SAFETY: BLK is populated before BLK_SPI's GIC line is ever enabled (kernel_main).
            unsafe { static_mut_ref!(BLK) }.ack_interrupt();
        }
        gic.end_interrupt(intid, InterruptGroup::Group0);
    }
}

#[unsafe(no_mangle)]
extern "C" fn unexpected_exception(v: usize) -> ! {
    // Error types corresponding to exception vectors in `vectors.s`.
    const ERROR_TYPES: [&str; 16] = [
        "sync_el1t",
        "irq_el1t",
        "fiq_el1t",
        "error_el1t",
        "sync_el1h",
        "irq_el1h",
        "fiq_el1h",
        "error_el1h",
        "sync_el0_64",
        "irq_el0_64",
        "fiq_el0_64",
        "error_el0_64",
        "sync_el0_32",
        "irq_el0_32",
        "fiq_el0_32",
        "error_el0_32",
    ];

    // Error status register and exception link register values.
    let esr = ESR_EL1.get();
    let elr = ELR_EL1.get();

    panic!(
        "Unexpected exception occurred {}\r\n\
        ESR_EL1: {:#x}, ELR_EL1: {:#x}",
        ERROR_TYPES[v], esr, elr
    );
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    let mut uart0_writer = UartWriter { uart: &UART0 };
    write!(
        &mut uart0_writer,
        "\r\n\nKernel Panic! (at: {})\r\n\n{}\r\n",
        info.location().unwrap_or(core::panic::Location::caller()),
        info.message(),
    )
    .unwrap_or(());
    hang()
}

/// Halts the CPU in an infinite loop, waiting for events.
/// Used to wait for an interrupt or at the end of kernel execution.
fn hang() -> ! {
    loop {
        unsafe { core::arch::asm!("wfe") };
    }
}
