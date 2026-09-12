#![no_std]
#![no_main]

extern crate alloc;

mod base_addresses;
mod uart;

use aarch64_cpu::registers::{DAIF, ELR_EL1, ESR_EL1, Readable, Writeable};
use alloc::vec::Vec;
use arm_gic::gicv2::GicV2;
use arm_gic::{IntId, InterruptGroup};
use base_addresses::{BASE_ADDRESSES, UART0_BASE, init_base_addresses};
use core::fmt::Write;
use core::panic::PanicInfo;
use linked_list_allocator::LockedHeap;
use uart::{Uart, UartWriter};

static UART0: Uart = Uart::new(UART0_BASE, 1);

/// Heap backing store: a fixed-size region we own, handed to `ALLOCATOR`
/// once at boot. 64 KiB is generous headroom for a single accumulated line
/// of UART input.
const HEAP_SIZE: usize = 64 * 1024;
static mut HEAP: [u8; HEAP_SIZE] = [0; HEAP_SIZE];

/// The global allocator backing every `alloc::*` type used in this crate
/// (`Vec`, `String`, `Box`, ...). `LockedHeap` wraps a `linked_list_allocator`
/// heap in a spinlock -- required because `GlobalAlloc::alloc`/`dealloc` take
/// `&self`, not `&mut self`, so *some* interior-mutability mechanism is
/// unavoidable even though we never expect real contention on this
/// single-core system.
#[global_allocator]
static ALLOCATOR: LockedHeap = LockedHeap::empty();

/// The line currently being typed, one byte per received character.
/// `Vec::new()` is a `const fn` that performs no allocation, so this is a
/// valid `static` initializer even before `ALLOCATOR` is set up -- the first
/// real allocation only happens on the first `push`, at runtime, safely
/// after `kernel_main` has initialized the heap. Safe as a `static mut` for
/// the same reason as `r03_timer`'s `STATE`: only `irq_handler` ever touches
/// it, and at most one execution of `irq_handler` runs at a time.
static mut LINE_BUFFER: Vec<u8> = Vec::new();

struct AnsiEscape;
impl AnsiEscape {
    const RED: &'static str = "\x1b[1;31m";
    const GREEN: &'static str = "\x1b[1;32m";
    const RESET: &'static str = "\x1b[0m";
}

// no_mangle:  Ensures the function name is not altered by the compiler, so that boot.S can
//             branch to its address by name.
// extern "C": Specifies the calling convention for the function, ensuring it can be called
//             from other languages or assembly code.

/// The main entry point for the kernel. This function is called after the system is initialized.
///
/// `dtb_ptr`: the device tree blob pointer the ARM64 boot protocol places
/// in x0 at entry, preserved by boot.s and passed through per AAPCS64.
#[unsafe(no_mangle)]
extern "C" fn kernel_main(dtb_ptr: usize) -> ! {
    // SAFETY: this is the only call to `init`, and it happens before
    // anything else in this function (or any interrupt, which isn't enabled
    // yet) can possibly allocate.
    unsafe { ALLOCATOR.lock().init(&raw mut HEAP as *mut u8, HEAP_SIZE) };

    let mut uart0_writer: UartWriter = UartWriter { uart: &UART0 };
    init_base_addresses(dtb_ptr, &mut uart0_writer);

    write!(
        uart0_writer,
        "Heap initialized: {} bytes free.\r\n",
        ALLOCATOR.lock().free()
    )
    .unwrap_or(());
    write!(
        uart0_writer,
        "{}Type a line and press Enter; it will be echoed back.\r\n{}",
        AnsiEscape::GREEN,
        AnsiEscape::RESET
    )
    .unwrap_or(());

    gic_init();
    UART0.enable_rx_interrupt();

    // Clear the IRQ mask bit in DAIF, allowing IRQ exceptions to be taken.
    DAIF.write(DAIF::I::CLEAR);

    // Sleep between interrupts
    loop {
        unsafe { core::arch::asm!("wfe") };
    }
}

