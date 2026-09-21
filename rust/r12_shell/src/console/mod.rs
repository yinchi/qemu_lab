//! A software text console: cursor-addressable character cells rendered as glyphs into a raw
//! pixel framebuffer.
//!
//! The display's resolution is negotiated with virtio-gpu, not hardware-fixed the way a real CRT
//! would be -- but nothing in this project needs runtime resizing, so it's treated as fixed by
//! convention: chosen once at startup, with `cols`/`rows` plain values derived from it rather
//! than something callers re-query.

pub mod cells;
pub mod font;
pub mod framebuffer;
pub mod input_layout;
pub mod utf8;

use cells::{CellGrid, Cursor};
use font::{GLYPH_HEIGHT, GLYPH_WIDTH, glyph_for, is_zero_width};
use framebuffer::Framebuffer;
use unifont::Glyph;

// ARGB colors (virtio-gpu's negotiated format -- see drivers/virtio/gpu.rs): alpha is a real
// channel here, not padding, so it must be opaque (0xFF) or the compositor may treat these pixels
// as transparent. `syscall/fd.rs`'s Console write path (a running program's stdout) draws in these
// same colors.
pub const FG: u32 = 0xFF55FF55;
pub const BG: u32 = 0xFF00_0000;

/// Column spacing for `\t`, matching the traditional terminal default.
const TAB_WIDTH: usize = 8;

/// A software text console: cursor-addressable character cells rendered as glyphs into a raw
/// pixel framebuffer. A cell is 8x16 pixels; a wide glyph (CJK and the like) takes two adjacent cells.
pub struct Console {
    /// The underlying pixel framebuffer.
    fb: Framebuffer,
    /// Which cells hold half of a wide glyph -- the pixels alone can't say (see `cells.rs`).
    grid: CellGrid,
    /// Number of character columns in the console.
    pub cols: usize,
    /// Number of character rows in the console.
    pub rows: usize,
    /// The cursor, with xterm's deferred wrap: after a glyph fills the last column the cursor waits
    /// there and the wrap happens when the next glyph arrives (see `cells.rs`).
    cursor: Cursor,
}

impl Console {
    pub fn new(fb: Framebuffer) -> Self {
        let cols = fb.width / GLYPH_WIDTH;
        let rows = fb.height / GLYPH_HEIGHT;
        Self {
            fb,
            grid: CellGrid::new(cols, rows),
            cols,
            rows,
            cursor: Cursor::new(),
        }
    }

    /// Returns the screen size in character cells as (cols, rows).
    #[allow(dead_code)]
    pub fn size(&self) -> (usize, usize) {
        (self.cols, self.rows)
    }

    /// Draws `glyph` (`width` cells wide) with its left edge at (`row`, `col`), without moving the
    /// cursor. If it covers only half of a wide glyph already there, the other half is blanked.
    fn draw_glyph(
        &mut self,
        row: usize,
        col: usize,
        glyph: &Glyph,
        width: usize,
        fg: u32,
        bg: u32,
    ) {
        assert!(row < self.rows && col + width <= self.cols);
        for other_half in self.grid.place(row, col, width).into_iter().flatten() {
            self.fill_cell(row, other_half, bg);
        }
        let x0 = col * GLYPH_WIDTH;
        let y0 = row * GLYPH_HEIGHT;
        for dy in 0..GLYPH_HEIGHT {
            for dx in 0..width * GLYPH_WIDTH {
                let color = if glyph.get_pixel(dx, dy) { fg } else { bg };
                self.fb.put_pixel(x0 + dx, y0 + dy, color);
            }
        }
    }

    /// Fills one cell with `bg` (pixels only).
    fn fill_cell(&self, row: usize, col: usize, bg: u32) {
        for dy in 0..GLYPH_HEIGHT {
            for dx in 0..GLYPH_WIDTH {
                self.fb
                    .put_pixel(col * GLYPH_WIDTH + dx, row * GLYPH_HEIGHT + dy, bg);
            }
        }
    }

