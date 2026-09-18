#![no_std]
#![no_main]

extern crate alloc;

mod argv;
mod base_addresses;
mod blk;
mod console;
mod cp437;
mod devices;
mod elf;
mod fat_io;
mod fd;
mod font;
mod gpu;
mod keyboard;
mod keymap;
mod line;
mod mmu;
mod process;
mod syscall;
mod tokens;
mod uart;
mod utils;
mod virtio_hal;

use core::fmt::Write;
use core::panic::PanicInfo;
use core::sync::atomic::Ordering;

use aarch64_cpu::registers::{DAIF, ELR_EL1, ESR_EL1, Readable, Writeable};
use arm_gic::{IntId, InterruptGroup, gicv2::GicV2};
use argv::{Argv, ParseError};
use base_addresses::{BASE_ADDRESSES, UART0_BASE, init_base_addresses};
use blk::Blk;
use console::Console;
use font::Font;
use gpu::Gpu;
use keyboard::{EV_KEY, Keyboard};
use keymap::{KeyState, LockState};
use line::{LineBuffer, LineEvent};
use linked_list_allocator::LockedHeap;
use uart::{Uart, UartWriter};

use crate::devices::{BLK, BLK_SPI, CONSOLE, GPU, KEYBOARD, KEYBOARD_SPI};
use crate::fat_io::BlkIo;
use hadris_fat::sync::FatVolumeReadExt;
use crate::keymap::{KEY_NAMES, KEY_STATE, LOCK_STATE, build_key_names};

// ARGB colors (virtio-gpu's negotiated format -- see gpu.rs): alpha is a real channel here, not
// padding, so it must be opaque (0xFF) or the compositor may treat these pixels as transparent.
// pub(crate): fd.rs's Console write path (a running program's stdout) draws in these same colors.
pub(crate) const FG: u32 = 0xFF55FF55;
pub(crate) const BG: u32 = 0xFF00_0000;

/// The DOS `SYSTEM`-adjacent reserved attribute bit (`0x40`) Stage 8 claims as this project's
/// own "executable" convention (`r08_fs/src/main.rs`) -- claimed there, actually enforced here:
/// the first stage with a loader to enforce it against.
const EXEC_BIT: u8 = 0x40;

/// The prompt shown before the line buffer -- fixed text with no relation to `LINE`'s own
/// content, so it's structurally impossible for Backspace (which only ever pops `LINE`, see
/// `line.rs`) to erase into or through it.
const PROMPT: &str = "> ";

static UART0: Uart = Uart::new(UART0_BASE, 1);

const HEAP_SIZE: usize = 128 * 1024;
static mut HEAP: [u8; HEAP_SIZE] = [0; HEAP_SIZE];

#[global_allocator]
static ALLOCATOR: LockedHeap = LockedHeap::empty();

/// The font, read off the block device once at boot. `Console`'s `Font` borrows this, so it
/// needs to be `'static` rather than a `kernel_main`-local array -- see Stage 6's copy of this
/// file, which could get away with a local since nothing there needed to outlive `kernel_main`.
static mut FONT_DATA: [u8; 4096] = [0; 4096];

/// The mounted FAT filesystem -- kept alive for the program's entire remaining life, not just at
/// boot, since a program name typed at the prompt needs looking up fresh every time. Its
/// `root_dir()`/`open_entry()` results (`FatDir<'a, BlkIo>`) borrow from this, so unlike `LINE`/
/// `KEY_STATE` those are never themselves stored in a static -- only ever re-derived, cheaply,
/// at each lookup (see `launch`).
static mut VOL: Option<hadris_fat::sync::FatVolume<BlkIo>> = None;

