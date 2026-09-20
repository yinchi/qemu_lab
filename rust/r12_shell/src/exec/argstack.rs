//! Where a new program's argument strings and pointer array go on its initial stack -- the address
//! bookkeeping half of `ROADMAP.md`'s Stage 10 `argc`/`argv` design, pure so it can be unit-tested on
//! the host (`hosttests/`); `process.rs` does the actual writes.
//!
//! Working *downward* from where the stack pointer currently is (the direction the stack grows), the
//! strings go first, each NUL-terminated, then a `NULL`-terminated array of pointers to them, its
//! base 16-byte aligned. Wherever that base ends up is the new stack pointer. The same shape is
//! reused for `envp` in Stage 16, hence the array-at-a-time interface.

use alloc::vec::Vec;

/// The addresses to write one array's strings and pointers at.
#[derive(Debug, PartialEq, Eq)]
pub struct ArrayPlan {
    /// The address of each string, in argument order (the first is the highest).
    pub strings: Vec<usize>,
    /// The address of the pointer array: `strings.len()` entries, then a `NULL` -- the C convention
    /// (`argv[argc] == NULL`). 16-byte aligned, and the stack pointer to hand the program if
    /// nothing else gets pushed below it.
    pub array: usize,
}

/// Plans one array of NUL-terminated strings of the given lengths below `sp`, or `None` if it
/// wouldn't fit above `floor` (or the arithmetic would wrap).
pub fn plan(sp: usize, floor: usize, lens: &[usize]) -> Option<ArrayPlan> {
    let mut cursor = sp;
    let mut strings = Vec::with_capacity(lens.len());
    for &len in lens {
        cursor = cursor.checked_sub(len.checked_add(1)?)?; // +1 for the NUL terminator
        strings.push(cursor);
    }
    let pointers = lens.len().checked_add(1)?.checked_mul(core::mem::size_of::<usize>())?;
    let array = cursor.checked_sub(pointers)? & !0xf;
    (array >= floor).then_some(ArrayPlan { strings, array })
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOP: usize = 0x4420_0000;
    const FLOOR: usize = TOP - 0x1_0000;

    #[test]
    fn no_arguments_is_just_a_null_slot() {
        let p = plan(TOP, FLOOR, &[]).unwrap();
        assert!(p.strings.is_empty());
        assert_eq!(p.array, (TOP - 8) & !0xf);
    }

    #[test]
    fn strings_descend_in_order_and_do_not_overlap() {
        let lens = [4, 0, 7, 1];
        let p = plan(TOP, FLOOR, &lens).unwrap();
        assert_eq!(p.strings[0], TOP - 5);
        for i in 1..lens.len() {
            assert_eq!(p.strings[i], p.strings[i - 1] - (lens[i] + 1));
        }
        // The last string is above the pointer array, with room for argc + 1 pointers.
        assert!(p.array + 8 * (lens.len() + 1) <= *p.strings.last().unwrap());
    }

    #[test]
    fn the_array_base_is_sixteen_byte_aligned_whatever_the_lengths() {
        for a in 0..40 {
            for n in 0..6 {
                let lens = alloc::vec![a; n];
                let p = plan(TOP - a, FLOOR, &lens).unwrap();
                assert_eq!(p.array % 16, 0, "a={a} n={n}");
            }
        }
    }

    #[test]
    fn refuses_what_does_not_fit_or_would_wrap() {
        assert!(plan(TOP, FLOOR, &[0x8000, 0x8000]).is_none());
        assert!(plan(TOP, FLOOR, &[0x1_0000 - 16]).is_none());
        assert!(plan(TOP, FLOOR, &[usize::MAX]).is_none());
        assert!(plan(TOP, FLOOR, &[usize::MAX - 1]).is_none());
        assert!(plan(4, 0, &[8]).is_none());
        assert!(plan(0x40, 0, &[0x100]).is_none());
        // Exactly fitting is fine: nothing between the array and the floor.
        let p = plan(TOP, FLOOR, &[]).unwrap();
        assert!(plan(TOP, p.array, &[]).is_some());
        assert!(plan(TOP, p.array + 16, &[]).is_none());
    }

    #[test]
    fn thirty_arguments() {
        let lens = alloc::vec![2; 30];
        let p = plan(TOP, FLOOR, &lens).unwrap();
        assert_eq!(p.strings.len(), 30);
        assert_eq!(p.array % 16, 0);
    }
}
