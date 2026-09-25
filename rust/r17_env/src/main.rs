#![no_std]
#![no_main]

extern crate alloc;

// The kernel's layers, bottom to top: `arch` and `platform` (the CPU and the board), `drivers`
// (device protocols), the services built on them -- `fs`, `console`, `keyboard` -- then `exec`
// (programs), `syscall`, and `shell` at the top. A module uses ones at or below its own level, with
// one deliberate exception: `exec::process` calls `syscall::fd`'s `reset_for_launch`/`end_launch` to
// set up and tear down a program's file-descriptor table around each run, since the table's lifecycle
// is tied to `syscall::fd`'s own state.
mod arch;
mod console;
mod drivers;
mod exec;
mod fs;
mod keyboard;
mod platform;
mod shell;
mod syscall;
mod util;

use core::fmt::Write;
use core::panic::PanicInfo;
use core::sync::atomic::Ordering;

use aarch64_cpu::registers::{
    CNTKCTL_EL1, DAIF, ELR_EL1, ESR_EL1, FAR_EL1, ReadWriteable, Readable, Writeable,
};
use abi::fs::ATTR_EXEC;
use arm_gic::{IntId, InterruptGroup, gicv2::GicV2};
use linked_list_allocator::LockedHeap;

use crate::arch::{
    gic::{gic_enable, gic_setup},
    mmu,
};
use crate::console::{BG, Console};
use crate::drivers::virtio::{blk::Blk, gpu::Gpu, input::Keyboard};
use crate::exec::shell_state::{FRAMES, Frames};
use crate::fs::{
    blkio::{BlkIo, VOL},
    find_entry_checked,
};
use crate::keyboard::{
    keymap::{KEY_NAMES, KEY_STATE, KeyState, LOCK_STATE, LockState, build_key_names},
    line_discipline::{LINE_DISCIPLINE, LineDiscipline},
};
use crate::platform::{
    base_addresses::{BASE_ADDRESSES, init_base_addresses},
    globals::{BLK, BLK_SPI, CONSOLE, GPU, KEYBOARD, KEYBOARD_SPI},
    uart::{UART0, UartWriter},
};

/// Size of the kernel heap: 16 MiB, up from 1 MiB in Stage 11. A launch reads a whole ELF into a
/// `Vec` (capped at `shell::launch`'s `MAX_PROGRAM_SIZE`, half of this), `fs/files.rs` snapshots
/// directory listings and holds open readers/writers (each with `hadris-fat`'s own buffers), and
/// none of it is freed until the program is done. The heap is a static in `.bss`, so `arch/mmu.rs`
/// maps it with the rest of `.data`/`.bss` (`__data_start..__data_end`) and nothing else needs
/// to know its size. The ceiling is not QEMU's RAM but the fixed user address: every user binary is
/// linked at `0x44000000`, so the image (about 23 MiB with this heap and the Unifont tables, from
/// `0x40000000`) must end below it -- which leaves an unmapped gap of roughly 41 MiB, where earlier
/// stages' 6 MiB image left 48 MiB.
const HEAP_SIZE: usize = 16 * 1024 * 1024;
static mut HEAP: [u8; HEAP_SIZE] = [0; HEAP_SIZE];

#[global_allocator]
static ALLOCATOR: LockedHeap = LockedHeap::empty();

