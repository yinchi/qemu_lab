//! Adapts the block device's whole-sector I/O (`Blk::read_blocks_irq`/`write_blocks_irq`) to the
//! byte-addressable `Read`/`Write`/`Seek` trait bundle `hadris-fat` needs for its backing store.
//!
//! Reaches its block device through the shared `BLK` statics (see `platform/globals.rs`) rather than owning
//! one directly: `Blk`'s IRQ completion path (`ack_interrupt`, called from `irq_handler`) has to
//! stay paired with the same instance `read_blocks_irq`/`write_blocks_irq` block on, for this
//! program's entire remaining lifetime.

use core::fmt;

// `hadris_fat::sync::Result<T, E = ErrorKind>` wraps `E` in the crate's own `Error<E>` -- every
// trait method below has to return `FatResult<T, Self::Error>`, not the prelude's plain
// `core::result::Result<T, Self::Error>`, to satisfy the trait. Imported under an alias rather
// than as bare `Result`, so it can't silently shadow the prelude name for the rest of this file.
use hadris_fat::sync::Error as FatError;
use hadris_fat::sync::IoResult as FatResult;
use hadris_fat::sync::{Read, Seek, SeekFrom, Write};

use crate::drivers::virtio::blk;

/// The mounted FAT filesystem -- kept alive for the program's entire remaining life, not just at
/// boot, since a program name typed at the prompt (and every path a running program opens, see
/// `files.rs`) needs looking up fresh every time. Its
/// `root_dir()`/`open_entry()` results (`FatDir<'a, BlkIo>`) borrow from this, so unlike `LINE_DISCIPLINE`/
/// `KEY_STATE` those are never themselves stored in a static -- only ever re-derived, cheaply,
/// at each lookup (see `launch`).
pub static mut VOL: Option<hadris_fat::sync::FatVolume<BlkIo>> = None;

/// The size of a single sector, in bytes.
const SECTOR_SIZE: usize = 512;

/// Errors that can occur during block device I/O operations.
#[derive(Debug)]
pub enum BlkIoError {
    /// An error originating from the underlying block device.
    Blk(virtio_drivers::Error),
    /// Attempted to seek to a negative position in the block device.
    NegativeSeek,
}

impl fmt::Display for BlkIoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BlkIoError::Blk(e) => write!(f, "block device error: {e:?}"),
            BlkIoError::NegativeSeek => write!(f, "seek to a negative position"),
        }
    }
}

impl core::error::Error for BlkIoError {}

// Implements the `embedded_io::Error` trait for `BlkIoError`. This allows `BlkIoError` to be
// used with the `embedded-io` ecosystem.
//
// For simplicity, `kind` always returns `ErrorKind::Other`.
impl embedded_io::Error for BlkIoError {
    fn kind(&self) -> embedded_io::ErrorKind {
        embedded_io::ErrorKind::Other
    }
}

/// A byte-addressable view of the whole block device, from byte 0 to its full sector-granular
/// capacity -- everything `hadris-fat` sees is the raw disk image, boot sector through data
/// clusters; it decides for itself which byte ranges are FAT tables, directory entries, or file
/// data.
pub struct BlkIo {
    /// Which block device (an index into `BLK`).
    dev: usize,
    /// Current position within the block device, in bytes.
    pos: u64,
    /// Total capacity of the block device, in bytes.
    total_bytes: u64,
}

impl BlkIo {
    /// Creates a new `BlkIo` instance representing the whole of block device `dev`.
    ///
    /// SAFETY: `dev` must be an index `BLK` is populated at, with its SPI already enabled at the GIC -- see
    /// `kernel_main`, same precondition `read_blocks_irq`/`write_blocks_irq` themselves rely on.
    pub unsafe fn new(dev: usize) -> Self {
        let total_bytes = unsafe { blk::get(dev) }.capacity_bytes();
        Self {
            dev,
            pos: 0,
            total_bytes,
        }
    }
}

impl Read for BlkIo {
    type Error = BlkIoError;

    fn read(&mut self, buf: &mut [u8]) -> FatResult<usize, Self::Error> {
        if self.pos >= self.total_bytes || buf.is_empty() {
            return Ok(0);
        }

        let sector = (self.pos / SECTOR_SIZE as u64) as usize;
        let offset = (self.pos % SECTOR_SIZE as u64) as usize;
        let mut sector_buf = [0u8; SECTOR_SIZE];

        // Copy the current sector from the block device into the sector buffer.
        // The current read() operation is guaranteed not to cross a sector boundary.
        //
        // SAFETY: see `new`'s doc comment -- holds for this whole program, not just at
        // construction time.
        unsafe { blk::get(self.dev) }
            .read_blocks_irq(sector, &mut sector_buf)
            .map_err(|e| FatError::from_source(BlkIoError::Blk(e)))?;

        // Read from the sector buffer until the end of the current sector/device or the
        // end of the buffer, whichever comes first.
        let n = buf
            .len()
            .min(SECTOR_SIZE - offset)
            .min((self.total_bytes - self.pos) as usize);
        buf[..n].copy_from_slice(&sector_buf[offset..offset + n]);

        // Update the current position within the block device and return the number of bytes read.
        self.pos += n as u64;
        Ok(n)
    }
}

impl Write for BlkIo {
    type Error = BlkIoError;

    fn write(&mut self, buf: &[u8]) -> FatResult<usize, Self::Error> {
        if buf.is_empty() {
            return Ok(0);
        }

        let sector = (self.pos / SECTOR_SIZE as u64) as usize;
        let offset = (self.pos % SECTOR_SIZE as u64) as usize;

        let mut sector_buf = [0u8; SECTOR_SIZE];

        // Read the current sector
        // SAFETY: see `Read::read`'s SAFETY comment -- identical reasoning.
        unsafe { blk::get(self.dev) }
            .read_blocks_irq(sector, &mut sector_buf)
            .map_err(|e| FatError::from_source(BlkIoError::Blk(e)))?;

        // Modify the sector buffer with the new data from the write() call.
        // The write() call is guaranteed to fit within the current sector.
        let n = buf.len().min(SECTOR_SIZE - offset);
        sector_buf[offset..offset + n].copy_from_slice(&buf[..n]);

        // Write the modified sector buffer back to the block device.
        // SAFETY: see `Read::read`'s SAFETY comment -- identical reasoning.
        unsafe { blk::get(self.dev) }
            .write_blocks_irq(sector, &sector_buf)
            .map_err(|e| FatError::from_source(BlkIoError::Blk(e)))?;

        // Update the current position within the block device and return the total bytes written.
        self.pos += n as u64;
        Ok(n)
    }

    fn flush(&mut self) -> FatResult<(), Self::Error> {
        // Every write above already lands on the device before returning -- nothing buffered to
        // push out here.
        Ok(())
    }
}

impl Seek for BlkIo {
    type Error = BlkIoError;

    fn seek(&mut self, pos: SeekFrom) -> FatResult<u64, Self::Error> {
        let new_pos = match pos {
            SeekFrom::Start(n) => n as i64,
            SeekFrom::Current(n) => self.pos as i64 + n,
            SeekFrom::End(n) => self.total_bytes as i64 + n,
        };
        if new_pos < 0 {
            return Err(FatError::from_source(BlkIoError::NegativeSeek));
        }
        self.pos = new_pos as u64;
        Ok(self.pos)
    }
}
