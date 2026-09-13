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

The `virtio_drivers` crate provides abstractions for interacting with VirtIO devices. See `r06_virtio/gpu.rs` or `r06_virtio/blk.rs` for examples of how to instantiate drivers for specific VirtIO devices.

The VirtIO device memory regions typically contain control and status registers for device interaction, but do not generally store persistent data; a register in the 512-byte VirtIO slot points to the actual location of the device's data in RAM (MMIO = memory-mapped I/O).  This might include the framebuffer for a GPU, or a data buffer for a block device (itself pointing to the actual data blocks loaded from the device to RAM).

> [!NOTE]
> The VirtIO devices all use MMIO as specified in our QEMU settings, a PCI option also exists but is not used in this project.

## Other Memory Regions

There are several other memory regions defined in the `virt` machine, e.g.:

- `0x0901_0000` for the RTC (Real-Time Clock)
- `0x0902_0000` for fw-cfg (QEMU's firmware configuration interface, used to pass boot data such as the kernel command line to guest firmware/bootloaders)
- `0x0903_0000` for GPIO (General-Purpose Input/Output) devices
- `0x1000_0000` for PCI devices

However, none are used by this project.  The next significant memory region for our purposes is the RAM starting at `0x4000_0000`.

## `0x4000_0000`: RAM

Our linker places the `_start` symbol at the beginning of the RAM region, which is located at the memory address `0x4000_0000`. This is where the execution of our kernel begins.
