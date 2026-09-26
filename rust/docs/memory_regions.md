# Memory Regions and memory-mapped devices

This document outlines some of the key memory regions and memory-mapped devices for the QEMU `virt` machine (as used by our kernel project), including their base addresses and sizes.

## `0x0000_0000`: boot ROM/flash memory

For the QEMU `virt` machine specifically, the "boot ROM" memory contains a device tree blob (DTB) that describes the hardware layout to the kernel.

```rust
static mut DTB_BUFFER: [u8; DTB_BUF_SIZE] = [0; DTB_BUF_SIZE];
```

The DTB contains its size at offset `0x4` within the blob; our `base_addresses.rs` script copies this many bytes from offset `0x0` of the boot ROM into a `&[u8]` slice for further processing, i.e. discovering the rest of the memory layout.

## `0x0800_0000`: GIC (Generic Interrupt Controller)

The start of this memory region is found by searching for a node in the parsed device tree with a `compatible` property set to `"arm,cortex-a15-gic"`.

```console
> dtc -I dtb -O dts /tmp/virt.dtb 2>/dev/null | grep -B3 'arm,cortex-a15-gic'
        intc@8000000 {
                phandle = <0x8002>;
                reg = <0x00 0x8000000 0x00 0x10000 0x00 0x8010000 0x00 0x10000>;
                compatible = "arm,cortex-a15-gic";
```

This shows that the GIC is located at the memory address `0x8000000` with a size of `0x10000` for the distributor and `0x10000` for the CPU interface, as indicated by the `reg` property in the device tree (64-bit alternating address/size pairs).

The `arm_gic::gicv2::GicV2` struct in the `arm_gic` crate provides an API for interacting with the GICv2 distributor and CPU interface, and its constructor `new` takes the base addresses of the distributor and CPU interface as arguments.

```rust
// In our actual code, these addresses are taken from the device tree, by
// copying it from the 0x0 memory region and parsing it in-kernel.
let gic = unsafe {
    arm_gic::gicv2::GicV2::new(0x8000000 as *mut u32, 0x8010000 as *mut u32)
};
```

## `0x0900_0000`: UART (Universal Asynchronous Receiver/Transmitter)

The UART serial port is found by searching for a node in the parsed device tree with a `compatible` property set to `"arm,pl011"`.

```console
> dtc -I dtb -O dts /tmp/virt.dtb 2>/dev/null | grep -B5 -A1 'arm,pl011'
        pl011@9000000 {
                clock-names = "uartclk\0apb_pclk";
                clocks = <0x8000 0x8000>;
                interrupts = <0x00 0x01 0x04>;
                reg = <0x00 0x9000000 0x00 0x1000>;
                compatible = "arm,pl011\0arm,primecell";
        };
```

This shows that the UART is located at the memory address `0x9000000` with a size of `0x1000`, as indicated by the `reg` property in the device tree (64-bit alternating address/size pairs).  However, in our code, we hardcode this base address in case the device tree is not available at runtime, or parsing it fails, so that our UART driver can still function correctly and print an error message if necessary.

```rust
// base_addresses.rs
pub const UART0_BASE: usize = 0x0900_0000;

// uart.rs
impl Uart {
    pub const fn new(base: usize) -> Self {
       // ...
    }
}
```

## `0x0A00_0000`: VirtIO devices

VirtIO devices on the `virt` machine are defined starting from memory address `0x0A00_0000` and can be found via the device tree with a `compatible` property set to `"virtio,mmio"`.

```console
> dtc -I dtb -O dts /tmp/virt.dtb 2>/dev/null | grep -B4 -A1 'virtio,mmio'
        virtio_mmio@a000000 {
                dma-coherent;
                interrupts = <0x00 0x10 0x01>;
                reg = <0x00 0xa000000 0x00 0x200>;
                compatible = "virtio,mmio";
        };
--
        virtio_mmio@a000200 {
```

and so on up to:

```console
        virtio_mmio@a003e00 {
                dma-coherent;
                interrupts = <0x00 0x2f 0x01>;
                reg = <0x00 0xa003e00 0x00 0x200>;
                compatible = "virtio,mmio";
        };
```

This gives us 32 VirtIO device slots, each occupying `0x200` (512) bytes of memory. To check if a particular slot is actually associated with a device, we read the following offsets within the slot's memory region:

- `0x000`: Magic value to identify the presence of a VirtIO device.
- `0x004`: Version of the VirtIO specification implemented by the device.
- `0x008`: Device ID to identify the type of VirtIO device.

The `virtio_drivers` crate provides abstractions for interacting with VirtIO devices. See `drivers/virtio/gpu.rs` or `drivers/virtio/blk.rs` in `<stage>/src/` (first written in `r06_virtio/`) for examples of how to instantiate drivers for specific VirtIO devices. The drivers themselves, and the HAL they share, are described in [`virtio.md`](virtio.md).

The VirtIO device memory regions typically contain control and status registers for device interaction, but do not generally store persistent data; a register in the 512-byte VirtIO slot points to the actual location of the device's data in RAM (MMIO = memory-mapped I/O).  This might include the framebuffer for a GPU, or a data buffer for a block device (itself pointing to the actual data blocks loaded from the device to RAM).

> [!NOTE]
> The VirtIO devices all use MMIO as specified in our QEMU settings, a PCI option also exists but is not used in this project.

## Other Memory Regions

There are several other memory regions defined in the `virt` machine, e.g.:

- `0x0901_0000` for the RTC (Real-Time Clock), a PL031. The kernel maps this one page (`RTC_BASE`, `platform/base_addresses.rs`) as device memory and reads its data register, `RTCDR` at offset 0: a read-only count of seconds since the Unix epoch, which QEMU sets from the host's clock at start-up (`platform/rtc.rs`, the `clock_gettime` syscall, and `date`). Its load, match and interrupt registers are not used
- `0x0902_0000` for fw-cfg (QEMU's firmware configuration interface, used to pass boot data such as the kernel command line to guest firmware/bootloaders)
- `0x0903_0000` for GPIO (General-Purpose Input/Output) devices
- `0x1000_0000` for PCI devices

However, none are used by this project.  The next significant memory region for our purposes is the RAM starting at `0x4000_0000`.

## `0x4000_0000`: RAM

Our linker places the `_start` symbol at the beginning of the RAM region, which is located at the memory address `0x4000_0000`. This is where the execution of our kernel begins.

### The kernel image

`link.ld` lays the kernel image out from `0x4000_0000` in this order, exporting a boundary symbol
(`__text_start`, `__rodata_end`, ...) for each region so that `arch/mmu.rs` can map each with its own
permissions (see [`mmu.md`](mmu.md)). Because permissions are per 4 KiB page, every boundary is
page-aligned.

| Region | Mapped as | What lives there |
|---|---|---|
| `.text` | read + execute (`kernel_rx`) | Kernel code, including `_start` (`.text.boot`) and the exception vectors |
| `.rodata` | read-only (`kernel_ro`) | Constants, string literals, the Unifont glyph tables |
| `.data`, `.bss` | read + write (`kernel_rw`) | Every `static`: the kernel heap, the VirtIO DMA pool, the device-tree buffer (`DTB_BUFFER`, 2 MiB), `KERNEL_CTX`, the exception stack, the device globals |
| stack guard | **unmapped** | 64 KiB (`__stack_guard`..`__stack_bottom`), so a stack overflow faults |
| kernel stack | read + write (`kernel_rw`) | The one kernel stack (1 MiB), ending at `stack_top` = `__kernel_end` |

The whole image is about 23 MiB, most of it the heap. Everything from `__kernel_end` up to the user
window at `0x4400_0000` (roughly 41 MiB) is left unmapped. The user window itself is 32 MiB from there (`0x4400_0000` to `0x4600_0000`); QEMU's default 128 MiB of RAM ends at `0x4800_0000`, so the top 32 MiB is left free.

### The kernel stack

There is one kernel stack, 1 MiB, reserved after `.bss` (and the guard below it) in `link.ld`;
`boot.s` selects `SP_EL1` and points it at `stack_top` before calling `kernel_main`. It holds:

- **Ordinary Rust call frames** &mdash; `kernel_main`, the shell's read-eval loop (`shell::run`), and
  everything it calls down to `process::run`. While a user program runs, all of these frames stay
  live on the stack, waiting. Small values that are passed around by value live here too, such as a
  `PreparedProgram` (four `usize`s).
- **One trap frame per exception taken to EL1** (a syscall or a fault from EL0, or an interrupt, from
  EL0 or from kernel code): 272 bytes &mdash; `x0`&ndash;`x30`, `ELR_EL1` and `SPSR_EL1` &mdash; pushed by
  `arch/vectors.s`, plus the handler's own frame on top. Exceptions use `SP_EL1`, so the frame lands on
  this same stack, just below whatever was running (for an exception from EL0, `process::run`'s live
  frame). It is popped when the handler returns, or simply discarded when `resume_kernel` restores `sp`
  after an `exit` or a fault.

It does **not** hold the registers `enter_el0` checkpoints (`sp`, the resume address and
`x19`&ndash;`x30`): those go in `KERNEL_CTX`, a 14-word static in `.bss` (see
[`launching_programs.md`](launching_programs.md)).

#### The guard and the exception stack

Below the stack lies a 64 KiB guard that `arch/mmu.rs` leaves unmapped (it maps `.data`/`.bss` and
the stack as two separate regions), so an overflow is a translation fault instead of silent
corruption of `.bss`. (Before the guard existed the stack grew straight into the end of `.bss`, all
of it mapped read-write, so an overflow silently overwrote whatever statics lay there.) The user
stack has the same kind of guard (see [`mmu.md`](mmu.md)).

The fault is an ordinary data abort taken at EL1, so it arrives at the `sync_el1h` entry of the
exception vector table (Current EL with `SPx`, Synchronous, index 4) &mdash; the same entry every
synchronous exception at EL1 already used. But the standard entry sequence pushes a 272-byte trap
frame onto `sp`, which is exactly what has just faulted. So `sync_el1h` first switches to a dedicated
64 KiB exception stack (`EXCEPTION_STACK_TOP`, a static in `.bss` defined in `arch/vectors.s`) and
only then saves registers and calls `unexpected_exception`. That function reads `FAR_EL1`; if the
faulting address lies in the guard it panics with "Kernel stack overflow", otherwise with the usual
"Unexpected exception". Every synchronous exception at EL1 is fatal here anyway, so the switch
applies to all of them, and the exception stack itself has no guard &mdash; the path on it only prints
a message and hangs.

### The kernel heap

The heap is `static mut HEAP`, a 16 MiB array in `.bss`, managed by `linked_list_allocator`'s
`LockedHeap` as the crate's `#[global_allocator]` and initialised first thing in `kernel_main`. It is
where everything that allocates lives &mdash; every `Vec`, `String` and `BiMap`:

- The whole ELF file `launch` reads into memory before running it (capped at `MAX_PROGRAM_SIZE`,
  half the heap), plus the temporary argument plan.
- The shell's own data: the lexer's tokens and the parsed pipeline for each line, the line buffer and
  the command history, the `KEY_NAMES` map.
- Filesystem state: directory listings and open-file readers/writers, each with `hadris-fat`'s own
  buffers.

The VirtIO DMA pool is *not* part of the heap: it is a separate 2 MiB, page-aligned static in `.bss`
(`drivers/virtio/hal.rs`) that holds the virtqueues and the 1.2 MiB framebuffer. (A disk read or write goes through a
512-byte buffer on the caller's stack, not the pool.)
