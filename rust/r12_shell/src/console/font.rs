//! The raw font (`disk/fonts/spleen.raw`, see `disk/fonts/NOTICE` -- read through the FAT
//! filesystem now, not a raw sector read; see `main.rs`'s `read_font`) as a directly-indexed
//! glyph table: 256 glyphs, each 8 pixels wide and 16 pixels tall.
//!
//! 16 rows of 8 binary pixels = each glyph is exactly 16 bytes, each representing one row of 8
//! pixels.

pub const GLYPH_WIDTH: usize = 8;
pub const GLYPH_HEIGHT: usize = 16;
const BYTES_PER_GLYPH: usize = GLYPH_HEIGHT; // 1 byte/row at 8px wide, no padding

/// The font, read off the block device once at boot. `Console`'s `Font` borrows this, so it
/// needs to be `'static` rather than a `kernel_main`-local array -- see Stage 6's copy of this
/// file, which could get away with a local since nothing there needed to outlive `kernel_main`.
pub static mut FONT_DATA: [u8; 4096] = [0; 4096];

pub struct Font<'a> {
    data: &'a [u8],
}

impl<'a> Font<'a> {
    /// `data` must be exactly 256 glyphs' worth of bytes -- the whole raw font file, unsliced,
    /// so that indexing by character code needs no offset math.
    pub fn new(data: &'a [u8]) -> Self {
        assert_eq!(
            data.len(),
            256 * BYTES_PER_GLYPH,
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
