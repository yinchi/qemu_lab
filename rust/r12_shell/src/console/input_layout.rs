//! Where a typed line lands on the screen: how many rows a prefix plus text needs, and where the next
//! glyph would go. The line discipline (`keyboard/line_discipline.rs`) uses it to size and place the input
//! area; Step 12's cursor-aware editor will map a cell offset to a screen position with the same code
//! (`end_cursor`, exposed then as an insertion point).
//!
//! It replays exactly what `Console::write_char` does, by driving the same `Cursor` (`cells.rs`) over
//! an endless screen -- a glyph that ends in the last column leaves the wrap pending, a wide glyph that
//! does not fit moves to the next row whole -- so the layout and the drawing cannot disagree. Text is
//! taken to be printable: the line buffer never holds control characters (`line.rs`), and the
//! prompt is plain text.
//!
//! Pure `no_std`, with no dependency on the rest of the kernel, so it is tested on the host
//! (`hosttests/`).

use super::cells::Cursor;
use super::font::cell_width;

/// Where the cursor ends up after drawing `prefix` and then `text` from the top-left cell of a screen
/// `cols` cells wide and endlessly tall.
fn end_cursor(prefix: &str, text: &str, cols: usize) -> Cursor {
    let mut cursor = Cursor::new();
    for c in prefix.chars().chain(text.chars()) {
        let width = cell_width(c);
        if width == 0 {
            continue; // draws nothing and takes no cell
        }
        cursor.start_glyph(width, cols, usize::MAX);
        cursor.end_glyph(width, cols);
    }
    cursor
}

/// How many rows `prefix` and `text` occupy: at least one (the prefix's row, even when both are empty).
/// A row filled exactly does not start the next -- that happens only when another glyph arrives.
pub fn rows_needed(prefix: &str, text: &str, cols: usize) -> usize {
    end_cursor(prefix, text, cols).row + 1
}

/// Whether the line may be as long as it is on a screen of `rows` rows: it may use at most `rows - 1`
/// of them, so it can never scroll off the top or push its own first row out of view.
pub fn fits_on_screen(prefix: &str, text: &str, cols: usize, rows: usize) -> bool {
    rows_needed(prefix, text, cols) < rows
}

#[cfg(test)]
mod tests {
    use super::*;

    const COLS: usize = 10;

    /// Where the next glyph typed after `text` goes, as (row, column) relative to the line's first row:
    /// the cell after the last glyph, resolving a pending wrap to the start of the next row. Step 12's
    /// cursor drawing needs this outside the tests; it moves out of here then.
    fn insertion_point(prefix: &str, text: &str, cols: usize) -> (usize, usize) {
        let cursor = end_cursor(prefix, text, cols);
        if cursor.wrap_pending {
            (cursor.row + 1, 0)
        } else {
            (cursor.row, cursor.col)
        }
    }

    #[test]
    fn an_empty_line_is_one_row_with_the_cursor_after_the_prefix() {
        assert_eq!(rows_needed("", "", COLS), 1);
        assert_eq!(rows_needed("> ", "", COLS), 1);
        assert_eq!(insertion_point("> ", "", COLS), (0, 2));
        assert_eq!(insertion_point("", "", COLS), (0, 0));
    }

    #[test]
    fn a_line_grows_a_row_only_when_a_glyph_no_longer_fits() {
        // "> " is two cells, so eight characters fill the row exactly.
        assert_eq!(rows_needed("> ", "1234567", COLS), 1);
        assert_eq!(rows_needed("> ", "12345678", COLS), 1);
        assert_eq!(rows_needed("> ", "123456789", COLS), 2);
        assert_eq!(rows_needed("> ", "12345678901234567890", COLS), 3);
    }

    #[test]
    fn the_insertion_point_after_a_full_row_is_the_start_of_the_next() {
        assert_eq!(insertion_point("> ", "1234567", COLS), (0, 9));
        assert_eq!(insertion_point("> ", "12345678", COLS), (1, 0));
        assert_eq!(insertion_point("> ", "123456789", COLS), (1, 1));
        assert_eq!(insertion_point("> ", "1234567890123456", COLS), (1, 8));
        assert_eq!(insertion_point("> ", "123456789012345678", COLS), (2, 0)); // 20 cells: two full rows
    }

    #[test]
    fn a_wide_glyph_that_does_not_fit_moves_to_the_next_row_whole() {
        // Two prefix cells plus seven letters leave one cell: the wide glyph starts the next row.
        assert_eq!(rows_needed("> ", "1234567日", COLS), 2);
        assert_eq!(insertion_point("> ", "1234567日", COLS), (1, 2));
        // With room for it, it stays.
        assert_eq!(rows_needed("> ", "123456日", COLS), 1);
        assert_eq!(insertion_point("> ", "123456日", COLS), (1, 0)); // filled the row exactly
        assert_eq!(insertion_point("> ", "12345日", COLS), (0, 9));
    }

    #[test]
    fn a_line_of_wide_glyphs_takes_two_cells_each() {
        // Five per row on ten columns.
        assert_eq!(rows_needed("", "日日日日日", COLS), 1);
        assert_eq!(rows_needed("", "日日日日日日", COLS), 2);
        assert_eq!(insertion_point("", "日日日日日日", COLS), (1, 2));
        // An odd number of columns leaves one cell blank at the end of each row.
        assert_eq!(rows_needed("", "日日日", 5), 2);
    }

    #[test]
    fn zero_width_characters_take_no_cell() {
        assert_eq!(insertion_point("> ", "a\u{200D}b", COLS), (0, 4));
        assert_eq!(rows_needed("", "\u{200D}\u{FE0F}", COLS), 1);
    }

    #[test]
    fn the_line_may_use_all_but_one_row_of_the_screen() {
        let rows = 4; // three rows of ten cells = 30 cells, prefix included
        let ok = "x".repeat(28);
        let full = "x".repeat(30);
        assert!(fits_on_screen("> ", &ok, COLS, rows)); // 30 cells: three rows, the last one exactly full
        assert!(!fits_on_screen("> ", &full, COLS, rows)); // 32 cells: a fourth row
        assert!(fits_on_screen("", "", COLS, 2));
        assert!(!fits_on_screen("", "", COLS, 1));
    }
}
