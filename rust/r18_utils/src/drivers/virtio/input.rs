//! Finds the VirtIO input device (`virtio-keyboard-device`) among the `virtio,mmio` slots
//! `base_addresses.rs` discovered. Modeled directly on Linux's `evdev` layer: events arrive as
//! `{type, code, value}` triples, one per key press/release/repeat, with no partial-sequence
//! ambiguity to resolve -- unlike Stage 5's UART-borne CSI escape sequences, there's nothing here
//! for a lone byte to be an incomplete prefix of.
//!
//! Same MMIO slot-probing approach as `blk.rs`/`gpu.rs`. Unlike Stage 6's polled block/GPU
//! devices, this one is genuinely asynchronous -- keys are pressed at arbitrary times, not in
//! response to something we requested -- so it's driven by a real GIC interrupt: `main.rs`'s
//! `irq_handler` drains events via `poll()` each time this device's SPI fires, rather than
//! looping on `poll()` itself.

pub use virtio_drivers::device::input::InputEvent;
use virtio_drivers::device::input::VirtIOInput;
use virtio_drivers::transport::DeviceType;
use virtio_drivers::transport::mmio::MmioTransport;

use super::{find_mmio_transport, hal::VirtioHalImpl};

/// `EV_KEY`, the `event_type` used for every key press/release/repeat (evdev's
/// `input-event-codes.h`). This is the only event type a `virtio-keyboard-device` ever sends.
pub const EV_KEY: u16 = 0x01;

/// Wrapper around a VirtIO input device, providing IRQ-driven event polling and interrupt
/// acknowledgment. Since we specify a keyboard, only events of type EV_KEY will be received.
pub struct Keyboard {
    inner: VirtIOInput<VirtioHalImpl, MmioTransport<'static>>,
}

impl Keyboard {
    /// Tries every discovered `virtio,mmio` slot in turn and returns a `Keyboard` (and its SPI
    /// number) for the first one that turns out to be an input device.
    pub fn find(mmio_slots: impl Iterator<Item = (usize, u32)>) -> Option<(Self, u32)> {
        let (transport, irq) = find_mmio_transport(mmio_slots, DeviceType::Input)?;
        let inner = VirtIOInput::new(transport).ok()?;
        Some((Self { inner }, irq))
    }

    /// Returns the next pending input event, if any -- non-blocking. Called from `irq_handler`
    /// in a drain loop each time this device's SPI fires, since one interrupt can cover more
    /// than one queued event (e.g. two keys changing in the same tick).
    pub fn poll(&mut self) -> Option<InputEvent> {
        self.inner.pop_pending_event()
    }

    /// Called from `irq_handler` once it's confirmed the acknowledged GIC interrupt was this
    /// device's SPI: clears the virtio-mmio transport's interrupt status so the level-triggered
    /// line deasserts (see `blk.rs::Blk::ack_interrupt`'s identical reasoning).
    pub fn ack_interrupt(&mut self) {
        self.inner.ack_interrupt();
    }
}
