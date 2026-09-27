//! Setup of the GICv2 interrupt controller: the distributor/CPU interface, and enabling one device's
//! interrupt line at a time. The IRQ *dispatch* (`irq_handler`, which decides what a given interrupt
//! means) stays in `main.rs`, since it has to know about every device.

use arm_gic::{IntId, gicv2::GicV2};

use crate::platform::base_addresses::BASE_ADDRESSES;

/// Sets up the GIC distributor/CPU interface and the priority mask. Called once; `gic_enable`
/// (below) is what actually turns on individual interrupt sources, and is called once per
/// device instead, at the point each one becomes safe to fire (see `kernel_main`).
pub fn gic_setup() {
    let mut gic = unsafe {
        GicV2::new(
            BASE_ADDRESSES.get_gicd() as *mut _,
            BASE_ADDRESSES.get_gicc() as *mut _,
        )
    };
    gic.setup();
    // Allow through interrupts of any priority -- these two sources are the only ones enabled,
    // so there's no priority scheme to enforce between them.
    gic.set_priority_mask(0xff);
}

/// Enables one SPI at the GIC. See `gic_setup`'s doc comment on why this constructs its own
/// `GicV2` rather than sharing one from `gic_setup` via a static (same reasoning Stage 2/3 use).
pub fn gic_enable(spi: u32) {
    let mut gic = unsafe {
        GicV2::new(
            BASE_ADDRESSES.get_gicd() as *mut _,
            BASE_ADDRESSES.get_gicc() as *mut _,
        )
    };
    let irq = IntId::spi(spi);
    // Arbitrary priority -- both enabled sources share it; see gic_setup.
    gic.set_interrupt_priority(irq, 0xa0);
    gic.enable_interrupt(irq, true)
        .expect("failed to enable interrupt");
}
