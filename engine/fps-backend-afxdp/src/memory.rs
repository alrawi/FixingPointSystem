// SPDX-License-Identifier: GPL-3.0-only
//! An in memory pair of frame queues, the fake that tests of code above
//! [`FrameIo`] use in place of a network device.

use std::collections::VecDeque;
use std::io;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::Duration;

use crate::frame::{FrameIo, QueueStats};

type Wire = Arc<(Mutex<VecDeque<Vec<u8>>>, Condvar)>;

/// One end of an in memory link. Frames that one end sends arrive at the
/// other end in order, and a full link makes sends return short, as a full
/// transmit ring does.
#[derive(Debug)]
pub struct MemoryFrames {
    queue: u32,
    max_len: usize,
    depth: usize,
    rx: Wire,
    tx: Wire,
    stats: QueueStats,
    scratch: Vec<u8>,
}

/// Makes two connected ends that hold at most `depth` frames per direction of
/// at most `max_frame_len` bytes each.
#[must_use]
pub fn frame_pair(depth: usize, max_frame_len: usize) -> (MemoryFrames, MemoryFrames) {
    let a_to_b: Wire = Arc::default();
    let b_to_a: Wire = Arc::default();
    let end = |rx: &Wire, tx: &Wire| MemoryFrames {
        queue: 0,
        max_len: max_frame_len,
        depth,
        rx: Arc::clone(rx),
        tx: Arc::clone(tx),
        stats: QueueStats::default(),
        scratch: vec![0; max_frame_len],
    };
    (end(&b_to_a, &a_to_b), end(&a_to_b, &b_to_a))
}

fn lock(wire: &Wire) -> MutexGuard<'_, VecDeque<Vec<u8>>> {
    // A poisoned lock only means that another test thread panicked, and the
    // queue itself stays consistent.
    wire.0
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl MemoryFrames {
    /// Sets the number that [`FrameIo::queue_id`] returns, so tests of a
    /// worker set tell their queues apart.
    #[must_use]
    pub fn with_queue_id(mut self, queue: u32) -> Self {
        self.queue = queue;
        self
    }
}

impl FrameIo for MemoryFrames {
    fn queue_id(&self) -> u32 {
        self.queue
    }

    fn max_frame_len(&self) -> usize {
        self.max_len
    }

    fn recv_batch(&mut self, max: usize, frame: &mut dyn FnMut(&[u8])) -> io::Result<usize> {
        let mut taken = Vec::new();
        {
            let mut q = lock(&self.rx);
            while taken.len() < max {
                match q.pop_front() {
                    Some(f) => taken.push(f),
                    None => break,
                }
            }
        }
        self.rx.1.notify_all();
        for f in &taken {
            frame(f);
            self.stats.rx_frames += 1;
            self.stats.rx_bytes += f.len() as u64;
        }
        Ok(taken.len())
    }

    fn send_batch_with(
        &mut self,
        count: usize,
        fill: &mut dyn FnMut(usize, &mut [u8]) -> usize,
    ) -> io::Result<usize> {
        let room = self.depth.saturating_sub(lock(&self.tx).len());
        let want = count.min(room);
        if want < count {
            self.stats.tx_full += 1;
        }
        let mut queued = 0;
        for i in 0..want {
            let len = fill(i, &mut self.scratch);
            let Some(bytes) = self.scratch.get(..len).filter(|b| !b.is_empty()) else {
                break;
            };
            lock(&self.tx).push_back(bytes.to_vec());
            self.stats.tx_frames += 1;
            self.stats.tx_bytes += len as u64;
            queued += 1;
        }
        self.tx.1.notify_all();
        Ok(queued)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }

    fn wait_readable(&mut self, timeout: Duration) -> io::Result<bool> {
        let guard = lock(&self.rx);
        if !guard.is_empty() {
            return Ok(true);
        }
        let (guard, _) = self
            .rx
            .1
            .wait_timeout_while(guard, timeout, |q| q.is_empty())
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        Ok(!guard.is_empty())
    }

    fn stats(&self) -> QueueStats {
        self.stats
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn drain(io: &mut MemoryFrames, max: usize) -> Vec<Vec<u8>> {
        let mut out = Vec::new();
        io.recv_batch(max, &mut |f| out.push(f.to_vec())).unwrap();
        out
    }

    #[test]
    fn frames_cross_in_order_and_batches_are_bounded() {
        let (mut a, mut b) = frame_pair(8, 64);
        let frames: Vec<Vec<u8>> = (1..=5u8).map(|i| vec![i; usize::from(i)]).collect();
        let refs: Vec<&[u8]> = frames.iter().map(Vec::as_slice).collect();
        assert_eq!(a.send_batch(&refs).unwrap(), 5);
        assert_eq!(drain(&mut b, 3), frames[..3]);
        assert_eq!(drain(&mut b, 3), frames[3..]);
        assert_eq!(drain(&mut b, 3), Vec::<Vec<u8>>::new());
        assert_eq!(a.stats().tx_frames, 5);
        assert_eq!(b.stats().rx_bytes, 15);
    }

    #[test]
    fn a_full_link_returns_short_until_the_peer_reads() {
        let (mut a, mut b) = frame_pair(2, 16);
        let f = [7u8; 4];
        assert_eq!(a.send_batch(&[&f, &f, &f]).unwrap(), 2);
        assert_eq!(a.stats().tx_full, 1);
        assert_eq!(drain(&mut b, 8).len(), 2);
        assert_eq!(a.send_batch(&[&f]).unwrap(), 1);
    }

    #[test]
    fn send_batch_rejects_bad_frames_before_queueing_any() {
        let (mut a, mut b) = frame_pair(4, 8);
        let ok = [1u8; 8];
        let long = [1u8; 9];
        let err = a.send_batch(&[&ok, &long]).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
        assert_eq!(
            a.send_batch(&[&ok, &[]]).unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
        assert!(drain(&mut b, 4).is_empty());
    }

    #[test]
    fn send_batch_with_fills_in_place_and_stops_at_a_zero_length() {
        let (mut a, mut b) = frame_pair(8, 8);
        let n = a
            .send_batch_with(4, &mut |i, buf| {
                if i == 2 {
                    return 0;
                }
                buf[0] = i as u8;
                1
            })
            .unwrap();
        assert_eq!(n, 2);
        assert_eq!(drain(&mut b, 8), vec![vec![0u8], vec![1u8]]);
    }

    #[test]
    fn wait_readable_wakes_for_a_frame_and_times_out_without_one() {
        let (mut a, mut b) = frame_pair(2, 8);
        assert!(!b.wait_readable(Duration::from_millis(5)).unwrap());
        let t = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(20));
            a.send_batch(&[&[9u8]]).unwrap();
            a
        });
        assert!(b.wait_readable(Duration::from_secs(5)).unwrap());
        t.join().unwrap();
    }

    #[test]
    fn queue_ids_come_from_the_builder() {
        let (a, _) = frame_pair(1, 1);
        assert_eq!(a.queue_id(), 0);
        assert_eq!(a.with_queue_id(3).queue_id(), 3);
    }
}
