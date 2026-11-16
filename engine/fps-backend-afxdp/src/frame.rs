// SPDX-License-Identifier: GPL-3.0-only
//! The frame trait that user space transports run over.

use std::io;
use std::time::Duration;

/// Counters of one queue, kept by the queue itself without atomics because a
/// queue belongs to one worker.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct QueueStats {
    /// Frames handed to the receive callback.
    pub rx_frames: u64,
    /// Bytes of those frames.
    pub rx_bytes: u64,
    /// Frames queued for transmission.
    pub tx_frames: u64,
    /// Bytes of those frames.
    pub tx_bytes: u64,
    /// Sends that found no free frame or no free transmit slot.
    pub tx_full: u64,
    /// Receive descriptors that pointed outside the UMEM, and frames that
    /// could not return to the receive pool. Both stay 0 while the kernel
    /// behaves.
    pub rx_invalid: u64,
    /// Calls that woke the kernel because a ring asked for it.
    pub kicks: u64,
}

/// One queue of a network port that exchanges whole Ethernet frames in batches.
///
/// A transport such as a user space TCP stack owns one value per queue and
/// calls it from one thread, so no method takes a lock. The AF_XDP queue of
/// this crate implements the trait over a UMEM, and [`crate::memory`] has an
/// in memory pair for tests of the code above it. A DPDK backend can
/// implement the same trait.
///
/// Receive and transmit lend frame memory instead of copying it. Both take a
/// callback that sees one frame at a time, so a batch costs no allocation.
pub trait FrameIo: Send {
    /// The queue number on the port.
    fn queue_id(&self) -> u32;

    /// The longest frame that this queue sends and receives, in bytes, without
    /// the frame check sequence.
    fn max_frame_len(&self) -> usize;

    /// Receives at most `max` frames and calls `frame` once for each in order.
    ///
    /// The slice is valid only inside the call, and the queue recycles the
    /// memory after the whole batch. The call never blocks and returns 0 when
    /// no frame waits. Use [`FrameIo::wait_readable`] to sleep until one
    /// arrives.
    ///
    /// # Errors
    ///
    /// Returns the error of the system call that failed.
    fn recv_batch(&mut self, max: usize, frame: &mut dyn FnMut(&[u8])) -> io::Result<usize>;

    /// Queues at most `count` frames, which `fill` writes in place.
    ///
    /// For each frame `fill` gets the index in the batch and a buffer of
    /// [`FrameIo::max_frame_len`] bytes, writes the frame at the start, and
    /// returns its length. A length of 0 or above the buffer ends the batch
    /// before that frame. The call returns the number of frames queued, which
    /// is lower than `count` when the queue has no free frame or slot, and
    /// then the caller retries after [`FrameIo::flush`]. It never blocks.
    ///
    /// # Errors
    ///
    /// Returns the error of the system call that failed.
    fn send_batch_with(
        &mut self,
        count: usize,
        fill: &mut dyn FnMut(usize, &mut [u8]) -> usize,
    ) -> io::Result<usize>;

    /// Queues the frames of `frames`, copying each into queue memory, and
    /// returns the number queued, with the meaning of
    /// [`FrameIo::send_batch_with`].
    ///
    /// # Errors
    ///
    /// Returns [`io::ErrorKind::InvalidInput`], before any frame is queued,
    /// when a frame is empty or longer than [`FrameIo::max_frame_len`].
    fn send_batch(&mut self, frames: &[&[u8]]) -> io::Result<usize> {
        let max = self.max_frame_len();
        if frames.iter().any(|f| f.is_empty() || f.len() > max) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("a frame is empty or longer than {max} bytes"),
            ));
        }
        self.send_batch_with(frames.len(), &mut |i, buf| match (
            frames.get(i),
            buf.get_mut(..frames.get(i).map_or(0, |f| f.len())),
        ) {
            (Some(src), Some(dst)) => {
                dst.copy_from_slice(src);
                src.len()
            }
            _ => 0,
        })
    }

    /// Hands queued frames to the kernel and takes back the memory of sent
    /// frames. Call it after a send that returned fewer frames than asked, and
    /// from a worker that has nothing to receive.
    ///
    /// # Errors
    ///
    /// Returns the error of the system call that failed.
    fn flush(&mut self) -> io::Result<()>;

    /// Waits until a frame can be received or `timeout` passes, and returns
    /// whether a frame waits.
    ///
    /// # Errors
    ///
    /// Returns the error of the system call that failed.
    fn wait_readable(&mut self, timeout: Duration) -> io::Result<bool>;

    /// The counters of this queue.
    fn stats(&self) -> QueueStats;
}

impl<T: FrameIo + ?Sized> FrameIo for Box<T> {
    fn queue_id(&self) -> u32 {
        (**self).queue_id()
    }
    fn max_frame_len(&self) -> usize {
        (**self).max_frame_len()
    }
    fn recv_batch(&mut self, max: usize, frame: &mut dyn FnMut(&[u8])) -> io::Result<usize> {
        (**self).recv_batch(max, frame)
    }
    fn send_batch_with(
        &mut self,
        count: usize,
        fill: &mut dyn FnMut(usize, &mut [u8]) -> usize,
    ) -> io::Result<usize> {
        (**self).send_batch_with(count, fill)
    }
    fn flush(&mut self) -> io::Result<()> {
        (**self).flush()
    }
    fn wait_readable(&mut self, timeout: Duration) -> io::Result<bool> {
        (**self).wait_readable(timeout)
    }
    fn stats(&self) -> QueueStats {
        (**self).stats()
    }
}
