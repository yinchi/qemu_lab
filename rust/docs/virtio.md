# VirtIO drivers and supporting modules

This file describes the VirtIO drivers and supporting modules implemented in Rust (as of r08_fs/).

## Overview

The VirtIO devices live in the `virtio_drivers::device` module. The three devices we use are:

- `VirtIOInput` (for a keyboard)
- `VirtIOBlk` (for block storage)
- `VirtIOGpu` (for graphics)

### Device discovery

In our QEMU `virt` device, the machine's `x0` register is initialized to the address of a [device tree blob (DTB)](https://devicetree-specification.readthedocs.io/en/stable/flattened-format.html), which contains information about the hardware devices available to the system. Our `boot.S` assembly code passes this address to the Rust kernel, which then parses the DTB to discover the available VirtIO devices.

The first eight bytes of the DTB contain:

```
0x0 D0 0D FE ED
0x4 <blob size in bytes, big-endian>
```

`base_addresses::init_base_addresses` copies the blob into RAM and parses it using the `fdt` (flattened device tree) crate.  In particular, it looks for nodes corresponding to the VirtIO devices, which contain a `compatible` field containing the string `"virtio,mmio"` (memory-mapped I/O).  For each such node, it extracts a `reg` field, which specifies the base address and size of the device's memory-mapped I/O region (though in practice this size is always `0x200` on our `virt` machine, so our code hardcodes it rather than reading it from the DTB), and an `interrupts` field, which specifies the interrupt line used by the device.  For our `virt` machine, these interrupts are on SPI (Shared Peripheral Interrupt) lines, e.g. SPI 1 &rarr; Interrupt ID 33 (offset=32).

Finally, each device driver in our kernel detects the presence of its corresponding VirtIO device by checking each base address extracted from the DTB and attempting to parse a `VirtIOHeader`. If a valid header is found and corresponds to a valid `MmioTransport` memory region, the driver then checks the device's `.device_type()` to ensure it matches the expected type for that driver. This repeats until the correct device is found or all base addresses have been checked.

See (for example) `keyboard::Keyboard::find` for an example of VirtIO device discovery and initialization.

### HAL (Hardware Abstraction Layer)

Each of the three VirtIO drivers is generic over `virtio_drivers::Hal`, the trait the crate uses to abstract over DMA (Direct Memory Access) allocation and physical/virtual address translation. We provide a single implementation, `virtio_hal::VirtioHalImpl`, shared by all three drivers.

Because this stage never enables the MMU (Memory Management Unit -- the hardware that would otherwise let code's virtual addresses map to different physical addresses) and every `virtio,mmio` DTB node is confirmed `dma-coherent`, the implementation is deliberately trivial:

- `dma_alloc()`: hands out pages from a single static, page-aligned `DMA_POOL` (512 pages / 2 MiB) by bumping a monotonically increasing offset (`DMA_POOL_NEXT`); since the pool starts zeroed, nothing further needs initializing.
- `dma_dealloc()`: unimplemented. Every device we use reports MMIO version Legacy, so each sets up its DMA region with a single `Dma::new()` held for the program's entire lifetime and never dropped early -- so it's never called.
- `mmio_phys_to_virt()`: normally translates a device's physical MMIO base address into a virtual address the driver can dereference; here it's an identity mapping (physical == virtual), since the MMU is off.
- `share()`: normally hands a buffer to the device by returning the physical address its DMA should target, doing whatever cache-flushing or bounce-buffering is needed so the CPU and device agree on the buffer's contents. Here it's just that address translation, since there's no IOMMU (Input-Output MMU -- hardware that would otherwise remap or restrict which physical addresses a device's DMA can target, the same role the MMU plays for the CPU) to program, and DMA is already coherent.
- `unshare()`: the inverse of `share()` -- normally reclaims a shared buffer for CPU use, undoing any bounce-buffering or invalidating caches as needed. Here a no-op, for the same reasons as `share()`.

Because `dma_alloc()` never frees memory, exhausting `DMA_POOL` returns a dangling pointer rather than panicking -- allocation failure is left for the `virtio_drivers` crate itself to handle.

## Keyboard

The VirtIOInput driver is generic, supporting various input devices such as keyboards and mice, but in our QEMU setup, is tied to a keyboard.  It interacts with the keyboard hardware through two main functions: `pop_pending_event()`, called to retrieve the next pending input event after a keyboard interrupt is received, and `ack_interrupt()`, called to acknowledge that the corresponding interrupt has been acknowledged  (paired with `arm_gic::gicv2::GicV2::end_interrupt()` to signal that interrupt handling is complete).

`keyboard::Keyboard::poll()`, a thin wrapper around `pop_pending_event()`, retrieves a `InputEvent` representing the next pending input event from the keyboard.  `InputEvent.event_type` is then checked: for keyboard events, it should be 1.  Other important fields are:

- `InputEvent.code`: the key code of the key that generated the event.
- `InputEvent.value`: the value of the event, (0 for release, 1 for press, 2 for autorepeat).

As of r08_fs/, autorepeat events are ignored by the keyboard driver.

### Keymap

