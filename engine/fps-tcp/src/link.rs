// SPDX-License-Identifier: GPL-3.0-only
//! The in memory link between the two hosts of a bench pair.
//!
//! [`QueueDevice`] is a smoltcp device that keeps the frames it transmits in a
//! queue, and the pair driver moves them to the receive queue of the other
//! host. The link has no delay, no loss, and no reordering, so a measurement
//! shows the cost of the stack and of its `SessionIo` adapter and nothing
//! else. Buffers circulate through a small pool, so the link allocates only
//! while the pool fills.

use std::collections::VecDeque;

use smoltcp::phy::{
    Checksum, ChecksumCapabilities, Device, DeviceCapabilities, Medium, RxToken, TxToken,
};
use smoltcp::time::Instant;

/// The most spare buffers that a device keeps for reuse.
const SPARE_LIMIT: usize = 4096;

/// Settings of the link that both hosts of a pair share.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LinkConfig {
    /// The IP maximum transmission unit in bytes.
    pub mtu: usize,
    /// Whether the stack computes and verifies checksums in software.
    ///
    /// A NIC with checksum offload does this work in hardware, so `false`
    /// models such a NIC and `true` models a plain AF_XDP or DPDK port.
    pub checksums: bool,
}

impl Default for LinkConfig {
    fn default() -> Self {
        Self {
            mtu: 1500,
            checksums: true,
        }
    }
}

/// One end of the link, which smoltcp drives as an IP device.
#[derive(Debug)]
pub struct QueueDevice {
    config: LinkConfig,
    /// Frames that arrived and wait for the stack.
    rx: VecDeque<Vec<u8>>,
    /// Frames that the stack sent and the driver has not moved yet.
    tx: VecDeque<Vec<u8>>,
    /// Buffers ready for the next transmit.
    spare: Vec<Vec<u8>>,
    /// Buffers of consumed frames, which return to `spare` on the next call.
    done: Vec<Vec<u8>>,
    /// Frames the stack transmitted since the device was created.
    frames_sent: u64,
}

impl QueueDevice {
    /// Creates a device with empty queues.
    pub fn new(config: LinkConfig) -> Self {
        Self {
            config,
            rx: VecDeque::new(),
            tx: VecDeque::new(),
            spare: Vec::new(),
            done: Vec::new(),
            frames_sent: 0,
        }
    }

    /// Moves every frame that this device transmitted into the receive queue
    /// of `peer`, and returns how many frames moved.
    pub fn deliver_to(&mut self, peer: &mut QueueDevice) -> usize {
        let moved = self.tx.len();
        peer.rx.append(&mut self.tx);
        moved
    }

    /// Takes the oldest frame that this device transmitted, for a link that
    /// holds frames before it delivers them.
    pub fn take_tx(&mut self) -> Option<Vec<u8>> {
        self.tx.pop_front()
    }

    /// Queues a frame for the stack to receive.
    pub fn push_rx(&mut self, frame: Vec<u8>) {
        self.rx.push_back(frame);
    }

    /// Returns the number of frames that the stack transmitted so far.
    pub fn frames_sent(&self) -> u64 {
        self.frames_sent
    }

    /// Returns true when frames wait in the receive queue.
    pub fn has_pending_rx(&self) -> bool {
        !self.rx.is_empty()
    }

    fn recycle(&mut self) {
        while let Some(buf) = self.done.pop() {
            if self.spare.len() < SPARE_LIMIT {
                self.spare.push(buf);
            }
        }
    }
}

/// A frame taken from the receive queue.
pub struct QueueRx<'a> {
    buf: Vec<u8>,
    done: &'a mut Vec<Vec<u8>>,
}

impl RxToken for QueueRx<'_> {
    fn consume<R, F>(self, f: F) -> R
    where
        F: FnOnce(&[u8]) -> R,
    {
        let result = f(&self.buf);
        self.done.push(self.buf);
        result
    }
}

