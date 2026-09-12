#![no_std]
#![no_main]

extern crate alloc;

mod base_addresses;
mod keyboard;
mod timer;
mod uart;

use aarch64_cpu::registers::{DAIF, ELR_EL1, ESR_EL1, Readable, Writeable};
use alloc::vec::Vec;
use arm_gic::gicv2::GicV2;
use arm_gic::{IntId, InterruptGroup};
use base_addresses::{BASE_ADDRESSES, UART0_BASE, init_base_addresses};
use core::fmt::Write;
use core::panic::PanicInfo;
use keyboard::{ParserState, Token, Tokens};
use linked_list_allocator::LockedHeap;
use uart::{Uart, UartWriter};

static UART0: Uart = Uart::new(UART0_BASE, 1);

const HEAP_SIZE: usize = 64 * 1024;
static mut HEAP: [u8; HEAP_SIZE] = [0; HEAP_SIZE];

#[global_allocator]
static ALLOCATOR: LockedHeap = LockedHeap::empty();

/// How long a lone `ESC` byte (or an abandoned/truncated escape sequence) is
/// held pending disambiguation before we give up waiting and resolve it as a
/// standalone Escape keypress. 50ms matches vim's own default `ttimeoutlen`
/// -- long enough that a real terminal's fast, complete escape-sequence
/// burst always arrives well within it, short enough that a genuinely bare
/// Escape press feels immediate rather than laggy.
const ESC_TIMEOUT_MS: u64 = 50;

fn esc_timeout_ticks() -> u64 {
    timer::freq() / 1000 * ESC_TIMEOUT_MS
}

/// The line being edited: its content, the cursor's position within it (an
/// index into `buf`, `0..=buf.len()`), and how far through recognizing a
/// possible escape sequence we are. Bundled into one `static mut` for the
/// same reason as every earlier stage's shared IRQ state: `irq_handler`
/// (and the timer interrupt it also handles here) has no way to receive
/// this as a parameter, and at most one execution of `irq_handler` ever
/// runs at a time on this single core, so nothing else can race it.
static mut EDITOR: Editor = Editor {
    buf: Vec::new(),
    cursor: 0,
    parser: ParserState::Ground,
};

struct Editor {
    buf: Vec<u8>,
    cursor: usize,
    parser: ParserState,
}

struct AnsiEscape;
impl AnsiEscape {
    const RED: &'static str = "\x1b[1;31m";
    const GREEN: &'static str = "\x1b[1;32m";
    const RESET: &'static str = "\x1b[0m";
}

#[unsafe(no_mangle)]
extern "C" fn kernel_main(dtb_ptr: usize) -> ! {
    // SAFETY: the only call to `init`, and it happens before anything else
    // can possibly allocate.
    unsafe { ALLOCATOR.lock().init(&raw mut HEAP as *mut u8, HEAP_SIZE) };

    let mut uart0_writer: UartWriter = UartWriter { uart: &UART0 };
    init_base_addresses(dtb_ptr, &mut uart0_writer);

    write!(
        uart0_writer,
        "{}Type a line; use Left/Right/Delete/Backspace to edit; Enter to submit.\r\n{}",
        AnsiEscape::GREEN,
        AnsiEscape::RESET
    )
    .unwrap_or(());

    gic_init();
    UART0.enable_rx_interrupt();
    DAIF.write(DAIF::I::CLEAR);

    loop {
        unsafe { core::arch::asm!("wfe") };
    }
}

