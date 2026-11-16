// SPDX-License-Identifier: GPL-3.0-only
//! One AF_XDP queue: a socket with its own UMEM and four rings.

use std::io;
use std::sync::Arc;
use std::time::Duration;

use crate::config::{BindMode, BoundMode, QueueConfig};
use crate::error::AfXdpError;
use crate::frame::{FrameIo, QueueStats};
use crate::ring::{ConsumerRing, ProducerRing};
use crate::sys::{Attached, KernelStats, Mapping, XdpDesc, XskMap, XskSocket};

/// One queue of an AF_XDP port.
///
/// The queue owns a socket bound to one queue of the device, a UMEM of
/// `2 * ring_size` frames, and the four rings. The first half of the UMEM is
/// the receive pool, which starts in the fill ring and cycles through the
/// receive ring and back. The second half is the transmit pool, which cycles
/// through the transmit ring and the completion ring. Frames are never shared
/// between queues, so a worker needs no lock.
///
/// Open queues with [`crate::open_port`].
pub struct AfXdpQueue {
    queue_id: u32,
    ring_size: u32,
    frame_size: u32,
    frame_mask: u64,
    rx_frames: u32,
    max_len: usize,
    mode: BoundMode,
    fill: ProducerRing<u64>,
    completion: ConsumerRing<u64>,
    rx: ConsumerRing<XdpDesc>,
    tx: ProducerRing<XdpDesc>,
    /// Chunk addresses of transmit frames that no ring holds.
    tx_free: Vec<u64>,
    umem: Mapping,
    stats: QueueStats,
    sock: XskSocket,
    /// Keeps the XDP program attached while any queue lives.
    _attached: Option<Arc<Attached>>,
}

impl std::fmt::Debug for AfXdpQueue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AfXdpQueue")
            .field("queue_id", &self.queue_id)
            .field("mode", &self.mode)
            .field("ring_size", &self.ring_size)
            .field("frame_size", &self.frame_size)
            .finish_non_exhaustive()
    }
}