    /// Moves the cursor to the specified row and column.
    pub fn move_cursor(&mut self, row: usize, col: usize) {
        assert!(row < self.rows && col < self.cols);
        self.cursor.move_to(row, col);
    }

    /// The cursor's current (row, col) -- used by the line discipline's `begin` to find the row for
    /// a new line after a program's output (`fd::write`'s `Console` case) has moved the cursor
    /// independently of anything the line editing did.
    pub fn cursor(&self) -> (usize, usize) {
        (self.cursor.row, self.cursor.col)
    }

    /// Clears the whole screen to `bg`, an BGR color value.
    pub fn clear(&mut self, bg: u32) {
        for y in 0..self.fb.height {
            for x in 0..self.fb.width {
                self.fb.put_pixel(x, y, bg);
            }
        }
        self.grid.clear();
    }

    /// Clears one row of character cells to `bg`, an BGR color value.
    pub fn clear_row(&mut self, row: usize, bg: u32) {
        assert!(row < self.rows);
        for dy in 0..GLYPH_HEIGHT {
            for x in 0..self.fb.width {
                self.fb.put_pixel(x, row * GLYPH_HEIGHT + dy, bg);
            }
        }
        self.grid.clear_row(row);
    }

    /// Writes `c`'s glyph at the cursor and advances it by the glyph's width. Wrapping follows xterm:
    /// a glyph that ends in the last column leaves the cursor there, and the wrap -- to the start of the
    /// next row, scrolling first if that is past the last one -- happens only when the next glyph
    /// arrives, so a full row followed by `\n` doesn't leave a blank row. (A space is a glyph like
    /// any other: it lands in column 0 of the next row.) A wide glyph that would not fit in what is
    /// left of the row wraps first, whole, rather than being cut. A running EL0 program's output
    /// (via `fd::write`) and a typed line (`keyboard/line_discipline.rs`) can both span any number of
    /// rows, so nothing here assumes a single one.
    fn put_char_at_cursor(&mut self, c: char, fg: u32, bg: u32) {
        let glyph = glyph_for(c);
        let width = if glyph.is_fullwidth() { 2 } else { 1 };
        if self.cursor.start_glyph(width, self.cols, self.rows) {
            self.scroll_up(bg);
        }
        self.draw_glyph(self.cursor.row, self.cursor.col, glyph, width, fg, bg);
        self.cursor.end_glyph(width, self.cols);
    }

    /// Scrolls the console up by one row.
    ///
    /// The pixels are all the console keeps of what is on screen (plus which cells are half a wide
    /// glyph, in `grid`), so scrolling moves the pixel data up by one row's height and clears the
    /// last row.
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

        // Clear the last row (which also resets its cells in the grid), after moving the rest.
        self.grid.scroll_up();
        self.clear_row(self.rows - 1, bg);
    }

    /// Writes one character of text at the cursor, advancing it. `\r`/`\n`/`\t`/backspace move the
    /// cursor instead of drawing a glyph for them (backspace moves back one whole character -- two
    /// cells over a wide glyph -- and erases nothing); a zero-width code point (`font::is_zero_width`)
    /// does nothing; any other control character draws Unifont's U+FFFD, the same as a character the
    /// font lacks, so stray control bytes in a file show up instead of vanishing.
    ///
    /// User or system programs can use this function directly if treating the console as a dumb
    /// terminal, but for more advanced terminal handling (e.g. a TUI), they might want to manage
    /// cursor movement and screen updates themselves.
    pub fn write_char(&mut self, c: char, fg: u32, bg: u32) {
        match c {
            '\n' => {
                // Treat newlines as \r\n, moving the cursor to the start of the next line, and
                // scrolling if it was on the last one.
                if self.cursor.line_feed(self.rows) {
                    self.scroll_up(bg);
                }
            }
            '\r' => self.cursor.carriage_return(),
            '\t' => self.cursor.tab(TAB_WIDTH, self.cols),
            '\u{8}' => self.cursor.backspace(&self.grid),
            _ if is_zero_width(c) => {}
            _ => self.put_char_at_cursor(c, fg, bg),
        }
    }
}
