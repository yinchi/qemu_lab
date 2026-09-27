//! `ioctl` request codes: what a program can ask of the thing an fd is open on, out of band -- separate
//! from the bytes it reads and writes, so file contents can never control the display. The codes are
//! this project's own (Linux's are terminal-specific and would only mislead), with one deliberate
//! exception: `TIOCGWINSZ` keeps Linux's number and `struct winsize`, so a program learns "am I on a
//! terminal, and how big" the way it would there. The error for an fd that does not understand a
//! request is `ENOTTY`, as on Linux.

/// Clears the console and puts its cursor at the top left. Only the console (stdout/stderr, when they
/// are the console) understands it; `arg` is unused.
pub const CONSOLE_CLEAR: usize = 1;

/// Fills the [`WinSize`] that `arg` points to with the console's size in character cells (Linux's
/// `TIOCGWINSZ`, request number and struct). Answered for stdin (the keyboard) and stdout/stderr (the
/// display) while they are the console; `ENOTTY` when redirected, `EFAULT` for a bad `arg`.
pub const TIOCGWINSZ: usize = 0x5413;

/// Bytes in a [`WinSize`] as the kernel writes it.
pub const WINSIZE_SIZE: usize = 8;

/// Linux's `struct winsize`: four native-endian (little-endian here) `u16`s. The pixel fields are left 0:
/// a program needs the size in character cells, and the cell size is the console's own business.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WinSize {
    pub rows: u16,
    pub cols: u16,
    pub xpixel: u16,
    pub ypixel: u16,
}

impl WinSize {
    /// A size in character cells, pixel fields 0.
    pub const fn cells(rows: u16, cols: u16) -> Self {
        Self {
            rows,
            cols,
            xpixel: 0,
            ypixel: 0,
        }
    }

    /// The bytes the kernel writes to the user's buffer: `ws_row`, `ws_col`, `ws_xpixel`, `ws_ypixel`.
    pub fn encode(&self) -> [u8; WINSIZE_SIZE] {
        let mut out = [0u8; WINSIZE_SIZE];
        out[0..2].copy_from_slice(&self.rows.to_le_bytes());
        out[2..4].copy_from_slice(&self.cols.to_le_bytes());
        out[4..6].copy_from_slice(&self.xpixel.to_le_bytes());
        out[6..8].copy_from_slice(&self.ypixel.to_le_bytes());
        out
    }

    /// The inverse of [`WinSize::encode`].
    pub fn decode(bytes: [u8; WINSIZE_SIZE]) -> Self {
        Self {
            rows: u16::from_le_bytes([bytes[0], bytes[1]]),
            cols: u16::from_le_bytes([bytes[2], bytes[3]]),
            xpixel: u16::from_le_bytes([bytes[4], bytes[5]]),
            ypixel: u16::from_le_bytes([bytes[6], bytes[7]]),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_codes_are_pinned() {
        assert_eq!(CONSOLE_CLEAR, 1);
        assert_eq!(TIOCGWINSZ, 0x5413); // Linux's, on every architecture
    }

    #[test]
    fn winsize_is_linuxs_layout() {
        // rows, cols, xpixel, ypixel: four little-endian u16s, in that order.
        let bytes = WinSize {
            rows: 30,
            cols: 80,
            xpixel: 0x0102,
            ypixel: 0x0304,
        }
        .encode();
        assert_eq!(bytes, [30, 0, 80, 0, 0x02, 0x01, 0x04, 0x03]);
        assert_eq!(bytes.len(), WINSIZE_SIZE);
    }

    #[test]
    fn winsize_round_trips() {
        let size = WinSize::cells(30, 80);
        assert_eq!(WinSize::decode(size.encode()), size);
        assert_eq!((size.xpixel, size.ypixel), (0, 0));
    }
}
