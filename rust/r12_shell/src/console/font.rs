//! The raw font (`disk/fonts/spleen.raw`, see `disk/fonts/NOTICE` -- read through the FAT
//! filesystem now, not a raw sector read; see `main.rs`'s `read_font`) as a directly-indexed
//! glyph table: 256 glyphs, each 8 pixels wide and 16 pixels tall.
//!
//! 16 rows of 8 binary pixels = each glyph is exactly 16 bytes, each representing one row of 8
//! pixels.

/// The width of each glyph in pixels.
pub const GLYPH_WIDTH: usize = 8;
/// The height of each glyph in pixels.
pub const GLYPH_HEIGHT: usize = 16;
/// The number of bytes used to represent each glyph.
/// Each pixel is a single bit, and each row of 8 pixels is packed into one byte.
const BYTES_PER_GLYPH: usize = GLYPH_HEIGHT;
/// The total number of glyphs in the font.
const GLYPH_COUNT: usize = 256;

/// The font, read off the block device once at boot. `Console`'s `Font` borrows this, so it
/// needs to be `'static` rather than a `kernel_main`-local array -- see Stage 6's copy of this
/// file, which could get away with a local since nothing there needed to outlive `kernel_main`.
pub static mut FONT_DATA: [u8; GLYPH_COUNT * BYTES_PER_GLYPH] = [0; GLYPH_COUNT * BYTES_PER_GLYPH];

pub struct Font<'a> {
    data: &'a [u8],
}

impl<'a> Font<'a> {
    /// `data` must be exactly 256 glyphs' worth of bytes -- the whole raw font file, unsliced,
    /// so that indexing by character code needs no offset math.
    pub fn new(data: &'a [u8]) -> Self {
        assert_eq!(
            data.len(),
            GLYPH_COUNT * BYTES_PER_GLYPH,
            "font data is the wrong size"
        );
        Self { data }
    }

    /// Returns the 16 row-bytes for `ch`, MSB = leftmost pixel.
    pub fn glyph(&self, ch: u8) -> &[u8; GLYPH_HEIGHT] {
        let start = ch as usize * BYTES_PER_GLYPH;
        self.data[start..start + BYTES_PER_GLYPH]
            .try_into()
            .unwrap()
    }
}
