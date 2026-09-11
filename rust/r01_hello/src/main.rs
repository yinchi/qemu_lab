#![no_std]
#![no_main]

use core::fmt::Write;
use core::panic::PanicInfo;

/// Represents a UART (Universal Asynchronous Receiver-Transmitter) peripheral.
struct Uart {
    /// Data register; writing to this register transmits a character
    dr: *mut u32,
    /// Flag register; a read-only status register
    fr: *const u32,
}

impl Uart {
    /// Transmit buffer full flag; read-only
    const TXFF: u32 = 1 << 5;

    /// Constructs a new UART instance with the given base address.
    const fn new(base: u64) -> Self {
        Self {
            dr: base as *mut u32,
            fr: (base + 0x18) as *const u32,
        }
    }

    /// Output a single character to the UART console.
    fn putc(&self, c: u8) {
        unsafe {
            // Wait until the transmit buffer is not full
            while (self.fr.read_volatile() & Self::TXFF) != 0 {}
            // Transmit the character once the buffer is ready
            self.dr.write_volatile(c as u32);
        }
    }

    /// Output a string to the UART console.
    fn puts(&self, s: &str) {
        for b in s.bytes() {
            self.putc(b);
        }
    }
}

const UART0: Uart = Uart::new(0x09000000u64);

/// Struct with the `core::fmt::Write` trait for UART output. Required as our
/// panic handler receives `PanicMessage` structs, not plain strings (thus we use the `write!`
/// macro to format the message for UART output, not `uart_puts` directly).
struct UartWriter;
impl Write for UartWriter {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        UART0.puts(s);
        Ok(())
    }
}

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
#[unsafe(no_mangle)]
extern "C" fn kernel_main() -> ! {
    write!(
        UartWriter,
        "{}Hello, world!\r\n{}",
        AnsiEscape::GREEN,
        AnsiEscape::RESET
    )
    .unwrap_or(());
    panic!("Test panic message");
}

/// Panic handler; reached if our Rust code encounters an unrecoverable error
#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    write!(
        UartWriter,
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
