# VirtIO drivers and supporting modules

This file describes the VirtIO drivers and supporting modules implemented in Rust, as of `r14_file_times/` (`src/drivers/virtio/`); where an earlier stage differed, the text says so. The addresses these devices live at are in [`memory_regions.md`](memory_regions.md), and the page table the HAL relies on is in [`mmu.md`](mmu.md).

## Overview

The VirtIO devices live in the `virtio_drivers::device` module. The three devices we use are:

- `VirtIOInput` (for a keyboard)
- `VirtIOBlk` (for block storage)
- `VirtIOGpu` (for graphics)

### Device discovery

In our QEMU `virt` device, the machine's `x0` register is initialized to the address of a [device tree blob (DTB)](https://devicetree-specification.readthedocs.io/en/stable/flattened-format.html), which contains information about the hardware devices available to the system. Our `boot.s` assembly code (`arch/boot.s`) passes this address to the Rust kernel, which then parses the DTB to discover the available VirtIO devices.

The first eight bytes of the DTB contain:

```
0x0 D0 0D FE ED
0x4 <blob size in bytes, big-endian>
```

`base_addresses::init_base_addresses` copies the blob into RAM and parses it using the `fdt` (flattened device tree) crate.  In particular, it looks for nodes corresponding to the VirtIO devices, which contain a `compatible` field containing the string `"virtio,mmio"` (memory-mapped I/O).  For each such node, it extracts a `reg` field, which specifies the base address and size of the device's memory-mapped I/O region (though in practice this size is always `0x200` on our `virt` machine, so our code hardcodes it rather than reading it from the DTB), and an `interrupts` field, which specifies the interrupt line used by the device.  For our `virt` machine, these interrupts are on SPI (Shared Peripheral Interrupt) lines, e.g. SPI 1 &rarr; Interrupt ID 33 (offset=32).

Finally, each device driver in our kernel detects the presence of its corresponding VirtIO device by checking each base address extracted from the DTB (the shared `find_mmio_transport` in `drivers/virtio/mod.rs`, which each driver's `find` calls) and attempting to parse a `VirtIOHeader`. If a valid header is found and corresponds to a valid `MmioTransport` memory region, the driver then checks the device's `.device_type()` to ensure it matches the expected type for that driver. This repeats until the correct device is found or all base addresses have been checked.

See (for example) `Keyboard::find` in `drivers/virtio/input.rs` for an example of VirtIO device discovery and initialization.

### HAL (Hardware Abstraction Layer)

Each of the three VirtIO drivers is generic over `virtio_drivers::Hal`, the trait the crate uses to abstract over DMA (Direct Memory Access) allocation and physical/virtual address translation. We provide a single implementation, `VirtioHalImpl` (`drivers/virtio/hal.rs`), shared by all three drivers.

Because the kernel's page table is one flat identity mapping (physical == virtual; see [`mmu.md`](mmu.md)) rather than anything that would let code's virtual addresses map to different physical addresses, and every `virtio,mmio` DTB node is confirmed `dma-coherent`, the implementation is deliberately trivial. (Stages before Stage 12 never actually turned the MMU on, which had the same effect.)

- `dma_alloc()`: hands out pages from a single static, page-aligned `DMA_POOL` (512 pages / 2 MiB) by bumping a monotonically increasing offset (`DMA_POOL_NEXT`); since the pool starts zeroed, nothing further needs initializing.
- `dma_dealloc()`: unimplemented. Every device we use reports MMIO version Legacy, so each sets up its DMA region with a single `Dma::new()` held for the program's entire lifetime and never dropped early -- so it's never called.
- `mmio_phys_to_virt()`: normally translates a device's physical MMIO base address into a virtual address the driver can dereference; here it's an identity mapping (physical == virtual), since the page table maps every address to itself.
- `share()`: normally hands a buffer to the device by returning the physical address its DMA should target, doing whatever cache-flushing or bounce-buffering is needed so the CPU and device agree on the buffer's contents. Here it's just that address translation, since there's no IOMMU (Input-Output MMU -- hardware that would otherwise remap or restrict which physical addresses a device's DMA can target, the same role the MMU plays for the CPU) to program, and DMA is already coherent.
- `unshare()`: the inverse of `share()` -- normally reclaims a shared buffer for CPU use, undoing any bounce-buffering or invalidating caches as needed. Here a no-op, for the same reasons as `share()`.

Because `dma_alloc()` never frees memory, exhausting `DMA_POOL` returns a dangling pointer rather than panicking -- allocation failure is left for the `virtio_drivers` crate itself to handle.

## Keyboard

The VirtIOInput driver is generic, supporting various input devices such as keyboards and mice, but in our QEMU setup, is tied to a keyboard.  It interacts with the keyboard hardware through two main functions: `pop_pending_event()`, called to retrieve the next pending input event after a keyboard interrupt is received, and `ack_interrupt()`, called to acknowledge the device's interrupt (paired with `arm_gic::gicv2::GicV2::end_interrupt()` to signal that interrupt handling is complete).

