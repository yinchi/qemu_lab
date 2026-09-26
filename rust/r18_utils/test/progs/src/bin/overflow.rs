//! `overflow` -- recurses without end, so it runs off the bottom of its stack. The kernel must stop it with a
//! fault (`Segmentation fault ...`, exit status 139) and go on, not let it write over anything else -- see
//! Step 3 of `Stage12.md`.

#![no_std]
#![no_main]

userlib::entry!(run);

fn run() {
    userlib::write(1, b"overflowing\n");
    let n = recurse(1);
    // Never reached; `n` keeps the recursion from being turned into a loop.
    userlib::write(1, &[n as u8]);
}

/// A frame with a real, touched array, so each call needs its own stack space.
#[inline(never)]
#[allow(unconditional_recursion)] // the point
fn recurse(depth: usize) -> usize {
    let mut frame = [0u8; 4096];
    // SAFETY: a plain write to our own array; volatile so the frame is not optimized away.
    unsafe { core::ptr::write_volatile(&mut frame[depth % 4096], depth as u8) };
    let below = recurse(depth + 1);
    below + unsafe { core::ptr::read_volatile(&frame[depth % 4096]) } as usize
}