/// Finds a named entry in `dir`, case-sensitively matching the display name `hadris-fat` derives
/// from the on-disk 8.3/LFN entry -- same as `r09_userspace`'s own `find_entry`, except `None`
/// on a miss rather than a panic: that one only ever looks up filenames this project's own build
/// baked in, but this one looks up whatever the user just typed, and a typo is an ordinary,
/// expected outcome to report, not a kernel bug.
fn find_entry<'a>(
    dir: &hadris_fat::sync::FatDir<'a, BlkIo>,
    name: &str,
) -> Option<hadris_fat::sync::FileEntry> {
    dir.entries().find_map(|r| {
        let hadris_fat::sync::DirectoryEntry::Entry(entry) = r.expect("directory entry read failed");
        (entry.name() == name).then_some(entry)
    })
}

/// Finds `argv.program()` in `bin/` (nowhere else -- see `ROADMAP.md`'s Stage 10 section: no
/// `PATH`-style search, no relative/absolute paths, none of that exists in this project yet),
/// checks its executable bit (`EXEC_BIT`, Stage 8's convention -- claimed there, enforced here
/// for the first time), and runs it with `argv`'s full argument list if both succeed. Reports to
/// UART, never panics, on any failure a user's typo or an unmarked file can genuinely cause --
/// the same way a real shell's "command not found" does, not a kernel bug to crash over.
fn launch(
    vol: &hadris_fat::sync::FatVolume<BlkIo>,
    argv: &Argv,
    uart: &mut UartWriter,
    console: &mut Console,
) {
    let root = vol.root_dir();

    let Some(bin_dir_entry) = find_entry(&root, "bin") else {
        report(uart, console, "bin/ not found on the disk image");
        return;
    };
    let bin_dir = root
        .open_entry(&bin_dir_entry)
        .expect("failed to open bin directory");

    let Some(prog_entry) = find_entry(&bin_dir, argv.program()) else {
        report(uart, console, &alloc::format!("{}: not found", argv.program()));
        return;
    };

    if prog_entry.attributes().bits() & EXEC_BIT == 0 {
        report(
            uart,
            console,
            &alloc::format!("{}: not executable", argv.program()),
        );
        return;
    }

    let elf_bytes = read_file_to_vec(vol, &prog_entry);
    process::run_program(&elf_bytes, &argv.as_argv());
}

/// Reports one line of text to both UART and the console -- used for the errors `launch` (and
/// `Argv::parse` failing) can report, so a typo'd or unbuilt command is visible on screen, not
/// only in the UART log a user may not even have open.
fn report(uart: &mut UartWriter, console: &mut Console, msg: &str) {
    writeln!(uart, "{msg}").unwrap_or(());
    for c in msg.chars() {
        console.write_char(c, FG, BG);
    }
    console.write_char('\n', FG, BG);
}

/// Stage 10's bare line-reading loop's buffer (see `line.rs`). Lives across IRQs the same way
/// `KEY_STATE`/`LOCK_STATE` do, for the same reason: one keypress is one `handle_keyboard_irq`
/// call, so the line has to persist somewhere between them.
static mut LINE: Option<LineBuffer> = None;

/// Which row the live prompt+line is currently on. A finished line is never redrawn or erased
/// -- it's already showing correctly at this row from the live edits leading up to Enter -- so
/// finishing a line only ever needs to *advance* this to a new row (scrolling first if already
/// on the last one), not touch anything already on screen. Not an `Option` like `LINE`/
/// `KEY_STATE`: `0` is already the correct starting value, the same one `kernel_main`'s first
/// `show_row` call uses, so there's no real "uninitialized" state to model.
static mut INPUT_ROW: usize = 0;

