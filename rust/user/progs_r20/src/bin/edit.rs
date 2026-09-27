//! `edit FILE` -- Stage 20's editor. Step 5: open a file (a missing one is a new buffer), view it,
//! scroll, quit; `.editrc` loaded. No typing yet (Step 6), so nothing can become modified and `^X`
//! always just exits -- and no status bar or help footer either: the whole screen is the text area
//! until Step 6 carves rows out of it for those.
//!
//! Reads keys with `CONSOLE_READ_KEY` and draws whole frames with `CONSOLE_DRAW` (see
//! `abi::ioctl`); nothing here is a raw-mode toggle on `read(0)`; the two are independent readers
//! of the same token queue (`keyboard/stdin.rs`'s doc comment).

#![no_std]
#![no_main]

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;
use core::fmt::Write;

use abi::errno::ENOENT;
use abi::ioctl::{CELL_SIZE, CONSOLE_DRAW_HEADER_SIZE, Cell, ConsoleDraw};
use abi::keys::{
    KEY_BACKSLASH, KEY_DOWN, KEY_END, KEY_HOME, KEY_LEFT, KEY_PAGEDOWN, KEY_PAGEUP, KEY_RIGHT,
    KEY_SLASH, KEY_UP, KEY_X,
};
use progs::{Fd, fail};
use progs_r20::buffer::Buffer;
use progs_r20::editrc::{self, Config};
use progs_r20::layout::{self, Row};
use progs_r20::render::render_row;
use progs_r20::scroll::{self, Top};
use userlib::{ExitCode, O_RDONLY, close, console_draw, env, open, read, read_key, winsize};

userlib::entry_with_env!(run);

const USAGE: &str = "usage: edit FILE";

fn run(mut args: userlib::Args) -> ExitCode {
    let _ = args.next(); // argv[0]
    let (Some(path), None) = (args.next(), args.next()) else {
        let _ = writeln!(Fd(2), "{USAGE}");
        return ExitCode(2);
    };

    let config = load_editrc();

    let mut buffer = match load_file(path) {
        Ok(buffer) => buffer,
        Err(code) => {
            fail("edit", path, code);
            return ExitCode(1);
        }
    };

    let Ok(size) = winsize(1) else {
        let _ = writeln!(Fd(2), "edit: not a terminal");
        return ExitCode(1);
    };
    let (rows, cols) = (usize::from(size.rows), usize::from(size.cols));
    let tab_size = usize::from(config.tab_size);

    let mut top = Top::default();
    let mut preferred_col: Option<usize> = None;
    loop {
        top = scroll::scroll_to_cursor(&buffer, top, rows, cols, tab_size);
        draw(&buffer, top, rows, cols, tab_size);

        let Ok(event) = read_key(0) else {
            return ExitCode(1); // stdin isn't the keyboard -- nothing this program can do about it
        };
        let code = event.effective_code();

        // Every key below except Up/Down keeps the buffer's own column, not a remembered one.
        if code != KEY_UP && code != KEY_DOWN {
            preferred_col = None;
        }

        match code {
            // Ctrl+Left/Right (word movement) arrives in Step 6 with the rest of typing's ctrl
            // combinations; for now a plain move is what both keys do.
            KEY_LEFT => {
                buffer.move_left();
            }
            KEY_RIGHT => {
                buffer.move_right();
            }
            KEY_UP => {
                scroll::move_vertical(&mut buffer, true, &mut preferred_col, cols, tab_size);
            }
            KEY_DOWN => {
                scroll::move_vertical(&mut buffer, false, &mut preferred_col, cols, tab_size);
            }
            KEY_HOME => {
                buffer.move_home();
            }
            KEY_END => {
                buffer.move_end();
            }
            KEY_PAGEUP => page(&mut buffer, &mut preferred_col, true, rows, cols, tab_size),
            KEY_PAGEDOWN => page(&mut buffer, &mut preferred_col, false, rows, cols, tab_size),
            KEY_BACKSLASH if event.alt() => {
                buffer.move_to_first_line();
            }
            KEY_SLASH if event.alt() => {
                buffer.move_to_last_line();
            }
            KEY_X if event.ctrl() && !event.repeat() => break,
            _ => {}
        }
    }
    ExitCode(0)
}

/// Moves the cursor a screenful of rows up or down, one row at a time (so it still lands correctly
/// inside a line taller than the screen, or crosses several short ones) -- `^Y`/`^V`.
fn page(
    buffer: &mut Buffer,
    preferred_col: &mut Option<usize>,
    up: bool,
    rows: usize,
    cols: usize,
    tab_size: usize,
) {
    for _ in 0..rows {
        if !scroll::move_vertical(buffer, up, preferred_col, cols, tab_size) {
            break; // already at the very first or last row
        }
    }
}

