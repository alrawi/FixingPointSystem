// SPDX-License-Identifier: GPL-3.0-only
//! Session sampling and capture of the engine.
//!
//! A [`Capture`] mirrors a seeded sample of sessions into a bounded ring of
//! pcapng files, with the TLS key log of each session next to its packets.
//!
//! The engine sees the bytes of a session and no packets, so the capture
//! records the bytes that pass through a [`Tapped`] or [`TappedDatagram`]
//! wrapper and then synthesizes the frames, see the `frames` module: an
//! Ethernet header, IPv4 or IPv6, and TCP with a handshake, sequence numbers,
//! segmentation, and a FIN or RST close, or UDP with one datagram per send. The
//! synthesis and the file writing run on a writer thread, so a sampled session
//! costs its worker one copy of its bytes, and a session that the sampler did
//! not pick costs one branch.
//!
//! ```no_run
//! use std::sync::Arc;
//! use fps_capture::{Capture, CaptureConfig, RingConfig, SamplePolicy, SystemClock};
//!
//! let mut config = CaptureConfig::new(RingConfig::new("/var/lib/fps/captures"));
//! config.sample = SamplePolicy::Rate(0.01);
//! config.seed = 42;
//! let capture = Capture::start(config, Arc::new(SystemClock))?;
//! // Per session: `let io = capture.tap(session_id, flow, Side::Client, io);`
//! let stats = capture.stop();
//! println!("{} sessions captured", stats.sessions_written);
//! # Ok::<(), fps_capture::CaptureError>(())
//! ```
#![forbid(unsafe_code)]
#![warn(missing_docs, clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod cell;
mod frames;
pub mod pcapng;
mod ring;
mod sampler;
mod tap;

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{sync_channel, SyncSender, TrySendError};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{SystemTime, UNIX_EPOCH};

use fps_io::{DatagramIo, SessionIo};

pub use cell::CaptureCell;
use frames::{Event, EventKind, Params};
pub use frames::{FlowEndpoints, Side, Transport};
pub use ring::{KeyLogMode, RingConfig};
pub use sampler::{SamplePolicy, Sampler};
pub use tap::{Tapped, TappedDatagram};

/// Errors that a capture reports.
#[derive(Debug, thiserror::Error)]
pub enum CaptureError {
    /// The configuration is invalid.
    #[error("capture config: {0}")]
    InvalidConfig(String),
    /// A file operation of the ring failed.
    #[error("capture ring: {what}")]
    Io {
        /// The operation and the path.
        what: String,
        /// The cause.
        #[source]
        source: std::io::Error,
    },
}

/// The time source of the capture, in nanoseconds since the Unix epoch.
///
/// Only the edge of the process uses [`SystemClock`]. Tests inject their own.
pub trait CaptureClock: Send + Sync {
    /// Returns the current time.
    fn now_nanos(&self) -> u64;
}

/// The wall clock.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl CaptureClock for SystemClock {
    fn now_nanos(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| u64::try_from(d.as_nanos()).unwrap_or(u64::MAX))
    }
}

/// The settings of a capture.
#[derive(Debug, Clone)]
pub struct CaptureConfig {
    /// Which sessions to capture.
    pub sample: SamplePolicy,
    /// The run seed, which makes the sample and the synthesized sequence numbers repeat.
    pub seed: u64,
    /// The ring on disk.
    pub ring: RingConfig,
    /// Where the TLS key logs go.
    pub key_log: KeyLogMode,
    /// The link MTU of the synthesized frames, from 1280 to 9216. TCP segments
    /// fit it, and a UDP datagram is never fragmented.
    pub mtu: usize,
    /// The most payload bytes that one session records in each capture. The
    /// rest of the session is left out and the session counts as truncated.
    pub max_session_bytes: usize,
    /// The number of finished sessions that wait for the writer. A session that
    /// finds the queue full is dropped and counted in `sessions_dropped_overflow`.
    pub queue_depth: usize,
}

