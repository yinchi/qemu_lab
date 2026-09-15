//! A software text console: cursor-addressable character cells rendered as glyphs into a raw
//! pixel framebuffer.
//!
//! The display's resolution is negotiated with virtio-gpu, not hardware-fixed the way a real CRT
//! would be -- but nothing in this project needs runtime resizing, so it's treated as fixed by
//! convention: chosen once at startup, with `cols`/`rows` plain values derived from it rather
//! than something callers re-query.

use crate::cp437::unicode_to_cp437;
use crate::font::{Font, GLYPH_HEIGHT, GLYPH_WIDTH};

/// Column spacing for `\t`, matching the traditional terminal default.
const TAB_WIDTH: usize = 8;

/// A raw BGRX8888 pixel surface. (8 bits per channel, blue-green-red-padding = 32 bits / pixel).
/// Since the machine is little-endian, a color value 0xAARRGGBB (e.g. as a parameter to
/// `put_pixel`) will be stored in memory as BB GG RR AA.
pub struct Framebuffer {
    /// Pointer to the start of the pixel data.
    pub ptr: *mut u8,
    /// Width of the framebuffer in pixels.
    pub width: usize,
    /// Height of the framebuffer in pixels.
    pub height: usize,
    /// Bytes per row; may exceed `width * 4` if the source pads rows.
    pub stride: usize,
}

impl Framebuffer {
    /// Returns the byte offset of the pixel at (`x`, `y`) within the framebuffer's memory.
    fn pixel_offset(&self, x: usize, y: usize) -> usize {
        y * self.stride + x * 4
    }

    /// Sets the pixel at (`x`, `y`) to the given BGRX8888 color.
    pub fn put_pixel(&self, x: usize, y: usize, bgrx: u32) {
        debug_assert!(x < self.width && y < self.height);
        unsafe {
            (self.ptr.add(self.pixel_offset(x, y)) as *mut u32).write_volatile(bgrx);
        }
    }
}

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
    #[allow(dead_code)]
    pub fn move_cursor(&mut self, row: usize, col: usize) {
        assert!(row < self.rows && col < self.cols);
        self.cursor_row = row;
        self.cursor_col = col;
    }

    /// Get the current cursor position as (row, col).
    #[allow(dead_code)]
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
    #[allow(dead_code)]
    pub fn clear_row(&self, row: usize, bg: u32) {
        assert!(row < self.rows);
        for dy in 0..GLYPH_HEIGHT {
            for x in 0..self.fb.width {
                self.fb.put_pixel(x, row * GLYPH_HEIGHT + dy, bg);
            }
        }
    }

    /// Writes `ch` at the cursor and advances it, wrapping to the start of the next row at the
    /// end of a line. Scrolling past the last row is deliberately unhandled -- out of scope
    /// until this stage's demo actually needs more than one screenful.
    pub fn putc(&mut self, ch: u8, fg: u32, bg: u32) {
        self.put_char(self.cursor_row, self.cursor_col, ch, fg, bg);
        self.cursor_col += 1;
        if self.cursor_col >= self.cols {
            self.cursor_col = 0;
            self.cursor_row = (self.cursor_row + 1).min(self.rows - 1);
        }
    }

    /// Writes one character of text at the cursor, advancing it. `\r`, `\n`, and `\t` move the
    /// cursor instead of drawing a glyph for them -- they're structural, not visible content, so
    /// intercepting them here (rather than leaving it to a caller-side pre-split on `\n`, or
    /// letting them fall through to `unicode_to_cp437` and draw whatever CP437 glyph happens to
    /// occupy that byte) is what makes arbitrary UTF-8 text safe to feed straight through.
    /// Anything else goes through `unicode_to_cp437` and `putc`, same as before.
    pub fn write_char(&mut self, c: char, fg: u32, bg: u32) {
        match c {
            '\n' => {
                self.cursor_col = 0;
                self.cursor_row = (self.cursor_row + 1).min(self.rows - 1);
            }
            '\r' => self.cursor_col = 0,
            '\t' => {
                let next_stop = (self.cursor_col / TAB_WIDTH + 1) * TAB_WIDTH;
                self.cursor_col = next_stop.min(self.cols - 1);
            }
            _ => self.putc(unicode_to_cp437(c), fg, bg),
        }
    }
}
