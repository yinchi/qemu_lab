//! UART (Universal Asynchronous Receiver-Transmitter) driver for the PL011 peripheral.

use core::fmt::Write;
use core::sync::atomic::{AtomicBool, Ordering};

use super::base_addresses::UART0_BASE;

/// The board's first UART -- the kernel's serial console, and (mirrored by `syscall/fd.rs`) the
/// transcript of everything a session prints.
pub static UART0: Uart = Uart::new(UART0_BASE, 1);

/// Represents a UART (Universal Asynchronous Receiver-Transmitter) peripheral.
///
/// Note that QEMU's `virt` machine, which we use for emulation, allows 1 or 2 UART peripherals,
/// depending on whether a second `-serial` device is specified.
pub struct Uart {
    /// Data register; writing transmits a character, reading receives one
    dr: *mut u32,
    /// Flag register; a read-only status register
    fr: *const u32,
    /// Interrupt mask set/clear register
    #[allow(dead_code)]
    imsc: *mut u32,
    /// Interrupt clear register
    #[allow(dead_code)]
    icr: *mut u32,
    /// SPI number for this UART peripheral, used to determine its interrupt ID (32 + SPI).
    #[allow(dead_code)]
    pub spi: u32,
}

// SAFETY: every field is a raw pointer into MMIO register space, and every
// access to it goes through `read_volatile`/`write_volatile` -- there's no
// Rust-visible aliasing for the compiler to reason about, and the hardware
// itself serializes concurrent register access. This lets `Uart` live in a
// single shared `static` instead of being duplicated at every use site.
unsafe impl Sync for Uart {}

impl Uart {
    /// Transmit buffer full flag; read-only
    pub const TXFF: u32 = 1 << 5;

    /// Receive buffer empty flag; read-only
    #[allow(dead_code)]
    pub const RXFE: u32 = 1 << 4;

    /// Interrupt mask bit: receive (RX)
    #[allow(dead_code)]
    pub const RX: u32 = 1 << 4;

    /// Constructs a new UART instance with the given base address.
    pub const fn new(base: usize, spi: u32) -> Self {
        Self {
            dr: base as *mut u32,
            fr: (base + 0x18) as *const u32,
            imsc: (base + 0x38) as *mut u32,
            icr: (base + 0x44) as *mut u32,
            spi,
        }
    }

    /// Output a single character to the UART console.
    pub fn putc(&self, c: u8) {
        unsafe {
            // Wait until the transmit buffer is not full
            while (self.fr.read_volatile() & Self::TXFF) != 0 {}
            // Transmit the character once the buffer is ready
            self.dr.write_volatile(c as u32);
        }
    }

    /// Output a string to the UART console.
    pub fn puts(&self, s: &str) {
        for b in s.bytes() {
            self.putc(b);
        }
    }

    /// Reads one character from the receive FIFO, if one is available.
    #[allow(dead_code)]
    pub fn try_getc(&self) -> Option<u8> {
        unsafe {
            if (self.fr.read_volatile() & Self::RXFE) != 0 {
                None
            } else {
                Some((self.dr.read_volatile() & 0xff) as u8)
            }
        }
    }

    /// Enables receive interrupts.
    #[allow(dead_code)]
    pub fn enable_rx_interrupt(&self) {
        unsafe { self.imsc.write_volatile(Self::RX) };
    }

    /// Clears (acknowledges) a pending receive interrupt.
    #[allow(dead_code)]
    pub fn clear_rx_interrupt(&self) {
        unsafe { self.icr.write_volatile(Self::RX) };
    }
}

/// Whether the UART is at the start of a line, so `uart_ensure_newline` knows if it owes one.
static UART_AT_LINE_START: AtomicBool = AtomicBool::new(true);

/// The current value of `UART_AT_LINE_START` -- for `testhooks::report_and_reset` (`syscall/fd.rs`)
/// to save before its own diagnostic line and restore after: the harness strips that line from the
/// transcript entirely, so it must leave no trace on this tracking either, or a real program line
/// right before it that did *not* end in a newline would wrongly look like it had one.
#[cfg(feature = "testhooks")]
pub fn uart_at_line_start() -> bool {
    UART_AT_LINE_START.load(Ordering::Relaxed)
}

/// Restores a value `uart_at_line_start` read earlier -- see there.
#[cfg(feature = "testhooks")]
pub fn set_uart_at_line_start(at_line_start: bool) {
    UART_AT_LINE_START.store(at_line_start, Ordering::Relaxed);
}

/// Writes `bytes` to `UART0`, converting `\n` to `\r\n` (a serial line's convention, same as
/// the kernel's own messages), and remembers whether that left it at the start of a line.
/// This mirrors what programs print to the console (and what's typed to them), so a serial log
/// carries a readable transcript of a session, not just the kernel's own messages.
pub fn uart_write(bytes: &[u8]) {
    for &b in bytes {
        if b == b'\n' {
            UART0.putc(b'\r');
        }
        UART0.putc(b);
    }
    if let Some(&last) = bytes.last() {
        UART_AT_LINE_START.store(last == b'\n', Ordering::Relaxed);
    }
}

/// Clears the screen of whatever terminal is watching the serial line (ANSI: cursor to home, erase the
/// display), the serial counterpart of clearing the console; the transcript then counts as at the start of
/// a line. The console itself takes no escape sequences -- only this mirror does.
pub fn uart_clear_screen() {
    uart_write(b"\x1b[H\x1b[2J");
    UART_AT_LINE_START.store(true, Ordering::Relaxed);
}

/// Starts a new UART line unless already at the start of one -- used before the shell's own
/// output (its prompt, its error messages) so it never lands in the middle of a program's last
/// unterminated line.
pub fn uart_ensure_newline() {
    if !UART_AT_LINE_START.load(Ordering::Relaxed) {
        uart_write(b"\n");
    }
}

/// Struct with the `core::fmt::Write` trait for UART output. Required as our
/// panic handler receives `PanicMessage` structs, not plain strings (thus we use the `write!`
/// macro to format the message for UART output, not `uart_puts` directly).
pub struct UartWriter {
    pub uart: &'static Uart,
}

impl Write for UartWriter {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        self.uart.puts(s);
        Ok(())
    }
}
