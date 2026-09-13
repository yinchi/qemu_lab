#![no_std]
#![no_main]

extern crate alloc;

mod base_addresses;
mod blk;
mod console;
mod font;
mod gpu;
mod uart;
mod virtio_hal;

use aarch64_cpu::registers::{ELR_EL1, ESR_EL1, Readable};
use base_addresses::{UART0_BASE, init_base_addresses, virtio_mmio_bases};
use console::Console;
use core::fmt::Write;
use core::panic::PanicInfo;
use font::Font;
use linked_list_allocator::LockedHeap;
use uart::{Uart, UartWriter};

// ARGB colors (virtio-gpu's negotiated format -- see gpu.rs): alpha is a real channel here, not
// padding, so it must be opaque (0xFF) or the compositor may treat these pixels as transparent.
const FG: u32 = 0xFF55FF55;
const BG: u32 = 0xFF00_0000;

static UART0: Uart = Uart::new(UART0_BASE, 1);

const HEAP_SIZE: usize = 64 * 1024;
static mut HEAP: [u8; HEAP_SIZE] = [0; HEAP_SIZE];

#[global_allocator]
static ALLOCATOR: LockedHeap = LockedHeap::empty();

#[unsafe(no_mangle)]
extern "C" fn kernel_main(dtb_ptr: usize) -> ! {
    // SAFETY: the only call to `init`, and it happens before anything else
    // can possibly allocate.
    unsafe { ALLOCATOR.lock().init(&raw mut HEAP as *mut u8, HEAP_SIZE) };

    let mut uart0_writer = UartWriter { uart: &UART0 };
    init_base_addresses(dtb_ptr, &mut uart0_writer);

    // Find the VirtIO block device among the `virtio,mmio` (memory-mapped I/O) slots.
    let mut blk_dev = blk::find_block_device(virtio_mmio_bases())
        .expect("no virtio-blk device found among the virtio-mmio slots");

    // Log the discovery of the block device.
    write!(
        uart0_writer,
        "Block device found: {} sectors.\r\n",
        blk_dev.capacity()
    )
    .unwrap_or(());

    // The raw font is 256 glyphs x 16 bytes = 4096 bytes = 8 sectors
    let mut font_data = [0u8; 4096];

    // Read the font data from the block device into the buffer.
    blk_dev
        .read_blocks(0, &mut font_data)
        .expect("failed to read the font off the block device");

    // Confirm that the font data has been successfully read from the block device.
    uart0_writer
        .write_str("Font read from disk.\r\n")
        .unwrap_or(());

    // Initialize the font structure with the read font data.
    let font = Font::new(&font_data);

    // Find the VirtIO GPU device among the `virtio,mmio` (memory-mapped I/O) slots.
    let mut gpu_dev = gpu::Gpu::find(virtio_mmio_bases())
        .expect("no virtio-gpu device found among the virtio-mmio slots");

    // Fetch the framebuffer from the GPU device.
    let fb = gpu_dev.framebuffer();

    // Confirm that the framebuffer has been successfully retrieved from the GPU device.
    uart0_writer
        .write_str("virtio-gpu framebuffer ready.\r\n")
        .unwrap_or(());

    // Initialize the console interface for the framebuffer and
    // render the printable ASCII characters using it and the selected font.
    let mut console = Console::new(fb, font);
    console.clear(BG);
    for c in 0x20u8..=0x7E {
        console.putc(c, FG, BG);
    }
    // Flush the rendered content to the display.
    gpu_dev.flush();

    // Log that the printable ASCII characters have been rendered and flushed to the display.
    uart0_writer
        .write_str("Printable ASCII rendered and flushed to the display.\r\n")
        .unwrap_or(());

    hang()
}

#[unsafe(no_mangle)]
extern "C" fn unexpected_exception(v: usize) -> ! {
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

fn hang() -> ! {
    loop {
        unsafe { core::arch::asm!("wfe") };
    }
}