/// Reads `$HOME/.editrc`, if `$HOME` is set and the file exists; any problem it reports goes to
/// stderr for now (Step 6 gives the editor its own message line to show them on instead).
fn load_editrc() -> Config {
    let Some(home) = env::var("HOME") else {
        return Config::default();
    };
    let mut path = String::from(home);
    path.push_str("/.editrc");

    let fd = open(&path, O_RDONLY);
    if fd < 0 {
        return Config::default(); // missing (or unreadable): silent, every default applies
    }
    let bytes = read_whole(fd as usize);
    close(fd as usize);
    let text = String::from_utf8_lossy(&bytes);
    let parsed = editrc::parse(&text);
    for problem in &parsed.problems {
        let _ = writeln!(Fd(2), "edit: {path}:{}: {}", problem.line, problem.why);
    }
    parsed.config
}

/// Opens `path` and reads it whole, as LF-only text (Stage 20's plan: lossy, not a validity check --
/// carriage returns are dropped and invalid UTF-8 replaced, exactly as `String::from_utf8_lossy`
/// already does for the latter). A missing file is `Ok`, a fresh empty buffer -- everything else is
/// the syscall's own error.
fn load_file(path: &str) -> Result<Buffer, isize> {
    let fd = open(path, O_RDONLY);
    if fd == ENOENT {
        return Ok(Buffer::new());
    }
    if fd < 0 {
        return Err(fd);
    }
    let bytes = read_whole(fd as usize);
    close(fd as usize);
    let text = String::from_utf8_lossy(&bytes).replace('\r', "");
    Ok(Buffer::from_text(&text))
}

/// Reads all of `fd`'s remaining bytes into one `Vec` -- `edit` needs the whole file in memory
/// (Stage 16's heap is what makes that a program-window-independent choice; Stage 15's growable
/// window is what let the heap exist at all), unlike a streaming reader such as `cat`'s.
fn read_whole(fd: usize) -> Vec<u8> {
    let mut out = Vec::new();
    let mut chunk = [0u8; progs::CHUNK];
    loop {
        let n = read(fd, &mut chunk);
        if n <= 0 {
            return out;
        }
        out.extend_from_slice(&chunk[..n as usize]);
    }
}

/// Builds one whole frame from what `top` makes visible and sends it with one `CONSOLE_DRAW`. The
/// cursor's screen row is found by matching its own `Row` (from `layout::wrap_line`, which
/// `visible_rows` also built its list from) against that same list, rather than re-deriving it from
/// `top` by another route that could disagree with what was actually drawn.
fn draw(buffer: &Buffer, top: Top, rows: usize, cols: usize, tab_size: usize) {
    let (cursor_line, cursor_byte) = buffer.cursor();
    let (cursor_screen_row, cursor_col) =
        layout::position_to_cell(buffer.line(cursor_line), cursor_byte, cols, tab_size);
    let cursor_target: Row =
        layout::wrap_line(buffer.line(cursor_line), cols, tab_size)[cursor_screen_row];

    let mut cells = Vec::with_capacity(rows * cols);
    let mut cursor_row = 0;
    for (screen_row, (line, row)) in scroll::visible_rows(buffer, top, rows, cols, tab_size)
        .into_iter()
        .enumerate()
    {
        if line == cursor_line && row == cursor_target {
            cursor_row = screen_row;
        }
        render_row(&mut cells, buffer.line(line), row, cols, tab_size);
    }
    cells.resize(rows * cols, Cell::plain(' ')); // past the end of the buffer: blank rows, nano-style

    // The visible cursor: an ordinary cell, drawn inverse -- there is no separate cursor primitive
    // (`ConsoleDraw`'s own `cursor_row`/`cursor_col` only reposition the console's *bookkeeping*
    // cursor, for wherever the shell's next prompt starts once this program exits; see Step 3's
    // notes in `Stage20.md`). Always in range: `position_to_cell` never returns a column equal to
    // `cols` (a fully-packed row's one-past-the-end position always belongs to a *different* row,
    // by `layout`'s own trailing-empty-row rule), so the index below always lands inside `cells`.
    if let Some(cursor_cell) = cells.get_mut(cursor_row * cols + cursor_col) {
        cursor_cell.attr |= abi::ioctl::ATTR_INVERSE;
    }

    let header = ConsoleDraw {
        rows: rows as u16,
        cols: cols as u16,
        cursor_row: cursor_row as u16,
        cursor_col: cursor_col as u16,
    };
    let mut frame = Vec::with_capacity(CONSOLE_DRAW_HEADER_SIZE + cells.len() * CELL_SIZE);
    frame.extend_from_slice(&header.encode());
    for cell in &cells {
        frame.extend_from_slice(&cell.encode());
    }
    let _ = console_draw(1, &frame);
}
