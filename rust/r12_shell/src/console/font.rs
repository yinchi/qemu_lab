//! Which glyph draws a character, and how wide it is. The glyphs are GNU Unifont's, from the `unifont`
//! crate (compiled into the kernel): 8x16 for most, 16x16 for the wide ones (CJK and friends). The
//! crate covers the Basic Multilingual Plane only; anything else draws U+FFFD.
//!
//! Pure apart from that crate, so it is tested on the host (`hosttests/`).

use unifont::Glyph;

/// The width of a one-cell glyph in pixels; a wide glyph is two cells, `2 * GLYPH_WIDTH`.
pub const GLYPH_WIDTH: usize = 8;
/// The height of every glyph in pixels.
pub const GLYPH_HEIGHT: usize = 16;

const REPLACEMENT: char = '\u{FFFD}';

/// Code points that take no cell and draw nothing: zero-width space, joiners and directional marks,
/// the word joiner, variation selectors, and the byte-order mark.
pub fn is_zero_width(c: char) -> bool {
    matches!(c, '\u{200B}'..='\u{200F}' | '\u{2060}' | '\u{FE00}'..='\u{FE0F}' | '\u{FEFF}')
}

/// The glyph that draws `c`: Unifont's, or U+FFFD's if the font has none (everything above U+FFFF, or
/// unassigned) or `c` is a control character (the console interprets the few it uses before
/// asking for a glyph).
pub fn glyph_for(c: char) -> &'static Glyph {
    let found = if c.is_control() { None } else { unifont::get_glyph(c) };
    found.unwrap_or_else(|| {
        unifont::get_glyph(REPLACEMENT).expect("Unifont has a glyph for U+FFFD")
    })
}

/// How many cells `c` takes when written: 0 (`is_zero_width`), 2 (a wide glyph), or 1. The four
/// characters the console interprets rather than draws (newline, carriage return, tab, backspace)
/// count as 1, like everything else `show_row` may be asked to lay out.
pub fn cell_width(c: char) -> usize {
    if is_zero_width(c) {
        0
    } else if glyph_for(c).is_fullwidth() && !matches!(c, '\n' | '\r' | '\t' | '\u{8}') {
        2
    } else {
        1
    }
}

/// The longest tail of `text` that is at most `budget` cells wide -- whole characters only, so the
/// window never starts in the middle of a wide glyph.
pub fn tail_window(text: &str, budget: usize) -> &str {
    let mut width = 0;
    let mut start = text.len();
    for (index, c) in text.char_indices().rev() {
        width += cell_width(c);
        if width > budget {
            break;
        }
        start = index;
    }
    &text[start..]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_is_one_cell_and_cjk_is_two() {
        assert_eq!(cell_width('a'), 1);
        assert_eq!(cell_width(' '), 1);
        assert_eq!(cell_width('~'), 1);
        assert_eq!(cell_width('é'), 1);
        assert_eq!(cell_width('日'), 2);
        assert_eq!(cell_width('한'), 2);
        assert_eq!(cell_width('Ａ'), 2); // fullwidth Latin
    }

    #[test]
    fn the_zero_width_set_takes_no_cell() {
        for c in ['\u{200B}', '\u{200D}', '\u{200F}', '\u{2060}', '\u{FE0F}', '\u{FEFF}'] {
            assert!(is_zero_width(c), "{c:?}");
            assert_eq!(cell_width(c), 0);
        }
        assert!(!is_zero_width('a'));
        assert!(!is_zero_width('\u{FFFD}'));
    }

    #[test]
    fn characters_the_font_lacks_draw_the_replacement_glyph() {
        let replacement = glyph_for('\u{FFFD}');
        for c in ['😀', '\u{10000}', '\u{10FFFF}', '\u{0}', '\u{1B}', '\u{7F}', '\u{85}'] {
            assert_eq!(glyph_for(c), replacement, "{c:?}");
        }
    }

    #[test]
    fn ordinary_characters_have_their_own_glyphs() {
        let replacement = glyph_for('\u{FFFD}');
        for c in ['a', 'Z', '0', ' ', 'é', 'Ω', 'Ж', '日', '☺', '€'] {
            assert_ne!(glyph_for(c), replacement, "{c:?}");
        }
        assert_ne!(glyph_for('a'), glyph_for('b'));
    }

    #[test]
    fn the_interpreted_controls_count_as_one_cell() {
        for c in ['\n', '\r', '\t', '\u{8}'] {
            assert_eq!(cell_width(c), 1);
        }
    }

    #[test]
    fn the_window_is_measured_in_cells_and_never_splits_a_wide_glyph() {
        assert_eq!(tail_window("hello", 3), "llo");
        assert_eq!(tail_window("hello", 10), "hello");
        assert_eq!(tail_window("hello", 0), "");
        // 日本語 is 6 cells: a budget of 5 fits two glyphs (4 cells), not two and a half.
        assert_eq!(tail_window("日本語", 5), "本語");
        assert_eq!(tail_window("日本語", 6), "日本語");
        assert_eq!(tail_window("日本語", 1), "");
        // Mixed, with a zero-width character (free).
        assert_eq!(tail_window("a日\u{200D}b", 3), "日\u{200D}b");
        // A zero-width character costs nothing, so the window may start on it.
        assert_eq!(tail_window("a日\u{200D}b", 2), "\u{200D}b");
    }
}
