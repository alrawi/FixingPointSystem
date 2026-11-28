// SPDX-License-Identifier: GPL-3.0-only
//! A frame device for smoltcp that the flow table fills with one frame and
//! drains after each call. No interface loop owns it.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

use smoltcp::phy::{Device, DeviceCapabilities, Medium, RxToken, TxToken};
use smoltcp::time::Instant;

/// Recycled frame buffers, shared by the devices and the wire of one thread.
pub type Pool = Rc<RefCell<Vec<Vec<u8>>>>;

/// The MTU of the device. The packets are IPv4 packets, as the `medium-ip`
/// medium wants them.
pub const MTU: usize = 1500;

/// The device of one host.
pub struct FrameDev {
    /// Frames that smoltcp has not read yet.
    pub rx: VecDeque<Vec<u8>>,
    /// Frames that smoltcp wrote and that nobody collected yet.
    pub out: Vec<Vec<u8>>,
    pool: Pool,
}

impl FrameDev {
    /// Makes a device that takes its buffers from `pool`.
    pub fn new(pool: Pool) -> Self {
        Self {
            rx: VecDeque::new(),
            out: Vec::new(),
            pool,
        }
    }

    /// A buffer from the pool, or a new one.
    pub fn buffer(&self) -> Vec<u8> {
        self.pool.borrow_mut().pop().unwrap_or_default()
    }

    /// Gives a buffer back.
    pub fn recycle(&self, mut buf: Vec<u8>) {
        buf.clear();
        self.pool.borrow_mut().push(buf);
    }
}

/// Reads one frame.
pub struct Rx<'a> {
    buf: Vec<u8>,
    pool: &'a RefCell<Vec<Vec<u8>>>,
}

/// Writes one frame to the output queue.
pub struct Tx<'a> {
    out: &'a mut Vec<Vec<u8>>,
    pool: &'a RefCell<Vec<Vec<u8>>>,
}

impl RxToken for Rx<'_> {
    fn consume<R, F: FnOnce(&[u8]) -> R>(mut self, f: F) -> R {
        let r = f(&self.buf);
        self.buf.clear();
        self.pool.borrow_mut().push(self.buf);
        r
    }
}

impl TxToken for Tx<'_> {
    fn consume<R, F: FnOnce(&mut [u8]) -> R>(self, len: usize, f: F) -> R {
        let mut buf = self.pool.borrow_mut().pop().unwrap_or_default();
        buf.resize(len, 0);
        let r = f(&mut buf);
        self.out.push(buf);
        r
    }
}

impl Device for FrameDev {
    type RxToken<'a> = Rx<'a>;
    type TxToken<'a> = Tx<'a>;

    fn receive(&mut self, _timestamp: Instant) -> Option<(Rx<'_>, Tx<'_>)> {
        let buf = self.rx.pop_front()?;
        Some((
            Rx {
                buf,
                pool: &self.pool,
            },
            Tx {
                out: &mut self.out,
                pool: &self.pool,
            },
        ))
    }

    fn transmit(&mut self, _timestamp: Instant) -> Option<Tx<'_>> {
        Some(Tx {
            out: &mut self.out,
            pool: &self.pool,
        })
    }

    fn capabilities(&self) -> DeviceCapabilities {
        let mut caps = DeviceCapabilities::default();
        caps.medium = Medium::Ip;
        caps.max_transmission_unit = MTU;
        caps
    }
}
