//! A software text console: cursor-addressable character cells rendered as glyphs into a raw
//! pixel framebuffer.
//!
//! The display's resolution is negotiated with virtio-gpu, not hardware-fixed the way a real CRT
//! would be -- but nothing in this project needs runtime resizing, so it's treated as fixed by
//! convention: chosen once at startup, with `cols`/`rows` plain values derived from it rather
//! than something callers re-query.

pub mod cp437;
pub mod font;
pub mod framebuffer;

use cp437::unicode_to_cp437;
use font::{Font, GLYPH_HEIGHT, GLYPH_WIDTH};
use framebuffer::Framebuffer;

// ARGB colors (virtio-gpu's negotiated format -- see drivers/virtio/gpu.rs): alpha is a real
// channel here, not padding, so it must be opaque (0xFF) or the compositor may treat these pixels
// as transparent. `syscall/fd.rs`'s Console write path (a running program's stdout) draws in these
// same colors.
pub const FG: u32 = 0xFF55FF55;
pub const BG: u32 = 0xFF00_0000;

/// Column spacing for `\t`, matching the traditional terminal default.
const TAB_WIDTH: usize = 8;

/// A software text console: cursor-addressable character cells rendered as glyphs into a raw
/// pixel framebuffer.
pub struct Console<'a> {
    /// The underlying pixel framebuffer.
    fb: Framebuffer,
    /// The font used to render character glyphs, backed by a monochrome bitmap as a raw byte array.
    font: Font<'a>,
    /// Number of character columns in the console.
    pub cols: usize,
    /// Number of character rows in the console.
    pub rows: usize,
    /// The current cursor row.
    cursor_row: usize,
    /// The current cursor column.
    cursor_col: usize,
}

impl<'a> Console<'a> {
    pub fn new(fb: Framebuffer, font: Font<'a>) -> Self {
        let cols = fb.width / GLYPH_WIDTH;
        let rows = fb.height / GLYPH_HEIGHT;
        Self {
            fb,
            font,
            cols,
            rows,
            cursor_row: 0,
            cursor_col: 0,
        }
    }

    /// Returns the screen size in character cells as (cols, rows).
    #[allow(dead_code)]
    pub fn size(&self) -> (usize, usize) {
        (self.cols, self.rows)
    }

    /// Places `ch` at (`row`, `col`) with the given foreground/background colors, without
    /// moving the cursor.
    pub fn put_char(&self, row: usize, col: usize, ch: u8, fg: u32, bg: u32) {
        // Ensure the specified row and column are within the console's bounds.
        assert!(row < self.rows && col < self.cols);

        // Fetch the glyph bitmap for the specified character.
        let glyph = self.font.glyph(ch);

        // Compute the top-left corner of the current character cell in the framebuffer.
        let x0 = col * GLYPH_WIDTH;
        let y0 = row * GLYPH_HEIGHT;

        // For each row of the glyph bitmap:
        for (dy, &row_bits) in glyph.iter().enumerate() {
            // For each bit in the current row of the glyph bitmap:
            for dx in 0..GLYPH_WIDTH {
                // Extract the bit corresponding to the current pixel within the glyph row
                // (a u8).
                let bit_set = (row_bits >> (7 - dx)) & 1 != 0;

                // Draw the pixel in the framebuffer with the appropriate color based on the bit.
                self.fb
                    .put_pixel(x0 + dx, y0 + dy, if bit_set { fg } else { bg });
            }
        }
    }

    /// Moves the cursor to the specified row and column.
    pub fn move_cursor(&mut self, row: usize, col: usize) {
        assert!(row < self.rows && col < self.cols);
        self.cursor_row = row;
        self.cursor_col = col;
    }

    /// The cursor's current (row, col) -- used to resync `main.rs`'s own `INPUT_ROW` after a
    /// program's output (`fd::write`'s `Console` case) has moved the cursor independently of
    /// anything the shell's own line-editing did.
    pub fn cursor(&self) -> (usize, usize) {
        (self.cursor_row, self.cursor_col)
    }

    /// Clears the whole screen to `bg`, an BGR color value.
    pub fn clear(&self, bg: u32) {
        for y in 0..self.fb.height {
            for x in 0..self.fb.width {
                self.fb.put_pixel(x, y, bg);
            }
        }
    }

