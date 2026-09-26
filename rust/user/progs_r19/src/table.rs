//! Left-aligned text columns, as `lsblk` prints its devices: every column as wide as its widest cell (the header
//! included), separated by two spaces, the last column not padded, so a row never ends in spaces. Also the device
//! names (`vda`, `vdb`, ...). Pure, so it is tested on the host (`hosttests/`).

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

/// The name Linux gives virtio disk `index`: `vda`, `vdb`, ... `vdz`; further devices continue `vdaa`, `vdab`, ...
pub fn device_name(index: usize) -> String {
    let mut letters = alloc::vec::Vec::new();
    let mut n = index;
    loop {
        letters.push(b'a' + (n % 26) as u8);
        if n < 26 {
            break;
        }
        n = n / 26 - 1;
    }
    letters.reverse();
    format!("vd{}", String::from_utf8(letters).unwrap_or_default())
}

/// The header and rows laid out, one `\n`-ended line each. Every row has as many cells as the header.
pub fn render(header: &[&str], rows: &[Vec<String>]) -> String {
    let mut widths: Vec<usize> = header.iter().map(|h| h.chars().count()).collect();
    for row in rows {
        for (width, cell) in widths.iter_mut().zip(row) {
            *width = (*width).max(cell.chars().count());
        }
    }
    let mut out = String::new();
    let mut line = |cells: &mut dyn Iterator<Item = &str>| {
        let mut text = String::new();
        for (i, cell) in cells.enumerate() {
            if i > 0 {
                text.push_str("  ");
            }
            text.push_str(cell);
            for _ in cell.chars().count()..widths[i] {
                text.push(' ');
            }
        }
        out.push_str(text.trim_end());
        out.push('\n');
    };
    line(&mut header.iter().copied());
    for row in rows {
        line(&mut row.iter().map(String::as_str));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_run_a_to_z_then_on() {
        assert_eq!(device_name(0), "vda");
        assert_eq!(device_name(1), "vdb");
        assert_eq!(device_name(25), "vdz");
        assert_eq!(device_name(26), "vdaa");
        assert_eq!(device_name(27), "vdab");
        assert_eq!(device_name(51), "vdaz");
        assert_eq!(device_name(52), "vdba");
    }

    fn row(cells: &[&str]) -> Vec<String> {
        cells.iter().map(|c| String::from(*c)).collect()
    }

    #[test]
    fn columns_are_as_wide_as_their_widest_cell() {
        let text = render(
            &["NAME", "SIZE", "LABEL"],
            &[row(&["vda", "64M", "SYSTEM"]), row(&["vdb", "1M", "HOME"])],
        );
        assert_eq!(text, "NAME  SIZE  LABEL\nvda   64M   SYSTEM\nvdb   1M    HOME\n");
    }

    #[test]
    fn a_wide_cell_widens_its_column() {
        let text = render(&["A", "B"], &[row(&["long cell", "x"])]);
        assert_eq!(text, "A          B\nlong cell  x\n");
    }

    #[test]
    fn empty_cells_and_the_last_column_leave_no_trailing_spaces() {
        let text = render(&["NAME", "LABEL", "MOUNT"], &[row(&["vdb", "", ""]), row(&["vda", "SYSTEM", "/"])]);
        assert_eq!(text, "NAME  LABEL   MOUNT\nvdb\nvda   SYSTEM  /\n");
    }

    #[test]
    fn no_rows_is_just_the_header() {
        assert_eq!(render(&["NAME"], &[]), "NAME\n");
    }
}