impl AfXdpQueue {
    /// Creates the socket, the UMEM, and the rings of one queue, binds it, and
    /// stores the socket in `map`.
    pub(crate) fn open(
        interface: &str,
        ifindex: u32,
        queue_id: u32,
        cfg: &QueueConfig,
        map: &XskMap,
    ) -> Result<Self, AfXdpError> {
        let fail = |step: &'static str| {
            move |source: io::Error| AfXdpError::Syscall {
                step,
                interface: interface.to_owned(),
                queue: Some(queue_id),
                source,
            }
        };
        let frames = cfg.ring_size as usize * 2;
        let umem_len = frames * cfg.frame_size as usize;
        let umem = Mapping::anonymous(umem_len).map_err(fail("map the UMEM"))?;
        let sock = XskSocket::new().map_err(fail("create the socket"))?;
        sock.register_umem(&umem, cfg.frame_size)
            .map_err(fail("XDP_UMEM_REG"))?;
        let rings = sock
            .map_rings(cfg.ring_size)
            .map_err(fail("map the rings"))?;
        let o = rings.offsets;
        let n = cfg.ring_size;
        let mut fill = ProducerRing::<u64>::from_mapping(
            rings.fill,
            o.fill.producer,
            o.fill.consumer,
            o.fill.desc,
            o.fill.flags,
            n,
        );
        let completion = ConsumerRing::<u64>::from_mapping(
            rings.completion,
            o.completion.producer,
            o.completion.consumer,
            o.completion.desc,
            o.completion.flags,
            n,
        );
        let rx = ConsumerRing::<XdpDesc>::from_mapping(
            rings.rx,
            o.rx.producer,
            o.rx.consumer,
            o.rx.desc,
            o.rx.flags,
            n,
        );
        let tx = ProducerRing::<XdpDesc>::from_mapping(
            rings.tx,
            o.tx.producer,
            o.tx.consumer,
            o.tx.desc,
            o.tx.flags,
            n,
        );
        // The whole receive pool goes to the kernel before the first frame can
        // arrive, so no frame meets an empty fill ring.
        let frame = u64::from(cfg.frame_size);
        if fill.free(n) < n {
            return Err(fail("fill the fill ring")(io::Error::other(
                "the fill ring is smaller than requested",
            )));
        }
        for i in 0..u64::from(n) {
            fill.push(i * frame);
        }
        fill.submit();
        let tx_free: Vec<u64> = (u64::from(n)..u64::from(n) * 2)
            .map(|i| i * frame)
            .collect();
        sock.bind(ifindex, queue_id, cfg.mode)
            .map_err(fail("bind"))?;
        let mode = sock.bound_mode().map_err(fail("read XDP_OPTIONS"))?;
        map.insert(queue_id, &sock)
            .map_err(fail("store the socket in the XSK map"))?;
        Ok(Self {
            queue_id,
            ring_size: n,
            frame_size: cfg.frame_size,
            frame_mask: frame - 1,
            rx_frames: n,
            max_len: cfg.max_frame_len(),
            mode,
            fill,
            completion,
            rx,
            tx,
            tx_free,
            umem,
            stats: QueueStats::default(),
            sock,
            _attached: None,
        })
    }

    pub(crate) fn keep_attached(&mut self, attached: Arc<Attached>) {
        self._attached = Some(attached);
    }

    /// The bind mode that the socket reports now, read with `XDP_OPTIONS`.
    ///
    /// # Errors
    ///
    /// Returns the error of the `getsockopt` call.
    pub fn bind_mode(&self) -> io::Result<BoundMode> {
        self.sock.bound_mode()
    }

    /// The counters that the kernel keeps for this socket, such as frames it
    /// dropped because the receive ring was full.
    ///
    /// # Errors
    ///
    /// Returns the error of the `getsockopt` call.
    pub fn kernel_stats(&self) -> io::Result<KernelStats> {
        self.sock.kernel_stats()
    }

    /// The number of transmit frames that are free now.
    #[must_use]
    pub fn tx_frames_free(&self) -> usize {
        self.tx_free.len()
    }

    /// Moves the addresses of sent frames from the completion ring back to the
    /// free list.
    fn reap(&mut self) {
        let room = (self.ring_size as usize).saturating_sub(self.tx_free.len());
        let want = u32::try_from(room).unwrap_or(u32::MAX);
        let n = self.completion.available(want);
        for _ in 0..n {
            let Some(addr) = self.completion.pop() else {
                break;
            };
            // The kernel returns the address that we put in the descriptor,
            // which is a chunk base, but a kernel bug must not make the free
            // list outgrow the pool, so only addresses of the transmit pool
            // return.
            let chunk = addr & !self.frame_mask;
            if chunk >= u64::from(self.rx_frames) * u64::from(self.frame_size)
                && self.tx_free.len() < self.ring_size as usize
            {
                self.tx_free.push(chunk);
            }
        }
        if n > 0 {
            self.completion.release();
        }
    }

    /// Wakes the kernel to receive when the fill ring asks for it. A busy or
    /// stopped device is not an error, because the kernel then has nothing to
    /// receive into and the next call retries.
    fn wake_fill_if_asked(&mut self) -> io::Result<()> {
        if !self.fill.needs_wakeup() {
            return Ok(());
        }
        self.stats.kicks += 1;
        match self.sock.kick_rx() {
            Ok(()) => Ok(()),
            Err(e)
                if matches!(
                    e.raw_os_error(),
                    Some(libc::EAGAIN | libc::EBUSY | libc::ENOBUFS | libc::ENETDOWN)
                ) =>
            {
                Ok(())
            }
            Err(e) => Err(e),
        }
    }

    /// Wakes the kernel to send what the transmit ring holds. In copy mode the
    /// kernel sends inside this call, in at most 32 frame rounds, so the call
    /// repeats until the ring is empty. In zero copy mode the call is only
    /// needed when the ring asks for it.
    fn kick_tx(&mut self) -> io::Result<()> {
        if self.mode == BoundMode::ZeroCopy && !self.tx.needs_wakeup() {
            return Ok(());
        }
        let rounds = self.ring_size / 32 + 2;
        for _ in 0..rounds {
            self.stats.kicks += 1;
            match self.sock.kick_tx() {
                Ok(()) => {
                    if self.tx.head() == self.tx.consumer_index() {
                        break;
                    }
                }
                Err(e) => match e.raw_os_error() {
                    Some(libc::EAGAIN) => {}
                    // The device is busy or down, or out of buffers. The frames
                    // stay in the ring and the next flush retries.
                    Some(libc::ENOBUFS | libc::EBUSY | libc::ENETDOWN) => break,
                    _ => return Err(e),
                },
            }
        }
        Ok(())
    }
}

