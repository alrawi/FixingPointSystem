// SPDX-License-Identifier: GPL-3.0-only
//! The single producer, single consumer rings that AF_XDP shares with the
//! kernel.
//!
//! The fill and transmit rings have this process as producer, and the
//! completion and receive rings have it as consumer. Each ring is a pair of
//! `u32` indices that only ever grow and wrap, plus an array of a power of two
//! entries. The producer writes entries and then publishes the new producer
//! index with release ordering, and the consumer reads the producer index with
//! acquire ordering before it reads entries, so an entry is complete when its
//! index is visible. The consumer index works the same way in the other
//! direction, so the producer never overwrites an entry that the consumer has
//! not read. This module and `sys` are the only ones with `unsafe` code.

use std::sync::atomic::{AtomicU32, Ordering};

use crate::sys::Mapping;

/// The memory of one ring: the two indices, an optional flag word, and the
/// entries.
struct RingMem<T> {
    producer: *const AtomicU32,
    consumer: *const AtomicU32,
    flags: *const AtomicU32,
    entries: *mut T,
    mask: u32,
    size: u32,
    /// Keeps the kernel mapping alive. It is `None` for rings in tests.
    _map: Option<Mapping>,
}

// SAFETY: the pointers refer to memory that `_map` owns, or that the test
// that built the ring keeps alive. A ring is used from one thread at a time,
// and the kernel is the only other party, which the atomics order.
unsafe impl<T: Send> Send for RingMem<T> {}

impl<T: Copy> RingMem<T> {
    /// Builds a ring over the mapping `map` with the offsets that the kernel
    /// reported, and `size` entries.
    fn from_mapping(
        map: Mapping,
        producer: u64,
        consumer: u64,
        desc: u64,
        flags: u64,
        size: u32,
    ) -> Self {
        let base = map.as_ptr();
        // SAFETY: the kernel reported these offsets for a mapping that
        // `Mapping::of_socket` sized as `desc + size * entry size`, so each
        // pointer stays inside the mapping. The indices and flags are 4 byte
        // aligned inside a page aligned mapping.
        let (producer, consumer, flags, entries) = unsafe {
            (
                base.add(producer as usize).cast::<AtomicU32>().cast_const(),
                base.add(consumer as usize).cast::<AtomicU32>().cast_const(),
                base.add(flags as usize).cast::<AtomicU32>().cast_const(),
                base.add(desc as usize).cast::<T>(),
            )
        };
        Self {
            producer,
            consumer,
            flags,
            entries,
            mask: size - 1,
            size,
            _map: Some(map),
        }
    }

    fn producer(&self) -> &AtomicU32 {
        // SAFETY: the pointer is valid for the life of `self`, see
        // `from_mapping`, and atomics tolerate concurrent access.
        unsafe { &*self.producer }
    }

    fn consumer(&self) -> &AtomicU32 {
        // SAFETY: as in `producer`.
        unsafe { &*self.consumer }
    }
}

/// The producer end of a ring, for the fill and transmit rings.
pub(crate) struct ProducerRing<T> {
    mem: RingMem<T>,
    /// The next index to write, not yet visible to the consumer.
    head: u32,
    /// The consumer index when we last read it.
    seen_consumer: u32,
}

impl<T: Copy> ProducerRing<T> {
    pub(crate) fn from_mapping(
        map: Mapping,
        producer: u64,
        consumer: u64,
        desc: u64,
        flags: u64,
        size: u32,
    ) -> Self {
        Self::new(RingMem::from_mapping(
            map, producer, consumer, desc, flags, size,
        ))
    }

    fn new(mem: RingMem<T>) -> Self {
        let head = mem.producer().load(Ordering::Relaxed);
        let seen_consumer = mem.consumer().load(Ordering::Acquire);
        Self {
            mem,
            head,
            seen_consumer,
        }
    }

    /// The number of entries, up to `want`, that fit without overwriting an
    /// unread one. It reads the consumer index only when the cached value
    /// shows too little room.
    pub(crate) fn free(&mut self, want: u32) -> u32 {
        let mut free = self.mem.size - self.head.wrapping_sub(self.seen_consumer);
        if free < want {
            self.seen_consumer = self.mem.consumer().load(Ordering::Acquire);
            free = self.mem.size - self.head.wrapping_sub(self.seen_consumer);
        }
        free.min(want)
    }

