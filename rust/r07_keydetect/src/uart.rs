//! UART (Universal Asynchronous Receiver-Transmitter) driver for the PL011 peripheral.

use core::fmt::Write;

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