impl CaptureConfig {
    /// A configuration with sampling off, an MTU of 1500, sessions of at most
    /// 1 MiB, and a queue of 256 sessions.
    pub fn new(ring: RingConfig) -> Self {
        Self {
            sample: SamplePolicy::Off,
            seed: 1,
            ring,
            key_log: KeyLogMode::default(),
            mtu: 1500,
            max_session_bytes: 1 << 20,
            queue_depth: 256,
        }
    }

    fn validate(&self) -> Result<(), CaptureError> {
        self.ring.validate()?;
        if !(1280..=9216).contains(&self.mtu) {
            return Err(CaptureError::InvalidConfig(format!(
                "mtu {} is outside 1280 to 9216",
                self.mtu
            )));
        }
        if self.queue_depth == 0 || self.max_session_bytes == 0 {
            return Err(CaptureError::InvalidConfig(
                "queue depth and session size must be positive".into(),
            ));
        }
        Ok(())
    }
}

/// Counters of a capture, see [`Capture::stats`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CaptureStats {
    /// Sessions that the sampler picked.
    pub sessions_sampled: u64,
    /// Sessions that reached a ring file.
    pub sessions_written: u64,
    /// Sessions that never reached the writer: the sum of
    /// [`sessions_dropped_overflow`](Self::sessions_dropped_overflow) and
    /// [`sessions_cut_at_stop`](Self::sessions_cut_at_stop). It keeps the value
    /// that it had before the two were counted apart, so a caller that adds it
    /// to `sessions_dropped_oversize` gets what it got before. Read the two
    /// parts to tell a capture that was too slow from a capture that was
    /// stopped while sessions ran.
    pub sessions_dropped_queue: u64,
    /// Sessions that ended while the writer queue was full. This is the loss of
    /// a capture whose writer could not keep up, and it happens during a run.
    pub sessions_dropped_overflow: u64,
    /// Sessions that were still open when [`Capture::stop`] ran, or that
    /// finished after the writer had ended for another reason, and so found
    /// no writer. They are no loss of the writer: the stop cut them off.
    pub sessions_cut_at_stop: u64,
    /// Sessions dropped because they could not fit into one ring file.
    pub sessions_dropped_oversize: u64,
    /// Written sessions that were cut at `max_session_bytes`.
    pub sessions_truncated: u64,
    /// Packets written.
    pub packets: u64,
    /// Bytes of the packets written, without the pcapng framing.
    pub bytes: u64,
    /// Ring files opened.
    pub files_opened: u64,
    /// Ring files removed to keep the file count.
    pub files_evicted: u64,
    /// Failed writes, whose first message `Capture::stop` could not return.
    pub write_errors: u64,
}

#[derive(Default)]
struct Counters {
    sampled: AtomicU64,
    written: AtomicU64,
    dropped_overflow: AtomicU64,
    cut_at_stop: AtomicU64,
    dropped_oversize: AtomicU64,
    truncated: AtomicU64,
    packets: AtomicU64,
    bytes: AtomicU64,
    opened: AtomicU64,
    evicted: AtomicU64,
    write_errors: AtomicU64,
}

impl Counters {
    fn snapshot(&self) -> CaptureStats {
        let get = |c: &AtomicU64| c.load(Ordering::Relaxed);
        let overflow = get(&self.dropped_overflow);
        let cut = get(&self.cut_at_stop);
        CaptureStats {
            sessions_sampled: get(&self.sampled),
            sessions_written: get(&self.written),
            sessions_dropped_queue: overflow + cut,
            sessions_dropped_overflow: overflow,
            sessions_cut_at_stop: cut,
            sessions_dropped_oversize: get(&self.dropped_oversize),
            sessions_truncated: get(&self.truncated),
            packets: get(&self.packets),
            bytes: get(&self.bytes),
            files_opened: get(&self.opened),
            files_evicted: get(&self.evicted),
            write_errors: get(&self.write_errors),
        }
    }
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// One finished session on its way to the writer.
struct Record {
    session_id: u64,
    flow: FlowEndpoints,
    transport: Transport,
    side: Side,
    open_nanos: u64,
    end_nanos: u64,
    events: Vec<Event>,
    keys: String,
    truncated: bool,
}

enum Job {
    Session(Box<Record>),
    Sync(SyncSender<Vec<PathBuf>>),
    Stop,
}

struct Inner {
    sampler: Sampler,
    seed: u64,
    max_session_bytes: usize,
    clock: Arc<dyn CaptureClock>,
    tx: SyncSender<Job>,
    counters: Arc<Counters>,
    first_error: Arc<Mutex<Option<String>>>,
    worker: Mutex<Option<JoinHandle<()>>>,
}

/// A running capture, or a disabled one that does nothing.
///
/// Cloning is cheap, and every clone feeds the same ring.
#[derive(Clone)]
pub struct Capture {
    inner: Option<Arc<Inner>>,
}

impl std::fmt::Debug for Capture {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Capture")
            .field("enabled", &self.inner.is_some())
            .finish()
    }
}