`Keyboard::poll()` (`drivers/virtio/input.rs`), a thin wrapper around `pop_pending_event()`, retrieves a `InputEvent` representing the next pending input event from the keyboard.  `InputEvent.event_type` is then checked: for keyboard events, it should be 1.  Other important fields are:

- `InputEvent.code`: the key code of the key that generated the event.
- `InputEvent.value`: the value of the event, (0 for release, 1 for press, 2 for autorepeat).

Autorepeat is not acted on: `keyboard/events.rs` treats any non-release event as "down", and `KeyState::set` reports whether the set of held keys actually changed, so a repeat (or a plain press resent with no repeat tag, which is what this QEMU setup actually does) is a no-op. What happens to the resulting tokens (the queue, line editing, the two reading modes) is described in [`console.md`](console.md).

### Keymap

`keyboard/keymap.rs` defines human-readable strings for a large number of key codes, (`tokens.rs` shows a code with no name as `<K123>`).  It also provides structs for storing held key state and lock key state (Caps Lock, Num Lock, Scroll Lock).

```rust
pub struct LockState {
    pub caps: bool,
    pub num: bool,
    pub scroll: bool,
}

pub struct KeyState {
    held: [bool; MAX_CODE],
}
```

For example, if the user presses and releases Caps Lock, then holds down 'A' while pressing and releasing 'B', the `LockState` would reflect that Caps Lock is active, and the `KeyState` would have only 'A' marked in its `held` array. Neither type tracks which key was pressed last, since auto-repeat is not supported.

## GPU