#[unsafe(no_mangle)]
extern "C" fn kernel_main(dtb_ptr: usize) -> ! {
    // SAFETY: the only call to `init`, and it happens before anything else
    // can possibly allocate.
    unsafe { ALLOCATOR.lock().init(&raw mut HEAP as *mut u8, HEAP_SIZE) };

    // Needs the allocator above (BiMap is hashmap-backed) but nothing else -- populated this
    // early because `Token::char()` and `LockState::apply` need it from the first key event onward
    // (see KEY_NAMES's doc comment in keyboard/keymap.rs).
    // SAFETY: sole write, happening before any key event can be processed.
    unsafe {
        KEY_NAMES = Some(build_key_names());
    }

    let mut uart0_writer = UartWriter { uart: &UART0 };
    init_base_addresses(dtb_ptr, &mut uart0_writer);

    // The MMU, turned on for the first time anywhere in this crate (see r09_userspace's own
    // first use, mirrored here). Must come after init_base_addresses (GICD/GICC are only known
    // once the DTB has been parsed) but before gic_setup/any other device access -- every
    // subsequent MMIO touch goes through the page table from this point on.
    let hardening = mmu::enable(BASE_ADDRESSES.get_gicd(), BASE_ADDRESSES.get_gicc());
    uart0_writer.write_str("MMU enabled.\r\n").unwrap_or(());
    // Let EL0 read the virtual counter (`CNTVCT_EL0`), so a program can tell how much time has passed
    // (there is no sleep syscall yet). `CNTFRQ_EL0`, its frequency, is always readable.
    CNTKCTL_EL1.modify(CNTKCTL_EL1::EL0VCTEN::SET);
    uart0_writer
        .write_str(if hardening.pan {
            "MMU hardening: WXN, stack alignment checks, PAN.\r\n"
        } else {
            "MMU hardening: WXN, stack alignment checks (no PAN on this CPU).\r\n"
        })
        .unwrap_or(());

    gic_setup();

    // Find the VirtIO block device and hand it over to BLK, then enable its SPI -- from this
    // point on, a real IRQ can call `static_mut_ref!(BLK)` inside `irq_handler`, so this write
    // must (and does) happen before `gic_enable(blk_spi)`. Unlike Stage 6/7, BLK stays live (and
    // its SPI enabled) for this program's entire remaining life: fs/blkio.rs's BlkIo reaches
    // through it for every filesystem read, not just one early load.
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

    // Mount the FAT filesystem built by `just disk` (see justfile) -- BlkIo presents the whole
    // block device as one byte-addressable stream, so hadris-fat can find its own boot sector,
    // FAT tables, and directory entries without this code needing to know their layout.
    //
    // SAFETY: BLK is populated and its SPI enabled above.
    let blk_io = unsafe { BlkIo::new() };
    // The volume stamps new and changed entries from the real-time clock (UTC), not the FAT epoch.
    let vol = hadris_fat::sync::FatVolumeBuilder::new(blk_io)
        .time_provider(&crate::fs::rtc_time::RTC_TIME)
        .open()
        .expect("failed to mount the FAT filesystem");
    uart0_writer
        .write_str("FAT filesystem mounted.\r\n")
        .unwrap_or(());

    // Mark every program in bin/ executable, per Stage 8's ATTR_EXEC convention -- claimed there,
    // enforced by `shell::launch` for the first time. `folder_to_img.sh`'s mtools-based image
    // build has no way to set this (mtools' `mattrib` only manages the standard DOS r/h/s/a
    // bits, not this project's own reserved one), so, same as r08_fs's own demo, it's set here
    // at boot instead -- freshly every run, since `just disk` reformats the image from scratch
    // each time.
    {
        let root = vol.root_dir();
        let bin_dir_entry = find_entry_checked(&root, "bin")
            .expect("failed to read the root directory")
            .expect("/bin/ not found on the disk image");
        let bin_dir = root
            .open_entry(&bin_dir_entry)
            .expect("failed to open bin directory");
        // Collected first: `set_attributes` rewrites directory entries, which mustn't happen
        // underneath a live `entries()` iterator over the same directory.
        let programs: alloc::vec::Vec<_> = bin_dir
            .entries()
            .map(|r| {
                let hadris_fat::sync::DirectoryEntry::Entry(entry) =
                    r.expect("directory entry read failed");
                entry
            })
            .filter(|entry| entry.is_file())
            .collect();
        for entry in programs {
            let new_attrs = hadris_fat::raw::DirEntryAttrFlags::from_bits_retain(
                entry.attributes().bits() | ATTR_EXEC,
            );
            vol.set_attributes(&entry, new_attrs)
                .expect("failed to set a program's executable bit");
        }
    }

    // The initial environment, from `/etc/environment`, into the shell's bottom frame. Before the first prompt
    // so its notes on the serial log come first, and with the local volume: the statics are set up below.
    let mut frames = Frames::new();
    shell::load_environment(&vol, &mut frames, &mut uart0_writer);

    // Find the VirtIO GPU device and set up the console -- polled, not interrupt-driven; see
    // drivers/virtio/gpu.rs's doc comment on why.
    let mut gpu_dev = Gpu::find(BASE_ADDRESSES.virtio_mmio_slots())
        .expect("no virtio-gpu device found among the virtio-mmio slots");
    let fb = gpu_dev.framebuffer();
    let mut console = Console::new(fb.into());
    console.clear(BG);

    // Find the VirtIO input device -- this stage's new piece. Its SPI isn't enabled yet: doing
    // so before CONSOLE/GPU/KEY_STATE/LOCK_STATE are populated below would let a keypress IRQ
    // reach `drain_keyboard` while those statics are still None.
    let (keyboard, kb_spi) = Keyboard::find(BASE_ADDRESSES.virtio_mmio_slots())
        .expect("no virtio-input device found among the virtio-mmio slots");

    uart0_writer
        .write_str("Keyboard found -- listening for key events via IRQ.\r\n")
        .unwrap_or(());

    let init_keys = KeyState::new();
    let init_locks = LockState::new();
    let mut line_discipline = LineDiscipline::new();
    shell::start_prompt(&mut line_discipline, &mut console);
    gpu_dev.flush();

    // Hand every piece of state the shell loop and the keyboard queue need over to its static home.
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
        LINE_DISCIPLINE = Some(line_discipline);
        FRAMES = Some(frames);
        VOL = Some(vol);
    }
    KEYBOARD_SPI.store(kb_spi, Ordering::Relaxed);
    gic_enable(kb_spi);

    // From here on `kernel_main` is the shell's read-eval loop, and never returns: the role `init`
    // plays. The keyboard interrupt only queues key presses (`irq_handler`); this loop consumes them.
    shell::run()
}

/// Handles IRQ (Interrupt Request) exceptions -- the only two possible sources are the block
/// device (for the filesystem reads and writes) and the keyboard.
///
/// See `arch::gic::gic_setup`'s doc comment for why this constructs its own `GicV2` rather than
/// sharing one via a static.
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
            // Only moves key presses from the device into the queue; the shell's loop does the rest.
            keyboard::queue::drain_keyboard();
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
    let far = FAR_EL1.get();
    // A fault at an address in the unmapped guard below the kernel stack (`arch/mmu.rs`) is an
    // overflow; `sync_el1h` runs this on a dedicated stack precisely so it can say so.
    if v == 4 && mmu::in_stack_guard(far as usize) {
        panic!(
            "Kernel stack overflow: the stack ran into its guard\r\n\
            FAR_EL1: {:#x}, ESR_EL1: {:#x}, ELR_EL1: {:#x}",
            far, esr, elr
        );
    }
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