impl Capture {
    /// A capture that never samples. It starts no thread and touches no file.
    pub fn disabled() -> Self {
        Self { inner: None }
    }

    /// Starts a capture.
    ///
    /// A configuration whose sampling is off gives [`Capture::disabled`] and
    /// creates nothing, so a run without capture pays for no thread and no
    /// directory. Otherwise this opens the ring directory, adopts the files of
    /// an earlier run with the same prefix, and starts the writer thread.
    pub fn start(
        config: CaptureConfig,
        clock: Arc<dyn CaptureClock>,
    ) -> Result<Self, CaptureError> {
        config.validate()?;
        let sampler = Sampler::new(config.sample, config.seed)?;
        if sampler.is_off() {
            return Ok(Self::disabled());
        }
        let ring = ring::Ring::open(config.ring.clone(), config.key_log)?;
        let (tx, rx) = sync_channel(config.queue_depth);
        let counters = Arc::new(Counters::default());
        let first_error = Arc::new(Mutex::new(None));
        let writer = Writer {
            ring,
            mtu: config.mtu,
            seed: config.seed,
            counters: Arc::clone(&counters),
            first_error: Arc::clone(&first_error),
        };
        let worker = std::thread::Builder::new()
            .name("fps-capture".into())
            .spawn(move || writer.run(&rx))
            .map_err(|e| CaptureError::Io {
                what: "start the writer thread".into(),
                source: e,
            })?;
        Ok(Self {
            inner: Some(Arc::new(Inner {
                sampler,
                seed: config.seed,
                max_session_bytes: config.max_session_bytes,
                clock,
                tx,
                counters,
                first_error,
                worker: Mutex::new(Some(worker)),
            })),
        })
    }

    /// Returns true when the capture can sample sessions.
    pub fn is_enabled(&self) -> bool {
        self.inner.is_some()
    }

    /// Decides whether the session is sampled and returns its recorder when it is.
    ///
    /// This is the only call on the path of a session that is not sampled, and
    /// it does no allocation, takes no lock, and reads no clock.
    #[inline]
    pub fn begin(
        &self,
        session_id: u64,
        flow: FlowEndpoints,
        transport: Transport,
        side: Side,
    ) -> Option<SessionHandle> {
        let inner = self.inner.as_ref()?;
        if !inner.sampler.sample(session_id) {
            return None;
        }
        Some(inner.start_session(session_id, flow, transport, side))
    }

    /// Wraps a TCP session stream, recording it when the sampler picks it.
    pub fn tap<IO: SessionIo>(
        &self,
        session_id: u64,
        flow: FlowEndpoints,
        side: Side,
        io: IO,
    ) -> Tapped<IO> {
        Tapped::new(io, self.begin(session_id, flow, Transport::Tcp, side))
    }

    /// Wraps a UDP session, recording it when the sampler picks it.
    pub fn tap_datagram<D: DatagramIo>(
        &self,
        session_id: u64,
        flow: FlowEndpoints,
        side: Side,
        io: D,
    ) -> TappedDatagram<D> {
        TappedDatagram::new(io, self.begin(session_id, flow, Transport::Udp, side))
    }

    /// Returns the counters now. Sessions that are still on their way to the
    /// writer do not count yet, see [`Capture::flush`].
    pub fn stats(&self) -> CaptureStats {
        self.inner
            .as_ref()
            .map(|i| i.counters.snapshot())
            .unwrap_or_default()
    }