The GPU driver (`drivers/virtio/gpu.rs`) negotiates a fixed 640x480 resolution once, in `Gpu::framebuffer()` (which calls the `virtio_drivers` crate's `change_resolution`), and returns a `FramebufferInfo` &mdash; pointer, width, height and stride &mdash; describing the DMA-backed pixel buffer in RAM. `flush()` dumps the framebuffer contents to the display. The resolution is fixed thereafter, and the driver knows nothing about text.

We also provide a `Console` abstraction that interacts with the GPU driver to render text output to the display, using functions such as `put_char` and `move_cursor` (the list below is the `r06`-`r11` API; `r14_file_times` changed it -- see "Stage 12 onward" under Font handling).  A call chain may include (all within `console.rs`, which became `console/mod.rs` in r12):

- `write_char()`: handles `\n`, `\r`, and `\t`, which move the cursor accordingly (a `\n` on the last row also calls `scroll_up()` to make room for the new line), and otherwise writes a character to the current cursor position using `putc`.
- `putc()`: writes a character to the current cursor position on the framebuffer, using `put_char`, and then advances the cursor position accordingly — note that in r06-r09 this does not itself scroll if writing runs past the last row (r10 and r11's `putc` scrolls itself).
- `put_char()`: writes a character directly to the framebuffer at the specified position.
- `scroll_up()`: shifts the framebuffer's pixel contents up by one character row and clears the newly-exposed last row.
- `size()`: returns the current size of the console as `(cols, rows)` rather than pixels.
- `move_cursor()`: moves the cursor to a specified position on the display.

Currently, although the GPU driver is pixel-based, we only interact with it through the character-based `Console` interface, except that `Console::clear()` operates at the pixel level to clear the entire screen.

### Font handling

The font scheme changes at Stage 12, so this section has two parts.

#### Stages r06-r11: a CP437 bitmap font

Font handling in the console is managed through a bitmap font, where each character is represented as a grid of pixels. Extracting the bitmap for a specific glyph is handled by the `font::Font::glyph()` function. Since each character is 8x16, the returned bitmap is an array of 16 bytes, with each byte representing a row of 8 pixels.

The glyph bitmaps are packed into a single contiguous array in CP437 order; however, the rest of the system typically interacts with characters using Unicode code points. Thus, `cp437::unicode_to_cp437()` provides the necessary mappings between Unicode code points and CP437 indices, allowing the console to correctly retrieve the corresponding glyph bitmaps for display.

#### Stage 12 onward: Unicode with GNU Unifont

`r14_file_times` draws Unicode directly. There is no codepage and no font file on the disk: the glyphs are GNU Unifont's, from the `unifont` crate (`no_std`, MIT; the font data itself is dual-licensed GPLv2+ with the font-embedding exception, or SIL OFL 1.1), compiled into the kernel image's read-only data (about 1.9 MB). `Font`, `cp437.rs`, `spleen.raw` and the boot-time font read are gone. Three small modules under `console/` divide the work, and all three are pure (apart from the `unifont` crate), so they are tested on the host (`hosttests/`):

- **Bytes to characters (`utf8.rs`, pure).** A program's console output is UTF-8, but a `write` can end in the middle of a character (`cat` sends 4096-byte chunks). `Utf8Decoder` decodes one byte at a time, keeps its state between calls, and yields `char`s. Each maximal invalid sequence becomes one U+FFFD and the decoder resynchronizes (the WHATWG algorithm, so overlong forms, surrogates and values above U+10FFFF are invalid). The UART mirror still carries the raw bytes. `syscall/fd.rs`'s `console_write` is the caller.
- **Characters to glyphs (`font.rs`).** `glyph_for(c)` returns `unifont::get_glyph(c)`: a `Glyph::Halfwidth` (8x16 pixels) or `Glyph::Fullwidth` (16x16). The crate covers the Basic Multilingual Plane only, so a character with no glyph -- everything above U+FFFF included -- and any control character the console does not interpret draws U+FFFD. `cell_width(c)` is 2 for a fullwidth glyph, 0 for the few code points that draw nothing (`is_zero_width`: zero-width space, joiners and directional marks (U+200B..U+200F), the word joiner, variation selectors, the byte-order mark) and 1 otherwise; the width comes from the glyph itself, with no East Asian Width table.
- **Cells (`cells.rs`, pure).** A cell is still 8x16 pixels; a wide glyph takes two adjacent cells. The console keeps pixels and forgets what it drew, so `CellGrid` records which cells are the left or right half of a wide glyph. That is what lets Backspace move back one whole character, and what blanks the other half when a glyph overwrites half of a wide one. It scrolls with the pixels.
- **The cursor (`cells.rs`, pure).** `Cursor` wraps the way xterm does: a glyph that ends in the last column leaves the cursor there with a wrap *pending*, and the wrap (and any scroll) happens only when the next glyph arrives. Carriage return, newline, backspace, tab and explicit positioning clear it without wrapping, so a full row followed by `\n` leaves no blank row. A wide glyph that would not fit in what is left of the row wraps whole. A space after an automatic wrap is an ordinary glyph, so it lands in column 0 of the next row, as in every terminal.

The `Console` API changed with it: `Console::new` takes only the framebuffer; `putc` and `put_char` are replaced by `write_char` (which interprets `\n`, `\r`, `\t` and backspace, and draws everything else through the private `put_char_at_cursor` and `draw_glyph`), plus a new public `put_char_at(row, col, c, fg, bg)` for position-addressed drawing that never touches the cursor (the line editor's cursor cell); `size()` is gone, since `cols` and `rows` are public fields; and a typed line wraps over as many rows as it needs (`console/input_layout.rs`, drawn by the keyboard's line discipline), where earlier stages' `show_row` slid a one-row window sideways.

What it does not do: characters above the Basic Multilingual Plane (emoji included), combining marks (each draws standalone in its own cell), emoji sequences, bidirectional text, and shaping for complex scripts such as Arabic and Indic scripts -- text arrives in logical order and each character draws in isolation. No key of the US-only keymap can type any of it, so this affects only output from files and programs. The range-indexed font file that would add the astral plane, built from Unifont's `.hex` files (BDF and PCF only hold Plane 0), is described as the upgrade path in `Stage12.md`'s Step 2b.

## Block Device

We wrap the underlying `VirtIOBlk` driver and provide two functions (`drivers/virtio/blk.rs`):

- `read_blocks_irq()`: Read one or more 512-byte sectors from the device, starting at a block ID, into a specified buffer.
- `write_blocks_irq()`: Write one or more 512-byte sectors to the device, starting at a block ID, from a specified buffer.

Both functions submit the request and then block (sleeping with `wfe` between checks) until the device's completion interrupt has arrived, so to the caller they are synchronous.  We also provide an accessor to the underlying driver's `ack_interrupt()` method for handling interrupt acknowledgments, and `blk::Blk::capacity_bytes()`, which returns the device's total capacity in bytes by multiplying the underlying driver's `capacity()` (the total number of 512-byte blocks available) by the block size.

The filesystem built on the block device is described in [`filesystem.md`](filesystem.md). However, the functions above are not to be used directly; instead, block read/write operations should be performed via the higher-level FAT-based filesystem interface provided by the `hadris_fat` crate, driven by our `BlkIo` (`Read + Write + Seek`) implementation in `fs/blkio.rs`.

### Fat-based filesystem

We import the `hadris_fat` crate to provide FAT-based filesystem support on top of the block device. To do this, we implement the `Read`, `Write`, and `Seek` traits for a `BlkIo` struct (`fs/blkio.rs`), then pass an instance of it to `hadris_fat::sync::FatVolume::open()`.

`fs::find_entry_checked()` (`fs/mod.rs`) locates a specific file or directory entry within a given directory on the FAT volume, and `fs::read_file_checked()` reads the contents of a file into a `Vec<u8>` (converted from a `FileReader` instance from the underlying `read_file()`).

Writing is handled by the open-file table in `fs/files.rs` (`open` with write/append, `write`, `close`, plus `mkdir`, `unlink`, `rename` and `chmod`), which the `syscall/` layer calls, rather than by a one-shot wrapper like the read helpers above.

**File permissions:** The FAT-based filesystem uses a simplified permission model. Each file has an attribute byte, where certain bits indicate read-only status, hidden files, system files, and our **custom-defined** executable bit (using one of the two reserved bits, as defined by the `ATTR_EXEC` constant (`0x40`) in the `abi` crate).

**File names:** The FAT-based filesystem contains [LFN (long filenames)](https://en.wikipedia.org/wiki/Long_filename) support, allowing files to have names longer than the traditional 8.3 format. However, this comes with all of the usual limitations of LFN, such as the need for the short names of each file to still be unique within their directory and following the 8.3 naming convention.