impl FrameIo for AfXdpQueue {
    fn queue_id(&self) -> u32 {
        self.queue_id
    }

    fn max_frame_len(&self) -> usize {
        self.max_len
    }

    fn recv_batch(&mut self, max: usize, frame: &mut dyn FnMut(&[u8])) -> io::Result<usize> {
        let want = u32::try_from(max).unwrap_or(u32::MAX).min(self.ring_size);
        let n = self.rx.available(want);
        if n == 0 {
            self.wake_fill_if_asked()?;
            return Ok(0);
        }
        // The queue owns `ring_size` receive frames, so the fill ring has room
        // for every frame of this batch once the kernel has taken the earlier
        // entries. This call refreshes the cached consumer index that `push`
        // checks against.
        self.fill.free(n);
        let mut got = 0;
        for _ in 0..n {
            let Some(d) = self.rx.pop() else {
                break;
            };
            got += 1;
            match self.umem.slice(d.addr, d.len as usize) {
                Some(bytes) => {
                    frame(bytes);
                    self.stats.rx_frames += 1;
                    self.stats.rx_bytes += u64::from(d.len);
                }
                None => self.stats.rx_invalid += 1,
            }
            // The frame goes back to the fill ring whether or not its
            // descriptor was valid, because the chunk belongs to the pool.
            let chunk = d.addr & !self.frame_mask;
            if chunk >= u64::from(self.rx_frames) * u64::from(self.frame_size)
                || !self.fill.push(chunk)
            {
                // The frame cannot return to the pool. This does not happen
                // while the kernel behaves, and the count makes it visible.
                self.stats.rx_invalid += 1;
            }
        }
        self.fill.submit();
        self.rx.release();
        self.wake_fill_if_asked()?;
        Ok(got)
    }

    fn send_batch_with(
        &mut self,
        count: usize,
        fill: &mut dyn FnMut(usize, &mut [u8]) -> usize,
    ) -> io::Result<usize> {
        if count == 0 {
            return Ok(0);
        }
        self.reap();
        let want = u32::try_from(count).unwrap_or(u32::MAX);
        let slots = self.tx.free(want) as usize;
        let n = count.min(self.tx_free.len()).min(slots);
        if n < count {
            self.stats.tx_full += 1;
        }
        let mut queued = 0usize;
        for i in 0..n {
            let Some(&addr) = self.tx_free.last() else {
                break;
            };
            let Some(buf) = self.umem.slice_mut(addr, self.max_len) else {
                break;
            };
            let len = fill(i, buf);
            if len == 0 || len > self.max_len {
                break;
            }
            self.tx_free.pop();
            // `len` is at most `max_len`, which is below the frame size.
            if !self.tx.push(XdpDesc {
                addr,
                len: len as u32,
                options: 0,
            }) {
                self.tx_free.push(addr);
                break;
            }
            self.stats.tx_frames += 1;
            self.stats.tx_bytes += len as u64;
            queued += 1;
        }
        if queued > 0 {
            self.tx.submit();
            self.kick_tx()?;
        } else if n < count {
            // Nothing could go out because the pools are empty, so make the
            // kernel send what it holds.
            self.kick_tx()?;
        }
        Ok(queued)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.reap();
        if self.tx.head() != self.tx.consumer_index() {
            self.kick_tx()?;
            self.reap();
        }
        Ok(())
    }

    fn wait_readable(&mut self, timeout: Duration) -> io::Result<bool> {
        if self.rx.available(1) > 0 {
            return Ok(true);
        }
        let ms = if timeout.is_zero() {
            0
        } else {
            i32::try_from(timeout.as_millis().max(1)).unwrap_or(i32::MAX)
        };
        self.sock.poll_readable(ms)
    }

    fn stats(&self) -> QueueStats {
        self.stats
    }
}

/// The bind mode that a queue reports, with the mode that was requested.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct QueueInfo {
    /// The queue number on the device.
    pub queue_id: u32,
    /// The mode that the configuration asked for.
    pub requested: BindMode,
    /// The mode that the socket reported after the bind.
    pub bound: BoundMode,
}