    /// Waits until the writer has written every session that finished before
    /// this call, and returns the ring files, oldest first.
    pub fn flush(&self) -> Vec<PathBuf> {
        let Some(inner) = &self.inner else {
            return Vec::new();
        };
        let (tx, rx) = sync_channel(1);
        if inner.tx.send(Job::Sync(tx)).is_err() {
            return Vec::new();
        }
        rx.recv().unwrap_or_default()
    }

    /// Returns the message of the first write error, if any happened.
    pub fn first_error(&self) -> Option<String> {
        self.inner
            .as_ref()
            .and_then(|i| lock(&i.first_error).clone())
    }

    /// Writes what is queued, closes the ring files, stops the writer, and
    /// returns the final counters. A session that is still open finds no
    /// writer when it ends, and counts in `sessions_cut_at_stop`, which the
    /// counters returned here cannot hold yet: read [`Capture::stats`] once the
    /// sessions have ended. Stopping twice is harmless.
    pub fn stop(&self) -> CaptureStats {
        let Some(inner) = &self.inner else {
            return CaptureStats::default();
        };
        let worker = lock(&inner.worker).take();
        if let Some(worker) = worker {
            let _ = inner.tx.send(Job::Stop);
            let _ = worker.join();
        }
        inner.counters.snapshot()
    }
}

impl Inner {
    fn start_session(
        self: &Arc<Self>,
        session_id: u64,
        flow: FlowEndpoints,
        transport: Transport,
        side: Side,
    ) -> SessionHandle {
        self.counters.sampled.fetch_add(1, Ordering::Relaxed);
        SessionHandle {
            shared: Arc::new(Shared {
                inner: Arc::clone(self),
                session_id,
                flow,
                transport,
                side,
                open_nanos: self.clock.now_nanos(),
                state: Mutex::new(State::default()),
            }),
        }
    }
}

#[derive(Default)]
struct State {
    events: Vec<Event>,
    payload: usize,
    truncated: bool,
    keys: String,
    local_closed: bool,
    peer_closed: bool,
    reset: bool,
}

struct Shared {
    inner: Arc<Inner>,
    session_id: u64,
    flow: FlowEndpoints,
    transport: Transport,
    side: Side,
    open_nanos: u64,
    state: Mutex<State>,
}

/// The recorder of one sampled session.
///
/// A handle is cheap to clone. The session is submitted to the writer when the
/// last clone drops, which happens when the session ends and the TLS layer,
/// if any, has dropped its tapped stream. The locks inside are private to one
/// sampled session, so no worker contends with another.
#[derive(Clone)]
pub struct SessionHandle {
    shared: Arc<Shared>,
}

impl std::fmt::Debug for SessionHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The key log lines stay out of debug output.
        f.debug_struct("SessionHandle")
            .field("session_id", &self.shared.session_id)
            .finish_non_exhaustive()
    }
}

impl SessionHandle {
    /// Returns the id of the session.
    pub fn session_id(&self) -> u64 {
        self.shared.session_id
    }

    fn push(&self, kind: EventKind) {
        let nanos = self.shared.inner.clock.now_nanos();
        lock(&self.shared.state).events.push(Event { nanos, kind });
    }

    fn record_data(&self, bytes: &[u8], outgoing: bool) {
        let max = self.shared.inner.max_session_bytes;
        let mut st = lock(&self.shared.state);
        let room = max.saturating_sub(st.payload);
        if st.truncated || (bytes.is_empty() && self.shared.transport == Transport::Tcp) {
            return;
        }
        // A datagram is kept whole or not at all, and a stream keeps its prefix.
        let keep = match self.shared.transport {
            Transport::Udp if bytes.len() > room => 0,
            _ => bytes.len().min(room),
        };
        if keep < bytes.len() {
            st.truncated = true;
            if keep == 0 {
                return;
            }
        }
        st.payload += keep;
        let data = bytes[..keep].to_vec();
        let kind = if outgoing {
            EventKind::Out(data)
        } else {
            EventKind::In(data)
        };
        let nanos = self.shared.inner.clock.now_nanos();
        st.events.push(Event { nanos, kind });
    }

