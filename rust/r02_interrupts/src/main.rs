#![no_std]
#![no_main]

mod base_addresses;
mod uart;

use aarch64_cpu::registers::{DAIF, ELR_EL1, ESR_EL1, Readable, Writeable};
use arm_gic::gicv2::GicV2;
use arm_gic::{IntId, InterruptGroup};
use base_addresses::{BASE_ADDRESSES, UART0_BASE, init_base_addresses};
use core::fmt::Write;
use core::panic::PanicInfo;
use uart::{Uart, UartWriter};

static UART0: Uart = Uart::new(UART0_BASE);

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
    let mut uart0_writer: UartWriter = UartWriter { uart: &UART0 };
    init_base_addresses(dtb_ptr, &mut uart0_writer);

    write!(
        &mut uart0_writer,
        "{}Echo server. Type characters; they will be echoed back.\r\n{}",
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
    let addrs = unsafe { BASE_ADDRESSES };
    let mut gic = unsafe { GicV2::new(addrs.gicd as *mut _, addrs.gicc as *mut _) };
    gic.setup();

    let uart_irq = IntId::spi(Uart::SPI1);

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
    // SAFETY: BASE_ADDRESSES is set exactly once in kernel_main, before
    // IRQs are unmasked, so this read can never race that write.
    let mut gic =
        unsafe { GicV2::new(BASE_ADDRESSES.gicd as *mut _, BASE_ADDRESSES.gicc as *mut _) };

    // Group0: matches the C version's plain GICC_IAR/GICC_EOIR (offsets
    // 0x00C/0x010), which is what this QEMU config (no `secure=on`, no GIC
    // Security Extensions) actually uses -- Group1 goes through the
    // separate AIAR/AEOIR registers instead.
    if let Some(intid) = gic.get_and_acknowledge_interrupt(InterruptGroup::Group0) {
        if intid == IntId::spi(Uart::SPI1) {
            // Read characters from the receive buffer until it is empty and echo them.
            while let Some(c) = UART0.try_getc() {
                handle_char(c, &UART0);
            }
            UART0.clear_rx_interrupt();
        }
        gic.end_interrupt(intid, InterruptGroup::Group0);
    }
}

/// Echoes a received character: printable ASCII is echoed as-is,
/// carriage return / newline becomes a proper \r\n, ESC is shown as
/// `^`, and backspace erases the previous character on the terminal
/// (`\x08 \x08`).
fn handle_char(c: u8, out: &Uart) {
    match c {
        32..=126 => out.putc(c),
        b'\r' | b'\n' => out.puts("\r\n"), // carriage return / newline
        0x1b => out.putc(b'^'),            // escape character
        0x08 => out.puts("\x08 \x08"),     // backspace character
        _ => {}
    }
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
