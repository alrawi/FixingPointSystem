// SPDX-License-Identifier: GPL-3.0-only
//! The neighbor service of one queue: it reads a batch of frames from a
//! [`FrameIo`], answers the ARP requests and neighbor solicitations for the
//! hosts of the port, and hands every other frame to the caller.
//!
//! One service belongs to one queue and one thread, as [`FrameIo`] asks. The
//! service allocates its reply slots once in [`NeighborService::new`] and
//! allocates nothing per frame after that. The replies of a batch go out in one
//! send call, and a full transmit queue is retried after a flush a few times
//! before the remaining replies are counted as dropped. A neighbor reply that
//! a busy device under test asks for again is cheaper to lose than a stalled
//! worker.

use std::io;

use fps_backend_afxdp::FrameIo;

use crate::neighbor::{Protocol, Responder, Verdict};
use crate::wire::MAX_REPLY_LEN;

/// How often a send that makes no progress is retried after a flush before the
/// rest of the batch is dropped.
const SEND_RETRIES: u32 = 8;

/// Counters of one [`NeighborService`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NeighborStats {
    /// Frames received.
    pub rx_frames: u64,
    /// ARP requests answered.
    pub arp_replies: u64,
    /// Neighbor advertisements sent.
    pub nd_replies: u64,
    /// Requests for addresses that no host of the table owns, and announcements.
    pub not_for_us: u64,
    /// Frames that broke a rule of ARP or neighbor discovery.
    pub malformed: u64,
    /// Frames handed to the caller.
    pub passed: u64,
    /// Replies that the transmit queue had no room for.
    pub tx_dropped: u64,
}

/// Answers ARP and neighbor discovery on one queue.
#[derive(Debug)]
pub struct NeighborService<'r> {
    responder: &'r Responder,
    batch: usize,
    slots: Vec<u8>,
    lens: Vec<usize>,
    stats: NeighborStats,
}

impl<'r> NeighborService<'r> {
    /// Returns a service that receives at most `batch` frames per call of
    /// [`NeighborService::poll`]. A `batch` of 0 becomes 1.
    pub fn new(responder: &'r Responder, batch: usize) -> Self {
        let batch = batch.max(1);
        Self {
            responder,
            batch,
            slots: vec![0; batch * MAX_REPLY_LEN],
            lens: Vec::with_capacity(batch),
            stats: NeighborStats::default(),
        }
    }

    /// The counters since the service was made.
    pub fn stats(&self) -> NeighborStats {
        self.stats
    }

    /// Receives one batch, sends the replies, and calls `pass` for each frame
    /// that is not a neighbor protocol request. It returns the number of
    /// frames received and never blocks.
    ///
    /// # Errors
    ///
    /// Returns the error of the queue, and returns
    /// [`io::ErrorKind::InvalidInput`] when the queue cannot carry a reply of
    /// [`MAX_REPLY_LEN`] bytes.
    pub fn poll<F: FrameIo + ?Sized>(
        &mut self,
        io: &mut F,
        pass: &mut dyn FnMut(&[u8]),
    ) -> io::Result<usize> {
        if io.max_frame_len() < MAX_REPLY_LEN {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "queue {} carries frames of {} bytes, a reply needs {MAX_REPLY_LEN}",
                    io.queue_id(),
                    io.max_frame_len()
                ),
            ));
        }
        self.lens.clear();
        let Self {
            responder,
            slots,
            lens,
            stats,
            batch,
        } = self;
        let received = io.recv_batch(*batch, &mut |frame| {
            stats.rx_frames += 1;
            let at = lens.len() * MAX_REPLY_LEN;
            let Some(slot) = slots.get_mut(at..at + MAX_REPLY_LEN) else {
                // More frames than the batch, which a queue must not deliver.
                stats.passed += 1;
                pass(frame);
                return;
            };
            match responder.process(frame, slot) {
                Verdict::Reply(proto, len) => {
                    match proto {
                        Protocol::Arp => stats.arp_replies += 1,
                        Protocol::Nd => stats.nd_replies += 1,
                    }
                    lens.push(len);
                }
                Verdict::Pass => {
                    stats.passed += 1;
                    pass(frame);
                }
                Verdict::NotForUs(_) => stats.not_for_us += 1,
                Verdict::Malformed(_) => stats.malformed += 1,
            }
        })?;
        self.send_replies(io)?;
        Ok(received)
    }

    fn send_replies<F: FrameIo + ?Sized>(&mut self, io: &mut F) -> io::Result<()> {
        let total = self.lens.len();
        let mut sent = 0;
        let mut stalled = 0;
        while sent < total {
            let (slots, lens) = (&self.slots, &self.lens);
            let first = sent;
            let queued = io.send_batch_with(total - sent, &mut |i, buf| {
                let n = first + i;
                let len = lens.get(n).copied().unwrap_or(0);
                let src = slots
                    .get(n * MAX_REPLY_LEN..n * MAX_REPLY_LEN + len)
                    .unwrap_or(&[]);
                match buf.get_mut(..src.len()) {
                    Some(dst) => {
                        dst.copy_from_slice(src);
                        src.len()
                    }
                    None => 0,
                }
            })?;
            sent += queued;
            if queued == 0 {
                stalled += 1;
                io.flush()?;
                if stalled > SEND_RETRIES {
                    self.stats.tx_dropped += (total - sent) as u64;
                    break;
                }
            } else {
                stalled = 0;
            }
        }
        io.flush()
    }
}