    /// Records bytes or one datagram that this end wrote.
    pub fn record_out(&self, bytes: &[u8]) {
        self.record_data(bytes, true);
    }

    /// Records bytes or one datagram that this end read.
    pub fn record_in(&self, bytes: &[u8]) {
        self.record_data(bytes, false);
    }

    /// Records that this end closed its write side, which sends FIN.
    pub fn record_local_close(&self) {
        if !std::mem::replace(&mut lock(&self.shared.state).local_closed, true) {
            self.push(EventKind::LocalClose);
        }
    }

    /// Records that this end read end of stream, so the peer sent FIN.
    pub fn record_peer_close(&self) {
        if !std::mem::replace(&mut lock(&self.shared.state).peer_closed, true) {
            self.push(EventKind::PeerClose);
        }
    }

    /// Records that this end aborted the session, which sends RST.
    pub fn record_local_reset(&self) {
        if !std::mem::replace(&mut lock(&self.shared.state).reset, true) {
            self.push(EventKind::LocalReset);
        }
    }

    /// Records that this end read a connection reset, so the peer sent RST.
    pub fn record_peer_reset(&self) {
        if !std::mem::replace(&mut lock(&self.shared.state).reset, true) {
            self.push(EventKind::PeerReset);
        }
    }

    /// Adds NSS key log lines of this session, which the ring stores next to its packets.
    ///
    /// Give each sampled TLS session its own in memory key log and pass its
    /// contents here when the session ends. Treat the lines like a private key.
    pub fn add_key_lines(&self, nss_lines: &str) {
        let mut st = lock(&self.shared.state);
        st.keys.push_str(nss_lines);
        if !nss_lines.ends_with('\n') {
            st.keys.push('\n');
        }
    }
}

impl Drop for Shared {
    fn drop(&mut self) {
        let st = std::mem::take(&mut *lock(&self.state));
        let record = Record {
            session_id: self.session_id,
            flow: self.flow,
            transport: self.transport,
            side: self.side,
            open_nanos: self.open_nanos,
            end_nanos: self.inner.clock.now_nanos(),
            events: st.events,
            keys: st.keys,
            truncated: st.truncated,
        };
        match self.inner.tx.try_send(Job::Session(Box::new(record))) {
            Ok(()) => {}
            // The queue is full: the writer is behind.
            Err(TrySendError::Full(_)) => {
                self.inner
                    .counters
                    .dropped_overflow
                    .fetch_add(1, Ordering::Relaxed);
            }
            // The writer has ended, which `Capture::stop` does.
            Err(TrySendError::Disconnected(_)) => {
                self.inner
                    .counters
                    .cut_at_stop
                    .fetch_add(1, Ordering::Relaxed);
            }
        }
        let _ = self.inner.seed;
    }
}

/// The writer thread: it synthesizes frames and appends sessions to the ring.
struct Writer {
    ring: ring::Ring,
    mtu: usize,
    seed: u64,
    counters: Arc<Counters>,
    first_error: Arc<Mutex<Option<String>>>,
}

impl Writer {
    fn run(mut self, rx: &std::sync::mpsc::Receiver<Job>) {
        while let Ok(job) = rx.recv() {
            match job {
                Job::Session(record) => self.write(&record),
                Job::Sync(reply) => {
                    let result = self.ring.flush();
                    self.note(result);
                    let _ = reply.send(self.ring.paths());
                }
                Job::Stop => break,
            }
        }
        // A session that was queued behind the stop is not written either.
        while let Ok(job) = rx.try_recv() {
            if matches!(job, Job::Session(_)) {
                self.counters.cut_at_stop.fetch_add(1, Ordering::Relaxed);
            }
        }
        let result = self.ring.close();
        self.note(result);
    }

    fn note(&self, result: Result<(), CaptureError>) {
        if let Err(e) = result {
            self.fail(&e);
        }
    }

    fn fail(&self, e: &CaptureError) {
        self.counters.write_errors.fetch_add(1, Ordering::Relaxed);
        let mut slot = lock(&self.first_error);
        if slot.is_none() {
            *slot = Some(match std::error::Error::source(e) {
                Some(cause) => format!("{e}: {cause}"),
                None => e.to_string(),
            });
        }
    }

