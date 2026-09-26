//! A fixed-capacity FIFO ring buffer: no allocation, and a full buffer refuses the *newest* item
//! (`push` hands it back) instead of overwriting an old one, so a burst of input that outruns the
//! reader keeps its beginning, in order, and loses only its tail. The keyboard's token queue is one
//! (`queue.rs`).
//!
//! Pure `no_std`, with no dependency on the rest of the kernel, so it is tested on the host
//! (`hosttests/`). It does no locking; whoever shares one between an interrupt handler and the code
//! it interrupts has to keep the two apart (`queue.rs` masks interrupts around each `pop`).

pub struct RingBuffer<T: Copy, const N: usize> {
    slots: [Option<T>; N],
    /// Index of the oldest item.
    head: usize,
    len: usize,
}

impl<T: Copy, const N: usize> RingBuffer<T, N> {
    pub const fn new() -> Self {
        Self {
            slots: [None; N],
            head: 0,
            len: 0,
        }
    }

    /// Adds `item` at the back. A full buffer is left as it was and returns the item as the error.
    pub fn push(&mut self, item: T) -> Result<(), T> {
        if self.len == N {
            return Err(item);
        }
        self.slots[(self.head + self.len) % N] = Some(item);
        self.len += 1;
        Ok(())
    }

    /// Removes and returns the oldest item, if any.
    pub fn pop(&mut self) -> Option<T> {
        if self.len == 0 {
            return None;
        }
        let item = self.slots[self.head].take();
        self.head = (self.head + 1) % N;
        self.len -= 1;
        item
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.len
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_ring_pops_nothing() {
        let mut r: RingBuffer<u8, 4> = RingBuffer::new();
        assert!(r.is_empty());
        assert_eq!(r.pop(), None);
    }

    #[test]
    fn items_come_out_in_the_order_they_went_in() {
        let mut r: RingBuffer<u8, 4> = RingBuffer::new();
        for i in 1..=3 {
            assert_eq!(r.push(i), Ok(()));
        }
        assert_eq!(r.len(), 3);
        assert_eq!(
            [r.pop(), r.pop(), r.pop(), r.pop()],
            [Some(1), Some(2), Some(3), None]
        );
        assert!(r.is_empty());
    }

    #[test]
    fn a_full_ring_refuses_the_newest_and_keeps_the_oldest() {
        let mut r: RingBuffer<u8, 3> = RingBuffer::new();
        assert_eq!([r.push(1), r.push(2), r.push(3)], [Ok(()), Ok(()), Ok(())]);
        assert_eq!(r.push(4), Err(4));
        assert_eq!(r.push(5), Err(5));
        assert_eq!(
            [r.pop(), r.pop(), r.pop(), r.pop()],
            [Some(1), Some(2), Some(3), None]
        );
    }

    #[test]
    fn it_wraps_around_many_times() {
        let mut r: RingBuffer<u16, 3> = RingBuffer::new();
        let mut next_in = 0u16;
        let mut next_out = 0u16;
        for round in 0..100 {
            // Alternate between filling it and draining it by different amounts.
            for _ in 0..(round % 3) + 1 {
                if r.push(next_in).is_ok() {
                    next_in += 1;
                }
            }
            for _ in 0..(round % 2) + 1 {
                if let Some(v) = r.pop() {
                    assert_eq!(v, next_out);
                    next_out += 1;
                }
            }
        }
        while let Some(v) = r.pop() {
            assert_eq!(v, next_out);
            next_out += 1;
        }
        assert_eq!(next_in, next_out);
    }

    #[test]
    fn space_freed_by_a_pop_can_be_used_again() {
        let mut r: RingBuffer<u8, 2> = RingBuffer::new();
        r.push(1).unwrap();
        r.push(2).unwrap();
        assert_eq!(r.push(3), Err(3));
        assert_eq!(r.pop(), Some(1));
        assert_eq!(r.push(3), Ok(()));
        assert_eq!([r.pop(), r.pop(), r.pop()], [Some(2), Some(3), None]);
    }
}
