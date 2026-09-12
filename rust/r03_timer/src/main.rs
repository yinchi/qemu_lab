#![no_std]
#![no_main]

mod base_addresses;
mod timer;
mod uart;

use aarch64_cpu::registers::{DAIF, ELR_EL1, ESR_EL1, Readable, Writeable};
use arm_gic::gicv2::GicV2;
use arm_gic::{IntId, InterruptGroup};
use base_addresses::{BASE_ADDRESSES, UART0_BASE, init_base_addresses};
use core::fmt::Write;
use core::panic::PanicInfo;
use core::sync::atomic::Ordering;
use uart::{Uart, UartWriter};

static UART0: Uart = Uart::new(UART0_BASE, 1);

struct AnsiEscape;
impl AnsiEscape {
    const RED: &'static str = "\x1b[1;31m";
    const GREEN: &'static str = "\x1b[1;32m";
    const RESET: &'static str = "\x1b[0m";
}

/// Which phase of the dots demo `irq_handler` is in. `Printing` carries its
/// own remaining-dot count, rather than that count living alongside the mode
/// as a separate field -- that would let `dots_remaining` hold stale/
/// meaningless data while `Waiting`, a combination that isn't actually valid
/// and now can't be expressed at all.
#[derive(Clone, Copy, PartialEq)]
enum Mode {
    /// Waiting for a `1`-`9` digit on UART0 selecting how many dots to print.
    Waiting,
    /// Printing one dot per second, this many more times.
    Printing { dots_remaining: u32 },
}

/// State shared between successive `irq_handler` invocations. Safe as a
/// `static mut`: taking an IRQ exception automatically sets `DAIF.I` on
/// entry, and `irq_handler` never clears it, so at most one execution of
/// `irq_handler` is ever running at a time on this single core -- and
/// nothing outside `irq_handler` touches this state.
static mut STATE: Mode = Mode::Waiting;

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

    print_prompt();
    gic_init();
    UART0.enable_rx_interrupt();

    // Clear the IRQ mask bit in DAIF, allowing IRQ exceptions to be taken.
    DAIF.write(DAIF::I::CLEAR);

    // Sleep between interrupts
    loop {
        unsafe { core::arch::asm!("wfe") };
    }
}

/// Prints the "select a digit" prompt -- shown at boot, and again after each
/// run of dots finishes.
fn print_prompt() {
    let mut w = UartWriter { uart: &UART0 };
    write!(
        w,
        "{}Select number of dots to print (1-9).\r\n{}",
        AnsiEscape::GREEN,
        AnsiEscape::RESET
    )
    .unwrap_or(());
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
    let gicd = BASE_ADDRESSES.gicd.load(Ordering::Relaxed);
    let gicc = BASE_ADDRESSES.gicc.load(Ordering::Relaxed);
    let mut gic = unsafe { GicV2::new(gicd as *mut _, gicc as *mut _) };
    gic.setup();

    let uart_irq = IntId::spi(UART0.spi);
    let timer_irq = IntId::ppi(timer::PPI);

    // The timer interrupt must outrank UART0's: handling a UART0 interrupt
    // (selecting a dot count) arms the timer, so the timer's own interrupt
    // must never be starved by a pending/in-progress UART0 one. Lower
    // numeric value = higher priority.
    gic.set_interrupt_priority(uart_irq, 0xa0);
    gic.set_interrupt_priority(timer_irq, 0x90);

    gic.enable_interrupt(uart_irq, true)
        .expect("failed to enable UART0 interrupt");
    gic.enable_interrupt(timer_irq, true)
        .expect("failed to enable timer interrupt");

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
        } else if intid == IntId::ppi(timer::PPI) {
            handle_timer_irq();
        }
        gic.end_interrupt(intid, InterruptGroup::Group0);
    }
}

/// Handles a UART0 receive interrupt: while waiting, looks for a `1`-`9`
/// digit selecting how many dots to print and arms the timer for the first
/// one; while printing, or once a digit is found, discards the rest of the
/// receive buffer.
fn handle_uart_irq() {
    // SAFETY: see STATE's doc comment -- at most one irq_handler runs at a
    // time, so this read/these writes can't race any other access.
    if unsafe { STATE } == Mode::Waiting {
        while let Some(c) = UART0.try_getc() {
            if (b'1'..=b'9').contains(&c) {
                // Echo back the number of dots selected.
                UART0.putc(c);
                UART0.puts("\r\n");

                unsafe {
                    STATE = Mode::Printing {
                        dots_remaining: (c - b'0') as u32,
                    }
                };
                timer::arm(timer::freq()); // Arm the timer for 1 second.
                break;
            }
        }
    }

    // Flush the rest of the receive buffer -- matches 04_dots: any leftover
    // bytes after a digit is found, or anything received while printing, is
    // simply discarded.
    while UART0.try_getc().is_some() {}
    UART0.clear_rx_interrupt();
}

/// Handles a timer interrupt: prints one dot and re-arms for the next one,
/// or -- once the requested count is reached -- returns to waiting and
/// re-prints the prompt.
fn handle_timer_irq() {
    // SAFETY: see STATE's doc comment.
    match unsafe { STATE } {
        Mode::Printing { dots_remaining } => {
            UART0.putc(b'.');
            let dots_remaining = dots_remaining - 1;

            if dots_remaining > 0 {
                unsafe { STATE = Mode::Printing { dots_remaining } };
                timer::arm(timer::freq()); // Re-arm for the next dot.
            } else {
                unsafe { STATE = Mode::Waiting };
                UART0.puts("\r\n"); // End the line of dots.
                print_prompt();
            }
        }
        Mode::Waiting => {
            // Timer fired while not printing -- shouldn't happen in normal
            // operation, but disable it defensively, matching 04_dots.
            timer::disable();
        }
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
