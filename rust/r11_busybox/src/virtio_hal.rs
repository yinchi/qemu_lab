//! `virtio_drivers::Hal` (Hardware Abstraction Layer) implementation for this platform.
//!
//! Deliberately trivial, for two reasons confirmed from the device tree
//! rather than assumed: every `virtio,mmio` node carries a `dma-coherent`
//! property (DMA and CPU caches are always consistent), and although this stage *does* enable
//! the MMU (`mmu.rs`, unlike earlier stages), it's one flat identity mapping -- physical and
//! virtual addresses are still always equal here, just via an explicit page table now rather
//! than there being no translation at all. There is also no IOMMU;
//! so `share` and `unshare` are effectively no-ops.

use core::ptr::NonNull;
use core::sync::atomic::{AtomicUsize, Ordering};

use virtio_drivers::{BufferDirection, Hal, PAGE_SIZE, PhysAddr};

// Backing store for every VirtIO Direct Memory Access (DMA) allocation this program ever makes.
// 2 MiB, so that our font table for this program fits comfortably.
/// Number of pages in the Direct Memory Access (DMA) pool.
const DMA_POOL_PAGES: usize = 512;
/// Size of the Direct Memory Access (DMA) pool in bytes.
const DMA_POOL_SIZE: usize = DMA_POOL_PAGES * PAGE_SIZE;

/// Aligned backing store for the Direct Memory Access (DMA) pool.  4096 is the PAGE_SIZE as
/// checked against `virtio_drivers::PAGE_SIZE`.
#[repr(align(4096))]
struct AlignedPool([u8; DMA_POOL_SIZE]);

/// The actual DMA pool (AlignedPool instance).
static mut DMA_POOL: AlignedPool = AlignedPool([0; DMA_POOL_SIZE]);

/// Byte offset of the next unused page in `DMA_POOL`. Only ever grows --
/// this program's DMA block device memory is write-once.
static DMA_POOL_NEXT: AtomicUsize = AtomicUsize::new(0);

/// Implementation of the `virtio_drivers::Hal` trait for this platform.
/// (HAL = hardware abstraction layer).
pub struct VirtioHalImpl;
// SAFETY: see this module's doc comment -- coherent DMA, MMU off, no IOMMU.
unsafe impl Hal for VirtioHalImpl {
    /// Allocate `pages` number of pages from the DMA pool.
    /// Since the pool starts out zeroed all we need is to return a memory address
    /// after reserving the memory.
    fn dma_alloc(pages: usize, _direction: BufferDirection) -> (PhysAddr, NonNull<u8>) {
        let size = pages * PAGE_SIZE;
        let offset = DMA_POOL_NEXT.fetch_add(size, Ordering::Relaxed);

        // DMA pool exhausted: return a null pointer.
        if offset + size > DMA_POOL_SIZE {
            return (0, NonNull::dangling());
        }

        // SAFETY: `offset..offset+size` was just reserved above and isn't
        // handed out to anyone else -- DMA_POOL_NEXT only ever increases.
        let ptr = unsafe { &raw mut DMA_POOL.0[offset] };

        // Return the physical address and a non-null pointer to the allocated memory.
        (ptr as usize as PhysAddr, NonNull::new(ptr).unwrap())
    }

    /// UNIMPLEMENTED: deallocate `pages` number of pages from the DMA pool, returning 0 on success.
    ///
    /// Deliberately unimplemented:
    ///
    /// 1. Every device we actually use -- blk, gpu, and keyboard -- reports MMIO version Legacy on
    ///    this QEMU build, so each one's `VirtQueue` sets up its DMA region with a single
    ///    `Dma::new()`, held for the device's entire lifetime and never dropped early.
    /// 2. Every device lives for the entire lifetime of the program.
    /// 3. The framebuffer for the GPU device is also allocated once and lives for the entire
    ///    lifetime of the program; changing resolution or reinitializing the framebuffer is not
    ///    supported.
    ///
    /// Because of this, dma_dealloc is never called and dma_alloc can use a monotonic increasing
    /// allocation strategy (DMA_POOL_NEXT).
    ///
    /// Note: MMIO version Modern would potentially call `dma_dealloc` if it fails (e.g. second
    /// allocation stage fails, first stage goes out of scope and Rust automatically Drops it),
    /// but no devices in our setup use Modern.
    unsafe fn dma_dealloc(_paddr: PhysAddr, _vaddr: NonNull<u8>, _pages: usize) -> i32 {
        unimplemented!()
    }

    /// Convert a physical MMIO (memory-mapped I/O) address to a virtual address.
    /// Identity mapping: physical and virtual addresses are the same on this device.
    unsafe fn mmio_phys_to_virt(paddr: PhysAddr, _size: usize) -> NonNull<u8> {
        NonNull::new(paddr as usize as *mut u8).expect("mmio_phys_to_virt: null physical address")
    }

    /// Share a buffer with the device, returning its physical address.
    /// Identity mapping: physical and virtual addresses are the same on this device.
    unsafe fn share(buffer: NonNull<[u8]>, _direction: BufferDirection) -> PhysAddr {
        buffer.as_ptr() as *mut u8 as usize as PhysAddr
    }

    /// Unshare a buffer with the device.
    /// No-op: nothing to do for unsharing in an identity-mapped, coherent DMA setup.
    unsafe fn unshare(_paddr: PhysAddr, _buffer: NonNull<[u8]>, _direction: BufferDirection) {}
}