    /// Clears one row of character cells to `bg`, an BGR color value.
    pub fn clear_row(&self, row: usize, bg: u32) {
        assert!(row < self.rows);
        for dy in 0..GLYPH_HEIGHT {
            for x in 0..self.fb.width {
                self.fb.put_pixel(x, row * GLYPH_HEIGHT + dy, bg);
            }
        }
    }

    /// Writes `ch` at the cursor and advances it, wrapping to the start of the next row at the
    /// end of a line -- scrolling first if that would run past the last one, the same as
    /// `write_char`'s `'\n'` case does, since a run of plain characters reaching the edge of a
    /// row is just as real an overflow as an explicit newline (a running EL0 program's output,
    /// via `fd::write`, has no reason to stay within a single row the way `show_row`'s
    /// pre-truncated prompt line does).
    pub fn putc(&mut self, ch: u8, fg: u32, bg: u32) {
        self.put_char(self.cursor_row, self.cursor_col, ch, fg, bg);
        self.cursor_col += 1;
        if self.cursor_col >= self.cols {
            self.cursor_col = 0;
            if self.cursor_row >= self.rows - 1 {
                self.scroll_up(bg);
            } else {
                self.cursor_row += 1;
            }
        }
    }

    /// Scrolls the console up by one row.
    ///
    /// We don't have a character-based buffer, so scrolling involves moving the pixel data up
    /// by one row's height and clearing the last row.
    pub fn scroll_up(&mut self, bg: u32) {
        let row_height = GLYPH_HEIGHT;
        let fb_width = self.fb.width;
        let fb_height = self.fb.height;

        // Move the pixel data up by one row's height.
        for y in 0..(fb_height - row_height) {
            for x in 0..fb_width {
                self.fb
                    .put_pixel(x, y, self.fb.get_pixel(x, y + row_height));
            }
        }

        // Clear the last row.
        self.clear_row(self.rows - 1, bg);
    }

    /// Writes one character of text at the cursor, advancing it. `\r`/`\n`/`\t` move the cursor
    /// instead of drawing a glyph for them; every other control character (C0/C1/DEL) is
    /// excluded outright, rather than falling through to `unicode_to_cp437`'s `b'?'` fallback --
    /// that fallback exists for genuinely unrepresentable *printable* Unicode, not as a catch-all
    /// for control characters, which `cp437.rs`'s own doc comment already says is this
    /// function's job to intercept, not something to silently render as a visible glyph either
    /// way (a misleading CP437 picture-glyph, or an equally uninformative literal `?`).
    ///
    /// User or system programs can use this function directly if treating the console as a dumb
    /// terminal, but for more advanced terminal handling (e.g. a TUI), they might want to manage
    /// cursor movement and screen updates themselves.
    pub fn write_char(&mut self, c: char, fg: u32, bg: u32) {
        match c {
            '\n' => {
                // Treat newlines as \r\n, moving the cursor to the start of the next line.
                self.cursor_col = 0;

                // If we are already on the last row, scroll up to make room for the new line.
                if self.cursor_row >= self.rows - 1 {
                    self.scroll_up(bg);
                }

                // Move the cursor to the start of the next line.
                self.cursor_row = (self.cursor_row + 1).min(self.rows - 1);
            }
            '\r' => self.cursor_col = 0,
            '\t' => {
                // Move the cursor to the next tab stop.
                let next_stop = (self.cursor_col / TAB_WIDTH + 1) * TAB_WIDTH;
                self.cursor_col = next_stop.min(self.cols - 1);
            }
            _ if c.is_control() => {}
            _ => self.putc(unicode_to_cp437(c), fg, bg),
        }
    }
}

/// Redraws one row with `prefix` followed by `line`, on the display only -- used for the
/// prompt+live-line row, redrawn on every keystroke, and for the fresh empty prompt drawn once a
/// line finishes. Display only: the UART hears the prompt and each *finished* line (see
/// `handle_keyboard_irq`), not a running echo of every keystroke.
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
pub fn show_row(console: &mut Console, row: usize, prefix: &str, line: &str) {
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