    /// Writes `value` at the head of the ring and returns `false`, writing
    /// nothing, when the ring is full as of the last [`ProducerRing::free`].
    /// The entry becomes visible to the kernel at [`ProducerRing::submit`].
    pub(crate) fn push(&mut self, value: T) -> bool {
        if self.head.wrapping_sub(self.seen_consumer) >= self.mem.size {
            return false;
        }
        // SAFETY: the index is masked into the array of `size` entries. The
        // check above shows that the consumer has read the entry that this
        // index last held, so the kernel does not read it now.
        unsafe {
            self.mem
                .entries
                .add((self.head & self.mem.mask) as usize)
                .write(value)
        };
        self.head = self.head.wrapping_add(1);
        true
    }

    /// Publishes every pushed entry to the consumer.
    pub(crate) fn submit(&mut self) {
        self.mem.producer().store(self.head, Ordering::Release);
    }

    /// The consumer index, which shows how many submitted entries the kernel
    /// has taken.
    pub(crate) fn consumer_index(&self) -> u32 {
        self.mem.consumer().load(Ordering::Acquire)
    }

    /// The producer index as published by [`ProducerRing::submit`].
    pub(crate) fn head(&self) -> u32 {
        self.head
    }

    /// Whether the kernel set the flag that asks for a wake up.
    pub(crate) fn needs_wakeup(&self) -> bool {
        // SAFETY: the flag word lives in the mapping like the indices. Rings
        // without a flag word (in tests) point it at a zeroed word.
        let flags = unsafe { &*self.mem.flags }.load(Ordering::Relaxed);
        crate::sys::needs_wakeup(flags)
    }
}

/// The consumer end of a ring, for the completion and receive rings.
pub(crate) struct ConsumerRing<T> {
    mem: RingMem<T>,
    /// The next index to read, not yet released to the producer.
    head: u32,
    /// The producer index when we last read it.
    seen_producer: u32,
}

impl<T: Copy> ConsumerRing<T> {
    pub(crate) fn from_mapping(
        map: Mapping,
        producer: u64,
        consumer: u64,
        desc: u64,
        flags: u64,
        size: u32,
    ) -> Self {
        Self::new(RingMem::from_mapping(
            map, producer, consumer, desc, flags, size,
        ))
    }

    fn new(mem: RingMem<T>) -> Self {
        let head = mem.consumer().load(Ordering::Relaxed);
        let seen_producer = head;
        Self {
            mem,
            head,
            seen_producer,
        }
    }

    /// The number of entries, up to `want`, that the kernel has published and
    /// that this end has not read. It reads the producer index only when the
    /// cached value shows too few.
    pub(crate) fn available(&mut self, want: u32) -> u32 {
        let mut n = self.seen_producer.wrapping_sub(self.head);
        if n < want {
            self.seen_producer = self.mem.producer().load(Ordering::Acquire);
            n = self.seen_producer.wrapping_sub(self.head);
        }
        n.min(want)
    }

    /// Reads the entry at the head and moves on. It returns `None` when no
    /// entry was published as of the last [`ConsumerRing::available`]. The
    /// slot returns to the producer at [`ConsumerRing::release`].
    pub(crate) fn pop(&mut self) -> Option<T> {
        if self.seen_producer == self.head {
            return None;
        }
        // SAFETY: the index is masked into the array of `size` entries, and
        // the acquire load of the producer index in `available` made the
        // entry complete. The producer does not write it until we release it.
        let v = unsafe {
            self.mem
                .entries
                .add((self.head & self.mem.mask) as usize)
                .read()
        };
        self.head = self.head.wrapping_add(1);
        Some(v)
    }

