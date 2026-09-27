//! Turning one screen row's text (a [`Row`] from `layout::wrap_line`) into the actual
//! `abi::ioctl::Cell`s a `CONSOLE_DRAW` frame needs: a tab becomes that many blank cells up to its
//! stop, a wide glyph's second column is filled with a blank cell (never read by the kernel, but
//! still needs an entry there -- see `abi::ioctl::Cell`'s own doc comment), and a zero-width mark
//! draws nothing. The row is always padded out to exactly `cols` cells, whatever it actually drew,
//! so a screen built row by row this way is always exactly `rows * cols` cells long, matching
//! `CONSOLE_DRAW`'s own requirement.
//!
//! Uses [`crate::layout::width_at`], the very function `wrap_line` measured the row with, so a row
//! it produced can never overflow here: this is deliberately not a second, independent width
//! computation that could drift from it.
//!
//! Pure `no_std` + `alloc`, so it is tested on the host (`hosttests/`).

use alloc::vec::Vec;

use abi::ioctl::Cell;

use crate::layout::{Row, width_at};

/// Renders one screen row -- `text[row.start..row.end]`, already known to fit within `cols` columns
/// -- as exactly `cols` cells, appended to `out`.
pub fn render_row(out: &mut Vec<Cell>, text: &str, row: Row, cols: usize, tab_size: usize) {
    let mut col = 0;
    for c in text[row.start..row.end].chars() {
        let width = width_at(c, col, tab_size).min(cols.saturating_sub(col));
        if width == 0 {
            continue; // a zero-width mark: nothing to draw for it
        }
        out.push(if c == '\t' {
            Cell::plain(' ')
        } else {
            Cell::plain(c)
        });
        for _ in 1..width {
            out.push(Cell::plain(' ')); // the rest of a tab's stop, or a wide glyph's second column
        }
        col += width;
    }
    for _ in col..cols {
        out.push(Cell::plain(' '));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::wrap_line;

    fn row_cells(text: &str, cols: usize, tab_size: usize) -> Vec<Vec<char>> {
        wrap_line(text, cols, tab_size)
            .into_iter()
            .map(|row| {
                let mut out = Vec::new();
                render_row(&mut out, text, row, cols, tab_size);
                assert_eq!(
                    out.len(),
                    cols,
                    "row for {text:?} must be exactly {cols} cells"
                );
                out.into_iter()
                    .map(|c| char::from_u32(c.ch).unwrap())
                    .collect()
            })
            .collect()
    }

    #[test]
    fn a_short_line_is_padded_with_blanks() {
        assert_eq!(row_cells("hi", 5, 8), [['h', 'i', ' ', ' ', ' ']]);
    }

    #[test]
    fn every_row_of_a_wrapped_line_is_exactly_cols_cells() {
        for (text, cols, tab) in [
            ("", 5, 8),
            ("hello world", 4, 8),
            ("12345", 5, 8), // exactly full, plus the trailing empty row
            ("a\tb\tc", 5, 4),
            ("ab日", 3, 8),
        ] {
            row_cells(text, cols, tab); // the assertion inside row_cells does the real checking
        }
    }

    #[test]
    fn a_tab_fills_blanks_up_to_its_stop() {
        // tab_size 4: 'a' then a tab reaching column 4, i.e. 3 blank cells.
        assert_eq!(row_cells("a\tb", 6, 4), [['a', ' ', ' ', ' ', 'b', ' ']]);
    }

    #[test]
    fn a_wide_glyph_is_followed_by_one_blank_filler_cell() {
        // "a日b" exactly fills 4 columns, so (as usual) there is a trailing empty row too.
        assert_eq!(
            row_cells("a日b", 4, 8),
            [['a', '日', ' ', 'b'], [' ', ' ', ' ', ' ']]
        );
    }

    #[test]
    fn a_zero_width_mark_draws_nothing() {
        assert_eq!(row_cells("a\u{200B}b", 4, 8), [['a', 'b', ' ', ' ']]);
    }

    #[test]
    fn an_empty_row_is_all_blanks() {
        assert_eq!(row_cells("", 3, 8), [[' ', ' ', ' ']]);
    }
}