`keymap.rs` defines a human-readable strings for a large number of key codes, with a `Knnn` fallback for unknown key codes.  It also provides structs for storing held key state and lock key state (Caps Lock, Num Lock, Scroll Lock).

```rust
pub struct LockState {
    pub caps: bool,
    pub num: bool,
    pub scroll: bool,
}

pub struct KeyState {
    held: [bool; MAX_CODE],
    /// The last key that was pressed (if still held), else None.
    /// In other words, `None` may indicate that no key is currently held, or that the last
    /// key pressed has been released.
    last_held: Option<u16>,
}
```

For example, if the user presses and releases Caps Lock, then holds down 'A' while pressing and releasing 'B', the `LockState` would reflect that Caps Lock is active, and the `KeyState` would contain 'A' only in the `held` array, with `last_held` set to None to indicate that the last key pressed ('B') has been released.  This reflects the fact that in most environments with autorepeat enabled, only the last held key is considered for autorepeat purposes, i.e. releasing 'B' does not revert autorepeat to 'A'.

## GPU

The GPU driver holds a pointer to a framebuffer in RAM, and provides the functions `flush()`, which dumps the framebuffer contents to the display, and `change_resolution()`, which changes the display resolution and replaces the framebuffer with a new one matching the new resolution (in our program, this is only called once at initialization, with the resolution fixed thereafter).

We also provide a `Console` abstraction that interacts with the GPU driver to render text output to the display, using functions such as `put_char` and `move_cursor`.  A call chain may include (all within `console.rs`):

- `write_char()`: handles `\n`, `\r`, and `\t`, which move the cursor accordingly (a `\n` on the last row also calls `scroll_up()` to make room for the new line), and otherwise writes a character to the current cursor position using `putc`.
- `putc()`: writes a character to the current cursor position on the framebuffer, using `put_char`, and then advances the cursor position accordingly — note that this does not itself scroll if writing runs past the last row.
- `put_char()`: writes a character directly to the framebuffer at the specified position.
- `scroll_up()`: shifts the framebuffer's pixel contents up by one character row and clears the newly-exposed last row.
- `size()`: returns the current size of the console as `(cols, rows)` rather than pixels.
- `move_cursor()`: moves the cursor to a specified position on the display.

Currently, although the GPU driver is pixel-based, we only interact with it through the character-based `Console` interface, except that `Console::clear()` operates at the pixel level to clear the entire screen.

### Font handling

Font handling in the console is managed through a bitmap font, where each character is represented as a grid of pixels. Extracting the bitmap for a specific glyph is handled by the `font::Font::glyph()` function. Since each character is 8x16, the returned bitmap is an array of 16 bytes, with each byte representing a row of 8 pixels.

The glyph bitmaps are packed into a single contiguous array in CP437 order; however, the rest of the system typically interacts with characters using Unicode code points. Thus, `cp437::unicode_to_cp437()` provides the necessary mappings between Unicode code points and CP437 indices, allowing the console to correctly retrieve the corresponding glyph bitmaps for display.

## Block Device

We wrap the underlying `VirtIOBlk` driver and provide two functions (`blk.rs`):

- `read_blocks_irq()`: Read a block from the device by ID and copy its contents to a specified buffer.
- `write_blocks_irq()`: Write a block to the device by ID from a specified buffer.

Both functions operate asynchronously and rely on interrupts to signal completion.  We also provide an accessor to the underlying driver's `ack_interrupt()` method for handling interrupt acknowledgments, and `blk::Blk::capacity_bytes()`, which returns the device's total capacity in bytes by multiplying the underlying driver's `capacity()` (the total number of 512-byte blocks available) by the block size.

However, the functions above are not to be used directly; instead, block read/write operations should be performed via the higher-level FAT-based filesystem interface provided by the `hadris_fat` crate, driven by our `BlkIo` (`Read + Write + Seek`) implementation in `fat_io.rs`.

### Fat-based filesystem

We import the `hadris_fat` crate to provide FAT-based filesystem support on top of the block device. To do this, we implement the `Read`, `Write`, and `Seek` traits for a `BlkIo` struct (`fat_io.rs`), then pass an instance of it to `hadris_fat::sync::FatVolume::open()`.

A `find_entry()` function is provided to locate a specific file or directory entry within a given directory on the FAT volume, as well as a `read_file_to_vec()` wrapper to read the contents of a file into a `Vec<u8>` (converted from a `FileReader` instance from the underlying `read_file()`).

Note: a wrapper function for file writes is not yet implemented, but would use `hadris_fat`'s corresponding `FileWriter` struct.

**File permissions:** The FAT-based filesystem uses a simplified permission model. Each file has an attribute byte, where certain bits indicate read-only status, hidden files, system files, and our **custom-defined** executable bit (using one of the two reserved bits, as defined by the `EXEC_BIT` constant in the main code).

**File names:** The FAT-based filesystem contains [LFN (long filenames)](https://en.wikipedia.org/wiki/Long_filename) support, allowing files to have names longer than the traditional 8.3 format. However, this comes with all of the usual limitations of LFN, such as the need for the short names of each file to still be unique within their directory and following the 8.3 naming convention.