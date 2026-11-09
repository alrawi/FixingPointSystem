// SPDX-License-Identifier: GPL-3.0-only
//! The shared cell through which a capture that starts in the middle of a run
//! reaches the workers.

use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, PoisonError, RwLock};

use fps_io::{DatagramIo, SessionIo};

use crate::{Capture, FlowEndpoints, Side, Tapped, TappedDatagram, Transport};

struct CellInner {
    on: AtomicBool,
    capture: RwLock<Option<Capture>>,
}

/// The capture that new sessions are tapped into.
///
/// A worker reads the cell once per session, when the session opens, with
/// [`CaptureCell::current`], [`CaptureCell::tap_stream`], or
/// [`CaptureCell::tap_datagram`]. While no capture runs that is one atomic
/// load, no lock, and no allocation. Cloning is cheap, and every clone shares
/// the cell, so the owner of a run hands a clone to its workers and keeps one
/// to [`CaptureCell::set`] a capture later. A session that is already open
/// keeps the recorder it began with, or none.
#[derive(Clone)]
pub struct CaptureCell {
    inner: Arc<CellInner>,
}

impl Default for CaptureCell {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for CaptureCell {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CaptureCell")
            .field("on", &self.inner.on.load(Ordering::Relaxed))
            .finish()
    }
}

impl CaptureCell {
    /// Returns an empty cell.
    pub fn new() -> Self {
        Self {
            inner: Arc::new(CellInner {
                on: AtomicBool::new(false),
                capture: RwLock::new(None),
            }),
        }
    }

    /// Returns the capture of the cell, or `None` when none runs.
    #[inline]
    pub fn current(&self) -> Option<Capture> {
        if !self.inner.on.load(Ordering::Acquire) {
            return None;
        }
        self.inner
            .capture
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Puts `capture` into the cell, so sessions that begin from now on are tapped.
    pub fn set(&self, capture: Capture) {
        *self
            .inner
            .capture
            .write()
            .unwrap_or_else(PoisonError::into_inner) = Some(capture);
        self.inner.on.store(true, Ordering::Release);
    }

    /// Empties the cell. Sessions that are open keep their recorder.
    pub fn clear(&self) {
        self.inner.on.store(false, Ordering::Release);
        *self
            .inner
            .capture
            .write()
            .unwrap_or_else(PoisonError::into_inner) = None;
    }

    /// Wraps a TCP session in the tap of the current capture, from the local
    /// and peer addresses of the tapped end.
    ///
    /// A session with no capture, one that the sampler does not pick, or one
    /// whose endpoints are of two address families is wrapped without a
    /// recorder.
    #[inline]
    pub fn tap_stream<IO: SessionIo>(
        &self,
        session_id: u64,
        side: Side,
        local: SocketAddr,
        peer: SocketAddr,
        io: IO,
    ) -> Tapped<IO> {
        let handle = self.current().and_then(|capture| {
            capture.begin(
                session_id,
                endpoints(side, local, peer)?,
                Transport::Tcp,
                side,
            )
        });
        Tapped::new(io, handle)
    }

    /// Wraps a UDP session in the tap of the current capture, with the same
    /// rules as [`CaptureCell::tap_stream`].
    pub fn tap_datagram<D: DatagramIo>(
        &self,
        session_id: u64,
        side: Side,
        local: SocketAddr,
        peer: SocketAddr,
        io: D,
    ) -> TappedDatagram<D> {
        let handle = self.current().and_then(|capture| {
            capture.begin(
                session_id,
                endpoints(side, local, peer)?,
                Transport::Udp,
                side,
            )
        });
        TappedDatagram::new(io, handle)
    }
}

/// Orders the two addresses of the tapped end into client and server.
fn endpoints(side: Side, local: SocketAddr, peer: SocketAddr) -> Option<FlowEndpoints> {
    let (client, server) = match side {
        Side::Client => (local, peer),
        Side::Server => (peer, local),
    };
    FlowEndpoints::new(client, server).ok()
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use fps_io::memory::duplex;
    use tokio::io::AsyncWriteExt;

    use super::*;
    use crate::{CaptureConfig, RingConfig, SamplePolicy, SystemClock};

    fn addr(a: &str) -> SocketAddr {
        a.parse().unwrap()
    }

    fn started(name: &str) -> Capture {
        let dir = std::env::temp_dir().join(format!("fps-cell-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut config = CaptureConfig::new(RingConfig::new(dir));
        config.sample = SamplePolicy::EveryNth(1);
        Capture::start(config, Arc::new(SystemClock)).unwrap()
    }

    #[test]
    fn an_empty_cell_gives_no_capture_and_a_plain_session() {
        let cell = CaptureCell::new();
        assert!(cell.current().is_none());
        let (a, _b) = duplex(64);
        let io = cell.tap_stream(1, Side::Client, addr("10.0.0.1:1"), addr("10.0.0.2:2"), a);
        assert!(io.handle().is_none());
    }

    #[tokio::test]
    async fn set_and_clear_reach_every_clone() {
        let cell = CaptureCell::new();
        let worker = cell.clone();
        let capture = started("set-clear");
        cell.set(capture.clone());
        assert!(worker.current().is_some());
        let (a, _b) = duplex(64);
        let mut io = worker.tap_stream(7, Side::Server, addr("10.0.0.2:2"), addr("10.0.0.1:1"), a);
        assert!(io.handle().is_some(), "sampled while the cell is set");
        io.write_all(b"x").await.unwrap();
        cell.clear();
        assert!(worker.current().is_none());
        let (a, _b) = duplex(64);
        let io2 = worker.tap_stream(8, Side::Server, addr("10.0.0.2:2"), addr("10.0.0.1:1"), a);
        assert!(io2.handle().is_none(), "plain after clear");
        // The session that began earlier keeps its recorder.
        assert!(io.handle().is_some());
        capture.stop();
    }

    #[test]
    fn endpoints_of_two_families_are_not_recorded() {
        let cell = CaptureCell::new();
        cell.set(started("mixed"));
        let (a, _b) = duplex(64);
        let io = cell.tap_stream(
            1,
            Side::Client,
            addr("10.0.0.1:1"),
            addr("[2001:db8::2]:2"),
            a,
        );
        assert!(io.handle().is_none());
        cell.current().unwrap().stop();
    }
}
