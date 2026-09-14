#![no_std]
#![no_main]

extern crate alloc;

mod base_addresses;
mod blk;
mod console;
mod devices;
mod font;
mod gpu;
mod keyboard;
mod keymap;
mod uart;
mod utils;
mod virtio_hal;

use core::fmt::Write;
use core::panic::PanicInfo;
use core::sync::atomic::Ordering;

use aarch64_cpu::registers::{DAIF, ELR_EL1, ESR_EL1, Readable, Writeable};
use arm_gic::{IntId, InterruptGroup, gicv2::GicV2};
use base_addresses::{BASE_ADDRESSES, UART0_BASE, init_base_addresses};
use blk::Blk;
use console::Console;
use font::Font;
use gpu::Gpu;
use keyboard::{EV_KEY, Keyboard};
use keymap::{KeyState, LockState};
use linked_list_allocator::LockedHeap;
use uart::{Uart, UartWriter};

use crate::devices::{BLK, BLK_SPI, CONSOLE, GPU, KEYBOARD, KEYBOARD_SPI};
use crate::keymap::{KEY_NAMES, KEY_STATE, LOCK_STATE, build_key_names};

// ARGB colors (virtio-gpu's negotiated format -- see gpu.rs): alpha is a real channel here, not
// padding, so it must be opaque (0xFF) or the compositor may treat these pixels as transparent.
const FG: u32 = 0xFF55FF55;
const BG: u32 = 0xFF00_0000;

static UART0: Uart = Uart::new(UART0_BASE, 1);

const HEAP_SIZE: usize = 64 * 1024;
static mut HEAP: [u8; HEAP_SIZE] = [0; HEAP_SIZE];

#[global_allocator]
static ALLOCATOR: LockedHeap = LockedHeap::empty();

/// The font, read off the block device once at boot. `Console`'s `Font` borrows this, so it
/// needs to be `'static` rather than a `kernel_main`-local array -- see Stage 6's copy of this
/// file, which could get away with a local since nothing there needed to outlive `kernel_main`.
static mut FONT_DATA: [u8; 4096] = [0; 4096];

/// Redraws one row with `prefix` followed by `line`, on both the display and UART.
fn show_row(console: &mut Console, row: usize, prefix: &str, line: &str, uart: &mut UartWriter) {
    console.clear_row(row, BG);
    console.move_cursor(row, 0);
    for b in prefix.bytes() {
        console.putc(b, FG, BG);
    }
    for b in line.bytes() {
        console.putc(b, FG, BG);
    }

    write!(uart, "{prefix}{line}\r\n").unwrap_or(());
}

/// Redraws row 0 with the currently held keys and row 1 with the currently toggled lock keys.
fn show_keyboard_state(
    console: &mut Console,
    keys: &KeyState,
    locks: &LockState,
    uart: &mut UartWriter,
) {
    show_row(console, 0, "Active keys: ", &keys.describe(), uart);
    show_row(console, 1, "Last held key: ", &keys.describe_last_held(), uart);
    show_row(console, 2, "Locks: ", &locks.describe(), uart);
}

