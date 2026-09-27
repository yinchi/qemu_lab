//! `edit FILE` -- Stage 20's editor. Step 6: typing and deleting, save (`^S`) and save-as (`^O`), the
//! status bar, the message/prompt row, exiting with unsaved changes, and the `^G` help screen. Rows
//! 0..R-2 are the text area; row R-2 is the inverse-video status bar; row R-1 is the help footer,
//! or a transient message, or the one-line prompt widget, whichever is current.
//!
//! Reads keys with `CONSOLE_READ_KEY` and draws whole frames with `CONSOLE_DRAW` (see
//! `abi::ioctl`); nothing here is a raw-mode toggle on `read(0)`; the two are independent readers
//! of the same token queue (`keyboard/stdin.rs`'s doc comment). Exiting restores whatever was on
//! screen before this program's first frame (Step 5b's console "alternate screen").

#![no_std]
#![no_main]

extern crate alloc;

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::fmt::Write;

use abi::errno::ENOENT;
use abi::ioctl::{ATTR_INVERSE, CELL_SIZE, CONSOLE_DRAW_HEADER_SIZE, Cell, ConsoleDraw};
use abi::keys::{
    KEY_BACKSLASH, KEY_BACKSPACE, KEY_C, KEY_DELETE, KEY_DOWN, KEY_END, KEY_ENTER, KEY_ESC, KEY_G,
    KEY_HOME, KEY_LEFT, KEY_O, KEY_PAGEDOWN, KEY_PAGEUP, KEY_RIGHT, KEY_S, KEY_SLASH, KEY_UP,
    KEY_X,
};
use progs::{Fd, fail, write_all};
use progs_r20::buffer::Buffer;
use progs_r20::editrc::{self, Config};
use progs_r20::layout::{self, Row};
use progs_r20::prompt::Prompt;
use progs_r20::render::render_row;
use progs_r20::scroll::{self, Top};
use userlib::{
    ExitCode, O_RDONLY, O_WRONLY, close, console_draw, env, open, read, read_key, winsize,
};

userlib::entry_with_env!(run);

const USAGE: &str = "usage: edit FILE";

/// Rows reserved below the text area: the status bar, then the footer/message/prompt row.
const RESERVED_ROWS: usize = 2;

/// The screen's shape: `TIOCGWINSZ`'s rows and columns, plus the configured tab stop width -- the
/// three unchanging inputs `draw` needs alongside the buffer and overlay state, bundled together
/// only so `draw` doesn't take eight separate arguments.
#[derive(Clone, Copy)]
struct Screen {
    rows: usize,
    cols: usize,
    tab_size: usize,
}

/// What the bottom row (and, for `Help`, the whole screen) currently shows instead of plain editing.
enum Overlay {
    /// The help footer, or a transient `message` if one is set.
    None,
    /// A one-line question -- Save As, so far -- with `Enter` to confirm and `Esc` to cancel.
    Prompt(Prompt, PromptPurpose),
    /// "Save modified buffer? (Y/N/Cancel)" -- `^X` on a modified buffer.
    Confirm,
    /// The full-screen key list `^G` shows; any key returns to `None`.
    Help,
}

enum PromptPurpose {
    SaveAs,
}