    fn write(&mut self, r: &Record) {
        let params = Params {
            flow: r.flow,
            side: r.side,
            mtu: self.mtu,
            seed: self.seed,
            session_id: r.session_id,
            open_nanos: r.open_nanos,
            end_nanos: r.end_nanos,
        };
        let frames = frames::synthesize(&params, r.transport, &r.events);
        let mut packets = Vec::new();
        let mut frame_bytes = 0u64;
        for f in &frames {
            pcapng::push_enhanced_packet(&mut packets, f.nanos, &f.data);
            frame_bytes += f.data.len() as u64;
        }
        match self.ring.append(&packets, &r.keys) {
            Ok(Some(done)) => {
                let c = &self.counters;
                c.written.fetch_add(1, Ordering::Relaxed);
                c.packets.fetch_add(frames.len() as u64, Ordering::Relaxed);
                c.bytes.fetch_add(frame_bytes, Ordering::Relaxed);
                c.opened.fetch_add(done.opened, Ordering::Relaxed);
                c.evicted.fetch_add(done.evicted, Ordering::Relaxed);
                if r.truncated {
                    c.truncated.fetch_add(1, Ordering::Relaxed);
                }
            }
            Ok(None) => {
                self.counters
                    .dropped_oversize
                    .fetch_add(1, Ordering::Relaxed);
            }
            Err(e) => self.fail(&e),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc::{sync_channel, Receiver};

    use super::*;

    struct Fixed;

    impl CaptureClock for Fixed {
        fn now_nanos(&self) -> u64 {
            1_000
        }
    }

    fn flow(id: u64) -> FlowEndpoints {
        FlowEndpoints::new(
            std::net::SocketAddr::from(([10, 0, 0, 1], 40_000 + id as u16)),
            std::net::SocketAddr::from(([10, 0, 0, 2], 80)),
        )
        .unwrap()
    }

    fn begin(capture: &Capture, id: u64) -> SessionHandle {
        capture
            .begin(id, flow(id), Transport::Tcp, Side::Client)
            .unwrap()
    }

    /// A capture whose queue holds `depth` sessions and whose writer is the test.
    fn capture_without_writer(depth: usize) -> (Capture, Receiver<Job>) {
        let (tx, rx) = sync_channel(depth);
        let inner = Inner {
            sampler: Sampler::new(SamplePolicy::Rate(1.0), 1).unwrap(),
            seed: 1,
            max_session_bytes: 1 << 20,
            clock: Arc::new(Fixed),
            tx,
            counters: Arc::new(Counters::default()),
            first_error: Arc::new(Mutex::new(None)),
            worker: Mutex::new(None),
        };
        (
            Capture {
                inner: Some(Arc::new(inner)),
            },
            rx,
        )
    }

    #[test]
    fn an_overflowing_queue_counts_overflow_and_nothing_cut_at_stop() {
        // Nobody reads the queue, so two sessions fill it and three find it full.
        let (capture, _rx) = capture_without_writer(2);
        for id in 0..5 {
            begin(&capture, id).record_out(b"x");
        }
        let stats = capture.stats();
        assert_eq!(stats.sessions_sampled, 5);
        assert_eq!(stats.sessions_dropped_overflow, 3, "{stats:?}");
        assert_eq!(stats.sessions_cut_at_stop, 0, "{stats:?}");
        assert_eq!(stats.sessions_dropped_queue, 3, "{stats:?}");
    }

    #[test]
    fn sessions_that_end_after_the_writer_is_gone_are_cut_at_stop() {
        let (capture, rx) = capture_without_writer(2);
        // One session ends in time and two are still open when the writer ends.
        begin(&capture, 0).record_out(b"x");
        let open = vec![begin(&capture, 1), begin(&capture, 2)];
        drop(rx);
        drop(open);
        let stats = capture.stats();
        assert_eq!(stats.sessions_dropped_overflow, 0, "{stats:?}");
        assert_eq!(stats.sessions_cut_at_stop, 2, "{stats:?}");
        assert_eq!(stats.sessions_dropped_queue, 2, "{stats:?}");
    }
}