/// Redraws one row with `prefix` followed by `line`, on the display only -- used for the
/// prompt+live-line row, redrawn on every keystroke, and for the fresh empty prompt drawn once a
/// line finishes. UART is kept quiet of the typing itself: it only ever hears about a `launch`/
/// `report` error (see `handle_keyboard_irq`'s `LineEvent::Finished` case), not a running echo
/// of every keystroke, or even of a line that ran successfully -- that line's own output (if
/// any) is what shows up instead, on the console, not UART.
///
/// Renders via `write_char` (not raw `putc`) specifically so `line` -- which, coming from
/// `LINE`, may contain a literal `\t` -- gets real tab-stop/control-character handling instead
/// of `putc`'s dumb byte-indexed-into-the-font behavior.
///
/// If `prefix`+`line` would overflow this row's width, only `line`'s *tail* is shown -- a
/// sliding window, not wrapping onto the next row. Wrapping would go uncleared by `clear_row`
/// above (only `row` itself is cleared), leaving stale glyphs behind once the line shrinks back;
/// `LINE` itself (see `line.rs`) is never truncated, only this on-screen slice of it.
///
/// The budget below reserves one trailing column so the rendered text never reaches `cols`
/// exactly: `Console::putc` auto-wraps to the *next* row the moment `cursor_col` hits `cols`,
/// even on what both this function and its caller consider the last character of a deliberately
/// single-row render. Filling the row exactly would leave the real cursor one row below where
/// `INPUT_ROW` still thinks it is -- invisible while typing (the next redraw resets the cursor
/// explicitly via `move_cursor` above), but very visible the moment `LineEvent::Finished`
/// unconditionally writes a `'\n'` to move off it: that lands one row further down than
/// intended, leaving the phantom-wrapped-to row sitting empty in between.
fn show_row(console: &mut Console, row: usize, prefix: &str, line: &str) {
    console.clear_row(row, BG);
    console.move_cursor(row, 0);

    let budget = console
        .cols
        .saturating_sub(prefix.chars().count())
        .saturating_sub(1);
    let skip = line.chars().count().saturating_sub(budget);

    for c in prefix.chars().chain(line.chars().skip(skip)) {
        console.write_char(c, FG, BG);
    }
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

/// Reads a whole file into a freshly allocated `Vec<u8>` -- same as `r09_userspace`'s helper of
/// the same name.
fn read_file_to_vec(
    vol: &hadris_fat::sync::FatVolume<BlkIo>,
    entry: &hadris_fat::sync::FileEntry,
) -> alloc::vec::Vec<u8> {
    vol.read_file(entry)
        .expect("failed to open file for reading")
        .read_to_vec()
        .expect("failed to read file")
}

/// Reads the font through the filesystem (`fonts/spleen.raw`), same as any other file -- no
/// more special-cased raw sector read the way Stage 6/7 needed before a real filesystem existed.
///
/// SAFETY: nothing else may touch FONT_DATA concurrently -- true here, since this only ever runs
/// once, from `kernel_main`, before anything else exists that could read or write it.
#[allow(clippy::deref_addrof)]
unsafe fn read_font(vol: &hadris_fat::sync::FatVolume<BlkIo>) -> &'static [u8; 4096] {
    let root = vol.root_dir();
    let fonts_dir_entry = find_entry(&root, "fonts").expect("fonts/ not found on the disk image");
    let fonts_dir = root
        .open_entry(&fonts_dir_entry)
        .expect("failed to open fonts directory");
    let font_entry =
        find_entry(&fonts_dir, "spleen.raw").expect("fonts/spleen.raw not found on the disk image");
    let font_bytes = read_file_to_vec(vol, &font_entry);
    unsafe {
        (&raw mut FONT_DATA as *mut u8).copy_from_nonoverlapping(font_bytes.as_ptr(), 4096);
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

    // The MMU, turned on for the first time anywhere in this crate (see r09_userspace's own
    // first use, mirrored here). Must come after init_base_addresses (GICD/GICC are only known
    // once the DTB has been parsed) but before gic_setup/any other device access -- every
    // subsequent MMIO touch goes through the page table from this point on.
    mmu::enable(BASE_ADDRESSES.get_gicd(), BASE_ADDRESSES.get_gicc());
    uart0_writer.write_str("MMU enabled.\r\n").unwrap_or(());

    gic_setup();

    // Find the VirtIO block device and hand it over to BLK, then enable its SPI -- from this
    // point on, a real IRQ can call `static_mut_ref!(BLK)` inside `irq_handler`, so this write
    // must (and does) happen before `gic_enable(blk_spi)`. Unlike Stage 6/7, BLK stays live (and
    // its SPI enabled) for this program's entire remaining life: fat_io.rs's BlkIo reaches
    // through it for every filesystem read, not just one early font load.
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
    let vol = hadris_fat::sync::FatVolume::open(blk_io).expect("failed to mount the FAT filesystem");
    uart0_writer
        .write_str("FAT filesystem mounted.\r\n")
        .unwrap_or(());

    // Mark echo executable, per Stage 8's EXEC_BIT convention -- claimed there, enforced by
    // `launch` below for the first time. `folder_to_img.sh`'s mtools-based image build has no
    // way to set this (mtools' `mattrib` only manages the standard DOS r/h/s/a bits, not this
    // project's own reserved one), so, same as r08_fs's own demo, it's set here at boot instead
    // -- freshly every run, since `just disk` reformats the image from scratch each time.
    {
        let root = vol.root_dir();
        let bin_dir_entry = find_entry(&root, "bin").expect("bin/ not found on the disk image");
        let bin_dir = root
            .open_entry(&bin_dir_entry)
            .expect("failed to open bin directory");
        let echo_entry = find_entry(&bin_dir, "echo").expect("bin/echo not found on the disk image");
        let new_attrs = hadris_fat::raw::DirEntryAttrFlags::from_bits_retain(
            echo_entry.attributes().bits() | EXEC_BIT,
        );
        vol.set_attributes(&echo_entry, new_attrs)
            .expect("failed to set echo's executable bit");
    }

    // SAFETY: BLK is populated and its SPI enabled above.
    let font = Font::new(unsafe { read_font(&vol) });

    uart0_writer
        .write_str("Font read from disk.\r\n")
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
    show_row(&mut console, 0, PROMPT, "");
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
        LINE = Some(LineBuffer::new());
        VOL = Some(vol);
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
/// toggles, then runs each genuine press through `tokens::emit` and feeds the result into
/// `LINE` (see `line.rs` for what that does with it):
/// - A plain edit (`LineEvent::Changed`) redraws `INPUT_ROW` in place with the live line.
/// - A finished line (`LineEvent::Finished`) always moves off the input row first (an
///   unconditional `'\n'`), then either runs `launch` -- which may load and run a whole program,
///   producing output of its own via `fd.rs` -- or reports a parse error via `report`, then
///   resyncs `INPUT_ROW` to wherever the console's cursor *actually* ended up (not simply "one
///   row down": a launched program's output can span an arbitrary number of rows) before
///   drawing a fresh prompt there.
///
/// UART only ever hears a `launch`/parse-error message (`report`) -- never a running echo of
/// every keystroke, and not even of a line that ran successfully: a successful program's own
/// output goes to the console, not UART. Auto-repeat (`value == 2`) intentionally reaches none
/// of this, the same as it's already a no-op for `KeyState`/`LockState` below.
///
/// Drawing happens immediately per event, not deferred to a single redraw after the drain loop:
/// a batch containing more than one Enter needs each one to actually advance/scroll/launch in
/// turn, not collapse into one. `GPU.flush()` alone is still deferred to the end of the batch --
/// it's a presentation step, not something drawing operations need in between to stay correct.
///
/// SAFETY: at most one `irq_handler` invocation runs at a time (single core, and taking an IRQ
/// exception masks further IRQs for its duration), and nothing outside `irq_handler` touches
/// KEYBOARD/CONSOLE/GPU/KEY_STATE/LOCK_STATE/LINE/INPUT_ROW from the point `kernel_main` enables
/// KEYBOARD_SPI's GIC line onward -- so these `static mut` accesses can't race anything.
/// `launch`'s `process::run_program` is what actually upholds this while a program runs: it
/// masks every DAIF bit for the program's entire time at EL0 (see `process.rs`'s doc comment),
/// specifically so a keyboard IRQ can never land mid-program and re-enter this function while an
/// outer call is still on the stack, blocked inside `run_program` -- that would otherwise remap
/// the fixed user window a program is currently executing out of, and stomp the single-slot
/// `KERNEL_CTX` checkpoint (`process.s`) its own `enter_el0` just wrote.
fn handle_keyboard_irq() {
    // SAFETY: see this function's doc comment.
    let kb = unsafe { static_mut_ref!(KEYBOARD) };
    kb.ack_interrupt();

    let mut uart_writer = UartWriter { uart: &UART0 };
    let mut needs_flush = false;

    while let Some(event) = kb.poll() {
        if event.event_type != EV_KEY {
            continue;
        }

        // value: 0 = released, 1 = pressed, 2 = auto-repeat (treated as still-pressed).
        let down = event.value != 0;

        // SAFETY: see this function's doc comment.
        let key_changed = unsafe { static_mut_ref!(KEY_STATE) }.set(event.code, down);

        // Lock keys flip on press only -- LockState::apply ignores release/auto-repeat itself.
        // SAFETY: see this function's doc comment.
        unsafe { static_mut_ref!(LOCK_STATE) }.apply(event.code, event.value);

        // Emit a token only on a genuine down-edge. Gating on `event.value == 1` alone isn't
        // enough: `key_changed` (from `KeyState::set`) is what's actually robust to a held key
        // resending its press, whether the transport tags that a `value == 2` auto-repeat or --
        // as observed on this virtio-input/QEMU setup -- just resends plain `value == 1` events
        // with no distinct repeat tag at all.
        if down && key_changed {
            // SAFETY: see this function's doc comment.
            let token = unsafe {
                tokens::emit(event.code, static_ref!(KEY_STATE), static_ref!(LOCK_STATE))
            };
            let Some(token) = token else { continue };

            // SAFETY: see this function's doc comment.
            let line = unsafe { static_mut_ref!(LINE) };
            match line.feed(token) {
                Some(LineEvent::Changed) => {
                    // SAFETY: see this function's doc comment.
                    unsafe {
                        let row = INPUT_ROW;
                        show_row(
                            static_mut_ref!(CONSOLE),
                            row,
                            PROMPT,
                            static_ref!(LINE).as_str(),
                        );
                    }
                    needs_flush = true;
                }
                Some(LineEvent::Finished(text)) => {
                    // SAFETY: see this function's doc comment.
                    let console = unsafe { static_mut_ref!(CONSOLE) };

                    // Move off the just-finished input row *before* anything below writes a
                    // single byte -- otherwise a launched program's own output (or an error
                    // message) starts writing right where the just-typed line's cursor was left
                    // sitting, running straight into its tail instead of starting its own row.
                    console.write_char('\n', FG, BG);

                    match Argv::parse(&text) {
                        Ok(argv) => {
                            // SAFETY: see this function's doc comment.
                            let vol = unsafe { static_ref!(VOL) };
                            launch(vol, &argv, &mut uart_writer, console);
                        }
                        // A blank line (just Enter with nothing typed) isn't an error --
                        // nothing to log, same as any real shell.
                        Err(ParseError::Empty) => {}
                        Err(ParseError::Malformed) => {
                            report(&mut uart_writer, console, &alloc::format!("Malformed input: {text:?}"));
                        }
                    }
                    // Resync INPUT_ROW to wherever the console's own cursor actually ended up --
                    // unchanged if nothing ran (still sitting at the end of the just-finished
                    // prompt+line, from the last Changed redraw), or wherever `launch`'s program
                    // output (fd::write, via Console::write_char -- itself now scroll-safe, see
                    // its doc comment) left it, however many rows that spanned. A trailing
                    // newline only if not already at column 0, so a program whose own last write
                    // already ended in '\n' doesn't get a spurious blank line.
                    if console.cursor().1 != 0 {
                        console.write_char('\n', FG, BG);
                    }
                    let row = console.cursor().0;
                    // SAFETY: see this function's doc comment.
                    unsafe {
                        INPUT_ROW = row;
                    }
                    show_row(console, row, PROMPT, "");
                    needs_flush = true;
                }
                None => {}
            }
        }
    }

    if needs_flush {
        // SAFETY: see this function's doc comment.
        unsafe { static_mut_ref!(GPU) }.flush();
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