fn run(mut args: userlib::Args) -> ExitCode {
    let _ = args.next(); // argv[0]
    let (Some(path), None) = (args.next(), args.next()) else {
        let _ = writeln!(Fd(2), "{USAGE}");
        return ExitCode(2);
    };

    let (config, mut message) = load_editrc();

    let mut filename = path.to_string();
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
    let text_height = rows.saturating_sub(RESERVED_ROWS).max(1);
    let tab_size = usize::from(config.tab_size);
    let auto_indent = config.auto_indent;

    let mut top = Top::default();
    let mut preferred_col: Option<usize> = None;
    let mut overlay = Overlay::None;

    let screen = Screen {
        rows,
        cols,
        tab_size,
    };

    loop {
        top = scroll::scroll_to_cursor(&buffer, top, text_height, cols, tab_size);
        draw(
            &buffer,
            top,
            screen,
            &filename,
            &overlay,
            message.as_deref(),
        );

        let Ok(event) = read_key(0) else {
            return ExitCode(1); // stdin isn't the keyboard -- nothing this program can do about it
        };
        let code = event.effective_code();
        message = None; // a message is shown for exactly one frame, then cleared on the next key

        match &mut overlay {
            Overlay::Help => {
                overlay = Overlay::None;
            }

            Overlay::Confirm => match event.char() {
                Some('y' | 'Y') => match try_save(&buffer, &filename) {
                    Ok(_) => break,
                    Err(e) => {
                        message = Some(format!("Error writing {filename}: {}", progs::errmsg(e)));
                        overlay = Overlay::None;
                    }
                },
                Some('n' | 'N') => break,
                _ if code == KEY_ESC => overlay = Overlay::None,
                _ => {}
            },

            Overlay::Prompt(prompt, purpose) => match code {
                KEY_ESC => overlay = Overlay::None,
                KEY_ENTER => {
                    let answer = prompt.input().to_string();
                    match purpose {
                        PromptPurpose::SaveAs => {
                            if answer.is_empty() {
                                overlay = Overlay::None;
                            } else {
                                match try_save(&buffer, &answer) {
                                    Ok(lines) => {
                                        filename = answer;
                                        buffer.mark_saved();
                                        message = Some(format!("Wrote {lines} lines"));
                                    }
                                    Err(e) => {
                                        message = Some(format!(
                                            "Error writing {answer}: {}",
                                            progs::errmsg(e)
                                        ));
                                    }
                                }
                                overlay = Overlay::None;
                            }
                        }
                    }
                }
                KEY_LEFT => {
                    prompt.move_left();
                }
                KEY_RIGHT => {
                    prompt.move_right();
                }
                KEY_HOME => {
                    prompt.move_home();
                }
                KEY_END => {
                    prompt.move_end();
                }
                KEY_BACKSPACE => {
                    prompt.backspace();
                }
                KEY_DELETE => {
                    prompt.delete_forward();
                }
                _ if !event.ctrl() && !event.alt() => {
                    if let Some(c) = event.char()
                        && !c.is_control()
                    {
                        prompt.insert_char(c);
                    }
                }
                _ => {}
            },

            Overlay::None => {
                if code != KEY_UP && code != KEY_DOWN {
                    preferred_col = None;
                }
                match code {
                    // Word movement, before the plain-arrow arms below, which would otherwise also
                    // match a Ctrl-held one (`match` takes the first arm that fits the value alone;
                    // the guard is what makes this one win only when Ctrl is actually held).
                    KEY_LEFT if event.ctrl() => {
                        buffer.move_word_left();
                    }
                    KEY_RIGHT if event.ctrl() => {
                        buffer.move_word_right();
                    }
                    KEY_LEFT => {
                        buffer.move_left();
                    }
                    KEY_RIGHT => {
                        buffer.move_right();
                    }
                    KEY_UP => {
                        scroll::move_vertical(
                            &mut buffer,
                            true,
                            &mut preferred_col,
                            cols,
                            tab_size,
                        );
                    }
                    KEY_DOWN => {
                        scroll::move_vertical(
                            &mut buffer,
                            false,
                            &mut preferred_col,
                            cols,
                            tab_size,
                        );
                    }
                    KEY_HOME => {
                        buffer.move_home();
                    }
                    KEY_END => {
                        buffer.move_end();
                    }
                    KEY_PAGEUP => page(
                        &mut buffer,
                        &mut preferred_col,
                        true,
                        text_height,
                        cols,
                        tab_size,
                    ),
                    KEY_PAGEDOWN => page(
                        &mut buffer,
                        &mut preferred_col,
                        false,
                        text_height,
                        cols,
                        tab_size,
                    ),
                    KEY_BACKSLASH if event.alt() => {
                        buffer.move_to_first_line();
                    }
                    KEY_SLASH if event.alt() => {
                        buffer.move_to_last_line();
                    }
                    KEY_BACKSPACE => {
                        buffer.backspace();
                    }
                    KEY_DELETE => {
                        buffer.delete_forward();
                    }
                    KEY_ENTER => {
                        buffer.split_line(auto_indent);
                    }
                    KEY_S if event.ctrl() && !event.repeat() => {
                        match try_save(&buffer, &filename) {
                            Ok(lines) => {
                                buffer.mark_saved();
                                message = Some(format!("Wrote {lines} lines"));
                            }
                            Err(e) => {
                                message =
                                    Some(format!("Error writing {filename}: {}", progs::errmsg(e)));
                            }
                        }
                    }
                    KEY_O if event.ctrl() && !event.repeat() => {
                        overlay = Overlay::Prompt(
                            Prompt::new("File Name to Write: ", &filename),
                            PromptPurpose::SaveAs,
                        );
                    }
                    KEY_G if event.ctrl() && !event.repeat() => {
                        overlay = Overlay::Help;
                    }
                    KEY_C if event.ctrl() && !event.repeat() => {
                        message = Some(position_message(&buffer));
                    }
                    KEY_X if event.ctrl() && !event.repeat() => {
                        if buffer.is_modified() {
                            overlay = Overlay::Confirm;
                        } else {
                            break;
                        }
                    }
                    _ if !event.ctrl() && !event.alt() => {
                        if let Some(c) = event.char()
                            && (!c.is_control() || c == '\t')
                        {
                            buffer.insert_char(c);
                        }
                    }
                    _ => {}
                }
            }
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

/// Truncates `path` and writes the whole buffer to it (Stage 20's chosen save method: in place, no
/// temp file). Returns the number of lines written, or the negative error from whichever syscall
/// failed first -- `open`, the write itself, or `close` (a full disk can surface there instead).
fn try_save(buffer: &Buffer, path: &str) -> Result<usize, isize> {
    let fd = open(path, O_WRONLY);
    if fd < 0 {
        return Err(fd);
    }
    let fd = fd as usize;
    let text = buffer.to_text();
    let write_result = write_all(fd, text.as_bytes());
    let close_result = close(fd);
    write_result?;
    if close_result < 0 {
        return Err(close_result);
    }
    Ok(buffer.line_count())
}

/// Reads `$HOME/.editrc`, if `$HOME` is set and the file exists. The first problem it reports, if
/// any, comes back as a message for the message row's very first frame; the rest are silently
/// applied (their own setting keeps its default) but not individually shown -- one line is what the
/// row has room for.
fn load_editrc() -> (Config, Option<String>) {
    let Some(home) = env::var("HOME") else {
        return (Config::default(), None);
    };
    let mut path = String::from(home);
    path.push_str("/.editrc");

    let fd = open(&path, O_RDONLY);
    if fd < 0 {
        return (Config::default(), None); // missing (or unreadable): silent, every default applies
    }
    let bytes = read_whole(fd as usize);
    close(fd as usize);
    let text = String::from_utf8_lossy(&bytes);
    let parsed = editrc::parse(&text);
    let message = parsed.problems.first().map(|first| {
        let more = parsed.problems.len() - 1;
        let suffix = if more > 0 {
            format!(" (+{more} more)")
        } else {
            String::new()
        };
        format!("{path}:{}: {}{suffix}", first.line, first.why)
    });
    (parsed.config, message)
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

/// `^C`'s message: the cursor's line and column (both 1-based; column counts characters, not display
/// columns, so a line with tabs or wide glyphs still gets a plain count).
fn position_message(buffer: &Buffer) -> String {
    let (line, byte) = buffer.cursor();
    let column = buffer.line(line)[..byte].chars().count() + 1;
    format!(
        "line {} of {}, column {column}",
        line + 1,
        buffer.line_count()
    )
}

const HELP_FOOTER: &str = "^G Help  ^O WriteOut  ^S Save  ^X Exit";

const HELP_TEXT: &[&str] = &[
    "edit -- A simple text editor",
    "",
    "Arrows, Home/End       move by character / logical line",
    "Ctrl+Left/Right        move by word",
    "PgUp/PgDn              move a screenful",
    "Alt+\\ / Alt+/          first / last line",
    "",
    "^S    Save",
    "^O    Write Out (save as)",
    "^X    Exit (asks first if modified)",
    "^C    Show the cursor's line and column",
    "^G    This help screen",
    "Esc   Cancel a prompt",
    "",
    "Press any key to continue",
];

/// Pushes `text` as `cols` cells, one column per character (plain ASCII status/footer/prompt text,
/// not the width-aware text area), all with `attr`, truncated or padded with blanks to fit exactly.
fn push_line(cells: &mut Vec<Cell>, cols: usize, text: &str, attr: u8) {
    let mut count = 0;
    for c in text.chars().take(cols) {
        cells.push(Cell { ch: c as u32, attr });
        count += 1;
    }
    for _ in count..cols {
        cells.push(Cell {
            ch: ' ' as u32,
            attr,
        });
    }
}

/// The status bar: `name [Modified]   Ln x/y, Col z`, drawn inverse across the whole row.
fn render_status_bar(cells: &mut Vec<Cell>, cols: usize, filename: &str, buffer: &Buffer) {
    let (line, byte) = buffer.cursor();
    let modified = if buffer.is_modified() {
        " [Modified]"
    } else {
        ""
    };
    let column = buffer.line(line)[..byte].chars().count() + 1;
    let text = format!(
        "{filename}{modified}   Ln {}/{}, Col {column}",
        line + 1,
        buffer.line_count()
    );
    push_line(cells, cols, &text, ATTR_INVERSE);
}

/// The prompt row: `label` then the input typed so far, plain. Returns the column the cursor
/// belongs at (the label's width plus how far into the input it is).
fn render_prompt_row(cells: &mut Vec<Cell>, cols: usize, prompt: &Prompt) -> usize {
    let text = format!("{}{}", prompt.label(), prompt.input());
    push_line(cells, cols, &text, 0);
    prompt.label().chars().count() + prompt.input()[..prompt.cursor()].chars().count()
}

/// The bottom row when it isn't a prompt: the help footer, a transient message, or (during
/// `Overlay::Confirm`) the save-modified question -- plain text, one line.
fn render_footer(cells: &mut Vec<Cell>, cols: usize, text: &str) {
    push_line(cells, cols, text, 0);
}

/// Draws the `^G` help screen: `HELP_TEXT`, one line per row, cursor parked at the top left (there
/// is nothing here to place it meaningfully on).
fn draw_help(rows: usize, cols: usize) {
    let mut cells = Vec::with_capacity(rows * cols);
    for row in 0..rows {
        let text = HELP_TEXT.get(row).copied().unwrap_or("");
        render_row(
            &mut cells,
            text,
            Row {
                start: 0,
                end: text.len(),
            },
            cols,
            8,
        );
    }
    send_frame(&cells, rows, cols, 0, 0);
}

/// Encodes `cells` behind a `ConsoleDraw` header and sends them in one `CONSOLE_DRAW` call.
fn send_frame(cells: &[Cell], rows: usize, cols: usize, cursor_row: u16, cursor_col: u16) {
    let header = ConsoleDraw {
        rows: rows as u16,
        cols: cols as u16,
        cursor_row,
        cursor_col,
    };
    let mut frame = Vec::with_capacity(CONSOLE_DRAW_HEADER_SIZE + cells.len() * CELL_SIZE);
    frame.extend_from_slice(&header.encode());
    for cell in cells {
        frame.extend_from_slice(&cell.encode());
    }
    let _ = console_draw(1, &frame);
}

/// Builds one whole frame and sends it. `Overlay::Help` replaces the entire screen; otherwise the
/// text area (`rows - RESERVED_ROWS` rows), the status bar, and the footer/message/prompt row. The
/// cursor's screen row is found by matching its exact `Row` (from `layout::wrap_line`, which
/// `visible_rows` also built its list from) against that list, rather than re-deriving it a second
/// way that could disagree with what was actually drawn.
fn draw(
    buffer: &Buffer,
    top: Top,
    screen: Screen,
    filename: &str,
    overlay: &Overlay,
    message: Option<&str>,
) {
    let Screen {
        rows,
        cols,
        tab_size,
    } = screen;
    if let Overlay::Help = overlay {
        draw_help(rows, cols);
        return;
    }

    let text_height = rows.saturating_sub(RESERVED_ROWS).max(1);
    let (cursor_line, cursor_byte) = buffer.cursor();
    let (cursor_screen_row, cursor_col) =
        layout::position_to_cell(buffer.line(cursor_line), cursor_byte, cols, tab_size);
    let cursor_target: Row =
        layout::wrap_line(buffer.line(cursor_line), cols, tab_size)[cursor_screen_row];

    let mut cells = Vec::with_capacity(rows * cols);
    let mut cursor_row = 0;
    for (screen_row, (line, row)) in scroll::visible_rows(buffer, top, text_height, cols, tab_size)
        .into_iter()
        .enumerate()
    {
        if line == cursor_line && row == cursor_target {
            cursor_row = screen_row;
        }
        render_row(&mut cells, buffer.line(line), row, cols, tab_size);
    }
    cells.resize(text_height * cols, Cell::plain(' ')); // past the end of the buffer: blank rows

    render_status_bar(&mut cells, cols, filename, buffer);

    let (cursor_row, cursor_col) = match overlay {
        Overlay::Prompt(prompt, _) => {
            let col = render_prompt_row(&mut cells, cols, prompt);
            (text_height + 1, col)
        }
        Overlay::Confirm => {
            render_footer(&mut cells, cols, "Save modified buffer? (Y/N/Cancel)");
            (cursor_row, cursor_col)
        }
        Overlay::None => {
            render_footer(&mut cells, cols, message.unwrap_or(HELP_FOOTER));
            (cursor_row, cursor_col)
        }
        Overlay::Help => unreachable!("handled above"),
    };

    if let Some(cell) = cells.get_mut(cursor_row * cols + cursor_col) {
        cell.attr |= ATTR_INVERSE;
    }

    send_frame(&cells, rows, cols, cursor_row as u16, cursor_col as u16);
}
