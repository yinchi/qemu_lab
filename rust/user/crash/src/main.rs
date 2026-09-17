#![no_std]
#![no_main]

userlib::entry!(run);

fn run() {
    userlib::write(1, b"about to crash\n");

    // A canonical high/kernel-space-style address: invalid for a more
    // fundamental reason than "happens not to be mapped" -- the kernel
    // explicitly disables TTBR1_EL1 walks (TCR_EL1.EPD1), so this address
    // isn't in any translation regime at all, not just an unmapped entry
    // in the one that exists. A real load instruction, not something the
    // compiler could reason away as UB and optimize out.
    let bad_ptr = 0xffff_8000_0000_0000usize as *const u8;
    unsafe {
        core::ptr::read_volatile(bad_ptr);
    }

    // Unreachable if the fault above works as intended -- deliberately no
    // `exit()` call here, since reaching the fault is the whole point of
    // this program.
}