/// A permit to queue one frame for transmission.
pub struct QueueTx<'a> {
    tx: &'a mut VecDeque<Vec<u8>>,
    spare: &'a mut Vec<Vec<u8>>,
    frames_sent: &'a mut u64,
}

impl TxToken for QueueTx<'_> {
    fn consume<R, F>(self, len: usize, f: F) -> R
    where
        F: FnOnce(&mut [u8]) -> R,
    {
        let mut buf = self.spare.pop().unwrap_or_default();
        buf.clear();
        buf.resize(len, 0);
        let result = f(&mut buf);
        self.tx.push_back(buf);
        *self.frames_sent += 1;
        result
    }
}

impl Device for QueueDevice {
    type RxToken<'a> = QueueRx<'a>;
    type TxToken<'a> = QueueTx<'a>;

    fn receive(&mut self, _timestamp: Instant) -> Option<(Self::RxToken<'_>, Self::TxToken<'_>)> {
        self.recycle();
        let buf = self.rx.pop_front()?;
        Some((
            QueueRx {
                buf,
                done: &mut self.done,
            },
            QueueTx {
                tx: &mut self.tx,
                spare: &mut self.spare,
                frames_sent: &mut self.frames_sent,
            },
        ))
    }

    fn transmit(&mut self, _timestamp: Instant) -> Option<Self::TxToken<'_>> {
        self.recycle();
        Some(QueueTx {
            tx: &mut self.tx,
            spare: &mut self.spare,
            frames_sent: &mut self.frames_sent,
        })
    }

    fn capabilities(&self) -> DeviceCapabilities {
        let mut caps = DeviceCapabilities::default();
        caps.medium = Medium::Ip;
        caps.max_transmission_unit = self.config.mtu;
        caps.max_burst_size = None;
        let mut checksum = ChecksumCapabilities::default();
        if !self.config.checksums {
            checksum.ipv4 = Checksum::None;
            checksum.tcp = Checksum::None;
            checksum.udp = Checksum::None;
            checksum.icmpv4 = Checksum::None;
        }
        caps.checksum = checksum;
        caps
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_move_between_devices_in_order() {
        let mut a = QueueDevice::new(LinkConfig::default());
        let mut b = QueueDevice::new(LinkConfig::default());
        for byte in [1u8, 2, 3] {
            let tx = a.transmit(Instant::from_millis(0)).expect("transmit");
            tx.consume(1, |frame| frame[0] = byte);
        }
        assert_eq!(a.frames_sent(), 3);
        assert_eq!(a.deliver_to(&mut b), 3);
        let mut seen = Vec::new();
        while let Some((rx, _tx)) = b.receive(Instant::from_millis(0)) {
            rx.consume(|frame| seen.push(frame[0]));
        }
        assert_eq!(seen, vec![1, 2, 3]);
        assert!(!b.has_pending_rx());
    }

    #[test]
    fn consumed_buffers_are_reused() {
        let mut a = QueueDevice::new(LinkConfig::default());
        let mut b = QueueDevice::new(LinkConfig::default());
        let tx = a.transmit(Instant::from_millis(0)).expect("transmit");
        tx.consume(64, |frame| frame.fill(7));
        a.deliver_to(&mut b);
        let (rx, _tx) = b.receive(Instant::from_millis(0)).expect("frame");
        rx.consume(|frame| assert_eq!(frame.len(), 64));
        assert!(b.receive(Instant::from_millis(0)).is_none());
        assert_eq!(b.spare.len(), 1);
    }

    #[test]
    fn checksum_setting_reaches_the_capabilities() {
        let on = QueueDevice::new(LinkConfig::default()).capabilities();
        assert!(on.checksum.tcp.rx() && on.checksum.tcp.tx());
        let off = QueueDevice::new(LinkConfig {
            mtu: 9000,
            checksums: false,
        })
        .capabilities();
        assert!(!off.checksum.tcp.rx() && !off.checksum.tcp.tx());
        assert_eq!(off.max_transmission_unit, 9000);
    }
}