/// Sets up the GIC distributor/CPU interface and the priority mask. Called once; `gic_enable`
/// (below) is what actually turns on individual interrupt sources, and is called once per
/// device instead, at the point each one becomes safe to fire (see `kernel_main`).
fn gic_setup() {
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
fn gic_enable(spi: u32) {
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

/// Reads the font off the block device, IRQ-driven (see `blk::Blk::read_blocks_irq`).
///
/// SAFETY: BLK must already be populated, and nothing else may touch FONT_DATA concurrently --
/// true here, since this only ever runs once, from `kernel_main`, before anything else exists
/// that could read or write either.
#[allow(clippy::deref_addrof)]
unsafe fn read_font() -> &'static [u8; 4096] {
    unsafe {
        static_mut_ref!(BLK)
            .read_blocks_irq(0, &mut *(&raw mut FONT_DATA))
            .expect("failed to read the font off the block device");
        // `FONT_DATA` isn't an `Option`, so `static_mut_ref!`/`static_ref!` (see `utils.rs`)
        // don't apply to it -- deliberately kept as its own plain raw-pointer access rather than
        // complicating those two macros to handle a single one-off non-`Option` case.
        &*(&raw const FONT_DATA)
    }
}

#[unsafe(no_mangle)]
extern "C" fn kernel_main(dtb_ptr: usize) -> ! {
    // SAFETY: the only call to `init`, and it happens before anything else
    // can possibly allocate.
    unsafe { ALLOCATOR.lock().init(&raw mut HEAP as *mut u8, HEAP_SIZE) };

    // Needs the allocator above (BiMap is hashmap-backed) but nothing else -- populated this
    // early because KeyState::describe needs it from its very first call onward (see KEY_NAMES's
    // doc comment in keymap.rs).
    // SAFETY: sole write, happening before anything could possibly call KeyState::describe.
    unsafe {
        KEY_NAMES = Some(build_key_names());
    }

    let mut uart0_writer = UartWriter { uart: &UART0 };
    init_base_addresses(dtb_ptr, &mut uart0_writer);

    gic_setup();

    // Find the VirtIO block device and hand it over to BLK, then enable its SPI -- from this
    // point on, a real IRQ can call `static_mut_ref!(BLK)` inside `irq_handler`, so this write
    // must (and does) happen before `gic_enable(blk_spi)`.
    let (blk, blk_spi) = Blk::find(BASE_ADDRESSES.virtio_mmio_slots())
        .expect("no virtio-blk device found among the virtio-mmio slots");
    // SAFETY: sole write to BLK, and it happens before BLK_SPI's GIC line is enabled below --
    // irq_handler can't observe BLK until then.
    unsafe {
        BLK = Some(blk);
    }

    BLK_SPI.store(blk_spi, Ordering::Relaxed);
    gic_enable(blk_spi);

    // DAIF stays unmasked from here on (never re-masked) -- both Stage 2/3's precedent and this
    // stage's rest of kernel_main run with real IRQs enabled the whole time.
    DAIF.write(DAIF::I::CLEAR);

    // SAFETY: BLK is populated and its SPI enabled above.
    let font = Font::new(unsafe { read_font() });

    uart0_writer
        .write_str("Font read from disk (IRQ-driven).\r\n")
        .unwrap_or(());

    // Find the VirtIO GPU device and set up the console -- polled, not interrupt-driven; see
    // gpu.rs's doc comment on why.
    let mut gpu_dev = Gpu::find(BASE_ADDRESSES.virtio_mmio_slots())
        .expect("no virtio-gpu device found among the virtio-mmio slots");
    let fb = gpu_dev.framebuffer();
    let mut console = Console::new(fb, font);
    console.clear(BG);

    // Find the VirtIO input device -- this stage's new piece. Its SPI isn't enabled yet: doing
    // so before CONSOLE/GPU/KEY_STATE/LOCK_STATE are populated below would let a keypress IRQ
    // reach handle_keyboard_irq while those statics are still None.
    let (keyboard, kb_spi) = Keyboard::find(BASE_ADDRESSES.virtio_mmio_slots())
        .expect("no virtio-input device found among the virtio-mmio slots");

    uart0_writer
        .write_str("Keyboard found -- listening for key events via IRQ.\r\n")
        .unwrap_or(());

    let init_keys = KeyState::new();
    let init_locks = LockState::new();
    show_keyboard_state(&mut console, &init_keys, &init_locks, &mut uart0_writer);
    gpu_dev.flush();

    // Hand every piece of state handle_keyboard_irq needs over to its static home.
    //
    // SAFETY: sole writes to each of these, and KEYBOARD_SPI's GIC line isn't enabled until
    // after this block -- irq_handler's keyboard branch can't run, and so can't observe any of
    // these, until then.
    unsafe {
        CONSOLE = Some(console);
        GPU = Some(gpu_dev);
        KEYBOARD = Some(keyboard);
        KEY_STATE = Some(init_keys);
        LOCK_STATE = Some(init_locks);
    }
    KEYBOARD_SPI.store(kb_spi, Ordering::Relaxed);
    gic_enable(kb_spi);

    // Sleep between interrupts -- every actual event, blk or keyboard, is now handled entirely
    // by irq_handler.
    loop {
        unsafe { core::arch::asm!("wfe") };
    }
}

/// Handles a keyboard interrupt: drains every pending event (one IRQ can cover more than one --
/// see `keyboard::Keyboard::poll`'s doc comment), updates the held-key set and the lock-key
/// toggles, and redraws only if something actually changed (so a run of auto-repeat events,
/// which `KeyState::set` turns into no-ops, doesn't cause a redundant redraw).
///
/// SAFETY: at most one `irq_handler` invocation runs at a time (single core, and taking an IRQ
/// exception masks further IRQs for its duration), and nothing outside `irq_handler` touches
/// KEYBOARD/CONSOLE/GPU/KEY_STATE/LOCK_STATE from the point `kernel_main` enables KEYBOARD_SPI's
/// GIC line onward -- so these `static mut` accesses can't race anything.
fn handle_keyboard_irq() {
    // SAFETY: see this function's doc comment.
    let kb = unsafe { static_mut_ref!(KEYBOARD) };
    kb.ack_interrupt();

    let mut changed = false;
    while let Some(event) = kb.poll() {
        if event.event_type != EV_KEY {
            continue;
        }

        // value: 0 = released, 1 = pressed, 2 = auto-repeat (treated as still-pressed).
        let down = event.value != 0;

        // SAFETY: see this function's doc comment.
        if unsafe { static_mut_ref!(KEY_STATE) }.set(event.code, down) {
            changed = true;
        }

        // Lock keys flip on press only -- LockState::apply ignores release/auto-repeat itself.
        // SAFETY: see this function's doc comment.
        if unsafe { static_mut_ref!(LOCK_STATE) }.apply(event.code, event.value) {
            changed = true;
        }
    }

    if changed {
        let mut uart_writer = UartWriter { uart: &UART0 };
        // SAFETY: see this function's doc comment.
        unsafe {
            show_keyboard_state(
                static_mut_ref!(CONSOLE),
                static_ref!(KEY_STATE),
                static_ref!(LOCK_STATE),
                &mut uart_writer,
            );
            static_mut_ref!(GPU).flush();
        }
    }
}

/// Handles IRQ (Interrupt Request) exceptions -- the only two possible sources are the block
/// device (only during the font read early in `kernel_main`) and the keyboard (for the rest of
/// the program's life).
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
        if intid == IntId::spi(BLK_SPI.load(Ordering::Relaxed)) {
            // SAFETY: BLK is populated before BLK_SPI's GIC line is ever enabled (kernel_main).
            unsafe { static_mut_ref!(BLK) }.ack_interrupt();
        } else if intid == IntId::spi(KEYBOARD_SPI.load(Ordering::Relaxed)) {
            handle_keyboard_irq();
        }
        gic.end_interrupt(intid, InterruptGroup::Group0);
    }
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