/// Initialize the Generic Interrupt Controller (GIC).
///
/// Constructs a fresh `GicV2` from the same base addresses used later in
/// `irq_handler` rather than storing one in a shared static: `new()` does no
/// register writes of its own (confirmed by reading arm-gic's source), so
/// nothing is lost by not persisting the instance, and it sidesteps the
/// `static mut` aliasing/exclusivity concerns entirely -- each instance's
/// lifetime is fully contained within the function that constructs it, so
/// there's never more than one alive at a time despite the same MMIO region
/// being wrapped repeatedly across separate calls.
fn gic_init() {
    let gicd = BASE_ADDRESSES.get_gicd();
    let gicc = BASE_ADDRESSES.get_gicc();
    let mut gic = unsafe { GicV2::new(gicd as *mut _, gicc as *mut _) };
    gic.setup();

    let uart_irq = IntId::spi(UART0.spi);

    // Arbitrary priority -- it's the only interrupt source we have.
    gic.set_interrupt_priority(uart_irq, 0xa0);
    gic.enable_interrupt(uart_irq, true)
        .expect("failed to enable UART0 interrupt");

    // Allow through interrupts of any priority.
    gic.set_priority_mask(0xff);
}

/// Handles unexpected exceptions, i.e. not IRQ_EL1H.
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

/// Handles IRQ (Interrupt Request) exceptions.
///
/// See `gic_init`'s doc comment for why this constructs its own `GicV2`
/// rather than sharing the one from `gic_init` via a static.
#[unsafe(no_mangle)]
extern "C" fn irq_handler() {
    let mut gic = unsafe {
        GicV2::new(
            BASE_ADDRESSES.get_gicd() as *mut _,
            BASE_ADDRESSES.get_gicc() as *mut _,
        )
    };

    // Group0: matches the C version's plain GICC_IAR/GICC_EOIR (offsets
    // 0x00C/0x010), which is what this QEMU config (no `secure=on`, no GIC
    // Security Extensions) actually uses -- Group1 goes through the
    // separate AIAR/AEOIR registers instead.
    if let Some(intid) = gic.get_and_acknowledge_interrupt(InterruptGroup::Group0) {
        if intid == IntId::spi(UART0.spi) {
            handle_uart_irq();
        }
        gic.end_interrupt(intid, InterruptGroup::Group0);
    }
}

/// Handles a UART0 receive interrupt: accumulates printable characters into
/// the heap-allocated `LINE_BUFFER`, growing it as needed; backspace erases
/// the last accumulated character; Enter echoes the whole accumulated line
/// back and clears the buffer for the next one.
fn handle_uart_irq() {
    // SAFETY: `&raw mut` forms a pointer without creating a reference, so
    // this alone needs no unsafe. Dereferencing it below does: at most one
    // execution of `irq_handler` ever runs at a time (see LINE_BUFFER's doc
    // comment), so this is the only place ever touching it concurrently
    // with itself.
    let line = unsafe { &mut *(&raw mut LINE_BUFFER) };

    while let Some(c) = UART0.try_getc() {
        match c {
            b'\r' | b'\n' => {
                UART0.puts("\r\n");
                UART0.puts(str::from_utf8(line).unwrap_or("<invalid utf-8>"));
                UART0.puts("\r\n");
                line.clear();
            }
            0x08 => {
                // Backspace: erase the last accumulated character, if any,
                // both in the buffer and on the terminal.
                if line.pop().is_some() {
                    UART0.puts("\x08 \x08");
                }
            }
            0x7f => {
                // Delete (0x7f): forward-delete has no real meaning without
                // a cursor that can sit before the end of the line
            }
            0x1b => {
                // Escape: shown in standard caret notation as "^[".
                UART0.puts("^[");
                line.push(b'^');
                line.push(b'[');
            }
            32..=126 => { // Printable ASCII
                UART0.putc(c);
                line.push(c);
            }
            _ => {}
        }
    }
    UART0.clear_rx_interrupt();
}

/// Panic handler; reached if our Rust code encounters an unrecoverable error
#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    let mut uart0_writer = UartWriter { uart: &UART0 };
    write!(
        &mut uart0_writer,
        "\r\n\n{}Kernel Panic! (at: {})\r\n\n{}{}\x1b[J",
        AnsiEscape::RED,
        _info.location().unwrap_or(core::panic::Location::caller()),
        _info.message(),
        AnsiEscape::RESET,
        // Clear the rest of the screen from the cursor down
    )
    .unwrap_or(());
    hang()
}

/// Hangs the CPU after program execution, so that we don't escape our program's memory space
fn hang() -> ! {
    loop {
        unsafe { core::arch::asm!("wfe") };
    }
}