/// Initialize the Generic Interrupt Controller (GIC), enabling both UART0's
/// receive interrupt and the timer's -- the timer at higher priority for the
/// same reason as `r03_timer`: it must never be starved by a pending/
/// in-progress UART0 interrupt while an escape-sequence timeout is running.
fn gic_init() {
    let gicd = BASE_ADDRESSES.get_gicd();
    let gicc = BASE_ADDRESSES.get_gicc();
    let mut gic = unsafe { GicV2::new(gicd as *mut _, gicc as *mut _) };
    gic.setup();

    let uart_irq = IntId::spi(UART0.spi);
    let timer_irq = IntId::ppi(timer::PPI);

    gic.set_interrupt_priority(uart_irq, 0xa0);
    gic.set_interrupt_priority(timer_irq, 0x90);

    gic.enable_interrupt(uart_irq, true)
        .expect("failed to enable UART0 interrupt");
    gic.enable_interrupt(timer_irq, true)
        .expect("failed to enable timer interrupt");

    gic.set_priority_mask(0xff);
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

#[unsafe(no_mangle)]
extern "C" fn irq_handler() {
    let mut gic = unsafe {
        GicV2::new(
            BASE_ADDRESSES.get_gicd() as *mut _,
            BASE_ADDRESSES.get_gicc() as *mut _,
        )
    };

    if let Some(intid) = gic.get_and_acknowledge_interrupt(InterruptGroup::Group0) {
        if intid == IntId::spi(UART0.spi) {
            handle_uart_irq();
        } else if intid == IntId::ppi(timer::PPI) {
            handle_timer_irq();
        }
        gic.end_interrupt(intid, InterruptGroup::Group0);
    }
}

/// Handles a UART0 receive interrupt: feeds every available byte through
/// the escape-sequence parser.
fn handle_uart_irq() {
    // SAFETY: see EDITOR's doc comment -- at most one irq_handler runs at a
    // time, so this can't race any other access.
    #[allow(clippy::deref_addrof)]
    let ed = unsafe { &mut *(&raw mut EDITOR) };
    while let Some(c) = UART0.try_getc() {
        let (tokens, next) = keyboard::parse(Some(c), ed.parser);
        ed.parser = next;
        dispatch_tokens(ed, &tokens);
        sync_timer(next);
    }
    UART0.clear_rx_interrupt();
}

/// Handles a timer interrupt: fires only while a possible escape sequence
/// is pending (every other path disables the timer before returning to
/// `Ground` -- see `sync_timer`), so feed the parser a timeout (`None`) to
/// resolve whatever was pending, dispatch what that produces, and stop the
/// timer. Leaving it armed here would re-assert the same expired condition
/// forever, the same interrupt-storm bug the original C project hit from a
/// missing timer re-arm.
fn handle_timer_irq() {
    timer::disable();
    #[allow(clippy::deref_addrof)]
    let ed = unsafe { &mut *(&raw mut EDITOR) };
    let (tokens, next) = keyboard::parse(None, ed.parser);
    ed.parser = next; // always Ground after a timeout resolves, per keyboard.rs's policy
    dispatch_tokens(ed, &tokens);
}

/// Arms the bounded escape-sequence timer whenever the parser is waiting on
/// more bytes, and disables it once back in `Ground` -- the half of
/// `keyboard.rs`'s termination contract that belongs to whoever wires it up
/// to real hardware (see that module's doc comment).
fn sync_timer(state: ParserState) {
    if state == ParserState::Ground {
        timer::disable();
    } else {
        timer::arm(esc_timeout_ticks());
    }
}

/// Turns each recognized `Token` into the corresponding line-editing action.
/// Deliberately an exhaustive match, not a wildcard catch-all: adding a new
/// `Token` variant in `keyboard.rs` should force a decision here about what
/// it does, rather than silently falling through to "discard."
fn dispatch_tokens(ed: &mut Editor, tokens: &Tokens) {
    for tok in tokens.iter() {
        match tok {
            Token::Char(c) => insert_char(ed, c as u8),
            Token::Enter => finish_line(ed),
            Token::Backspace => backspace(ed),
            Token::Delete => delete_at_cursor(ed),
            Token::ArrowLeft => cursor_left(ed),
            Token::ArrowRight => cursor_right(ed),
            Token::Home => cursor_home(ed),
            Token::End => cursor_end(ed),
            // A standalone Escape: shown in standard caret notation, same
            // as r02_interrupts/r04_alloc, confirmed against a real `sh`.
            Token::Escape => insert_str(ed, "^["),
            // Reconstructs the bytes actually consumed (`^[` + the
            // triggering character), matching the "visible or nothing"
            // principle this stage was built around -- see keyboard.rs's
            // module doc comment for why `Alt` shows up here at all rather
            // than a specific action.
            Token::Alt(c) => {
                insert_str(ed, "^[");
                insert_char(ed, c as u8);
            }
            // Recognized, but not actionable yet: history navigation is
            // Stage 11's job (ArrowUp/ArrowDown); Insert/PageUp/PageDown/Tab
            // have no assigned meaning in this stage; Fn/Ctrl are tracked by
            // the parser but nothing here binds them to anything (yet).
            Token::ArrowUp
            | Token::ArrowDown
            | Token::Insert
            | Token::PageUp
            | Token::PageDown
            | Token::Tab
            | Token::Fn(_)
            | Token::Ctrl(_) => {}
        }
    }
}

/// Inserts `c` at the cursor, then redraws everything from the insertion
/// point onward (the new character plus whatever was shifted right), and
/// repositions the terminal cursor back to sit right after what was typed.
fn insert_char(ed: &mut Editor, c: u8) {
    let from = ed.cursor;
    ed.buf.insert(ed.cursor, c);
    ed.cursor += 1;
    redraw(ed, from, 0);
}

/// Erases the character before the cursor, if any -- otherwise does nothing
/// (there's nothing before the cursor to erase, and "nothing" is itself the
/// correct, immediate, visible-or-nothing response here).
fn backspace(ed: &mut Editor) {
    if ed.cursor == 0 {
        return;
    }
    ed.buf.remove(ed.cursor - 1);
    ed.cursor -= 1;
    UART0.puts("\x08"); // move onto the deleted character's old column
    redraw(ed, ed.cursor, 1);
}

/// Erases the character under the cursor, if any. Meaningful now that a
/// real cursor exists (unlike `r04_alloc`'s always-at-the-end model, where
/// forward-delete had nothing to act on).
fn delete_at_cursor(ed: &mut Editor) {
    if ed.cursor >= ed.buf.len() {
        return;
    }
    ed.buf.remove(ed.cursor);
    redraw(ed, ed.cursor, 1);
}

fn cursor_left(ed: &mut Editor) {
    if ed.cursor == 0 {
        return;
    }
    ed.cursor -= 1;
    UART0.puts("\x1b[D");
}

fn cursor_right(ed: &mut Editor) {
    if ed.cursor >= ed.buf.len() {
        return;
    }
    ed.cursor += 1;
    UART0.puts("\x1b[C");
}

fn cursor_home(ed: &mut Editor) {
    if ed.cursor == 0 {
        return;
    }
    let back = ed.cursor;
    ed.cursor = 0;
    let mut w = UartWriter { uart: &UART0 };
    write!(w, "\x1b[{back}D").unwrap_or(());
}

fn cursor_end(ed: &mut Editor) {
    if ed.cursor >= ed.buf.len() {
        return;
    }
    let forward = ed.buf.len() - ed.cursor;
    ed.cursor = ed.buf.len();
    let mut w = UartWriter { uart: &UART0 };
    write!(w, "\x1b[{forward}C").unwrap_or(());
}

/// Reprints `buf[from..]` (assuming the terminal's own cursor is already
/// sitting at column `from`), followed by `erase_extra` trailing spaces (to
/// erase leftover glyphs from a now-shorter line), then moves the terminal
/// cursor back to align with `ed.cursor` -- the general redraw technique
/// behind every insert/delete above.
fn redraw(ed: &Editor, from: usize, erase_extra: usize) {
    UART0.puts(str::from_utf8(&ed.buf[from..]).unwrap_or(""));
    for _ in 0..erase_extra {
        UART0.putc(b' ');
    }
    let printed = (ed.buf.len() - from) + erase_extra;
    let back = printed - (ed.cursor - from);
    if back > 0 {
        let mut w = UartWriter { uart: &UART0 };
        write!(w, "\x1b[{back}D").unwrap_or(());
    }
}

/// Inserts each byte of `s` at the cursor in turn, via `insert_char`.
fn insert_str(ed: &mut Editor, s: &str) {
    for &b in s.as_bytes() {
        insert_char(ed, b);
    }
}

fn finish_line(ed: &mut Editor) {
    UART0.puts("\r\n");
    UART0.puts(str::from_utf8(&ed.buf).unwrap_or("<invalid utf-8>"));
    UART0.puts("\r\n");
    ed.buf.clear();
    ed.cursor = 0;
}

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
    )
    .unwrap_or(());
    hang()
}

fn hang() -> ! {
    loop {
        unsafe { core::arch::asm!("wfe") };
    }
}