    /// Returns every popped slot to the producer.
    pub(crate) fn release(&mut self) {
        self.mem.consumer().store(self.head, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Heap memory laid out like a ring, which stands in for the kernel.
    struct Fake {
        words: Box<[AtomicU32; 4]>,
        entries: Box<[u64]>,
    }

    impl Fake {
        fn new(size: u32) -> Self {
            Self {
                words: Box::default(),
                entries: vec![0u64; size as usize].into_boxed_slice(),
            }
        }

        fn mem(&mut self) -> RingMem<u64> {
            let w = self.words.as_ptr();
            RingMem {
                producer: w,
                // SAFETY: the array has four words.
                consumer: unsafe { w.add(1) },
                flags: unsafe { w.add(2) },
                entries: self.entries.as_mut_ptr(),
                mask: self.entries.len() as u32 - 1,
                size: self.entries.len() as u32,
                _map: None,
            }
        }
    }

    fn pair(fake: &mut Fake) -> (ProducerRing<u64>, ConsumerRing<u64>) {
        (ProducerRing::new(fake.mem()), ConsumerRing::new(fake.mem()))
    }

    #[test]
    fn entries_cross_in_order_and_wrap_the_array() {
        let mut fake = Fake::new(4);
        let (mut p, mut c) = pair(&mut fake);
        let mut next = 0u64;
        let mut seen = Vec::new();
        for _ in 0..10 {
            let room = p.free(3);
            for _ in 0..room {
                assert!(p.push(next));
                next += 1;
            }
            p.submit();
            let n = c.available(8);
            for _ in 0..n {
                seen.push(c.pop().unwrap());
            }
            c.release();
        }
        assert_eq!(seen, (0..next).collect::<Vec<_>>());
        assert!(next >= 20, "the ring wrapped several times, next={next}");
    }

    #[test]
    fn a_full_ring_refuses_until_the_consumer_releases() {
        let mut fake = Fake::new(4);
        let (mut p, mut c) = pair(&mut fake);
        assert_eq!(p.free(10), 4);
        for i in 0..4 {
            assert!(p.push(i));
        }
        assert!(!p.push(99), "no fifth entry in a ring of four");
        p.submit();
        assert_eq!(p.free(1), 0);
        assert_eq!(c.available(2), 2);
        assert_eq!(c.pop(), Some(0));
        assert_eq!(c.pop(), Some(1));
        assert_eq!(
            p.free(4),
            0,
            "popped slots stay with the consumer until release"
        );
        c.release();
        assert_eq!(p.free(4), 2);
        assert!(p.push(4));
        assert_eq!(p.head(), 5);
    }

    #[test]
    fn entries_are_invisible_until_submit() {
        let mut fake = Fake::new(8);
        let (mut p, mut c) = pair(&mut fake);
        assert_eq!(p.free(1), 1);
        assert!(p.push(7));
        assert_eq!(c.available(8), 0);
        assert_eq!(c.pop(), None);
        p.submit();
        assert_eq!(c.available(8), 1);
        assert_eq!(c.pop(), Some(7));
        assert_eq!(c.pop(), None);
    }

    #[test]
    fn indices_survive_wrapping_past_u32_max() {
        let mut fake = Fake::new(8);
        let start = u32::MAX - 2;
        fake.words[0].store(start, Ordering::Relaxed);
        fake.words[1].store(start, Ordering::Relaxed);
        let (mut p, mut c) = pair(&mut fake);
        assert_eq!(p.free(6), 6);
        for i in 0..6 {
            assert!(p.push(i));
        }
        p.submit();
        assert_eq!(c.available(6), 6);
        let got: Vec<u64> = (0..6).map(|_| c.pop().unwrap()).collect();
        assert_eq!(got, vec![0, 1, 2, 3, 4, 5]);
        c.release();
        assert_eq!(p.free(8), 8);
        assert_eq!(p.consumer_index(), start.wrapping_add(6));
    }

    #[test]
    fn the_wakeup_flag_is_read_from_the_flag_word() {
        let mut fake = Fake::new(4);
        let (p, _) = pair(&mut fake);
        assert!(!p.needs_wakeup());
        fake.words[2].store(1, Ordering::Relaxed);
        assert!(p.needs_wakeup());
    }

    #[test]
    fn two_threads_exchange_a_million_entries_in_order() {
        const N: u64 = 1_000_000;
        let mut fake = Fake::new(64);
        let (mut p, mut c) = pair(&mut fake);
        // SAFETY: both ends use the heap memory of `fake`, which outlives the
        // scope, and each end is used by one thread.
        std::thread::scope(|s| {
            s.spawn(move || {
                let mut next = 0u64;
                while next < N {
                    let room = p.free(16);
                    for _ in 0..room.min((N - next) as u32) {
                        assert!(p.push(next));
                        next += 1;
                    }
                    p.submit();
                }
            });
            let mut want = 0u64;
            while want < N {
                let n = c.available(16);
                for _ in 0..n {
                    assert_eq!(c.pop(), Some(want));
                    want += 1;
                }
                c.release();
            }
        });
    }
}
