// SPDX-License-Identifier: GPL-3.0-only
//! Run time control of a started plan: the rates of `UpdateRates` and the
//! captures of `StartCapture` and `StopCapture`.
//!
//! [`Rates`] joins the two rate registries of a run. `fps-runtime` registers
//! the handles of the TCP client workers, and the UDP workers of this crate
//! register theirs in a registry of their own, so one update spans both and
//! still applies completely or not at all. [`CaptureCell`] is the cell that
//! the workers read when a session begins, so a capture that starts in the
//! middle of a run reaches every worker without a restart. [`Captures`] owns
//! the captures of one plan and the files they leave on the agent.

use std::collections::BTreeSet;
use std::fs::{self, File};
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, RwLock};
use std::time::{Duration, Instant, SystemTime};

use fps_capture::pcapng::{interface_description, section_header, EPB_TYPE};
use fps_capture::{
    Capture, CaptureConfig, KeyLogMode, RingConfig, SamplePolicy, SystemClock, TappedDatagram,
};
use fps_capture::{FlowEndpoints, Side, Transport};
use fps_sched::{Objective, RateControl};
use sha2::{Digest, Sha256};

/// Why an `UpdateRates` command was refused. Each variant but `Internal`
/// maps to `ERROR_CODE_INVALID_ARGUMENT`, and nothing is applied.
#[derive(Debug, thiserror::Error)]
pub enum RatesError {
    /// The command names no mix entry of the plan.
    #[error("mix entry {0:?} is not a mix entry of the plan")]
    Unknown(String),
    /// The plan has the entry, but no client worker of this agent runs it.
    #[error("mix entry {0:?} has no client worker with a rate on this agent")]
    NotDriven(String),
    /// The command names one entry twice.
    #[error("mix entry {0:?} is named twice")]
    Duplicate(String),
    /// The target is not of the kind of the load objective of the entry.
    #[error("mix entry {entry:?} runs {expected}, and the target is of the other kind")]
    Kind {
        /// The mix entry.
        entry: String,
        /// What the entry runs, `sessions_per_second` or `concurrent_sessions`.
        expected: &'static str,
    },
    /// The target is below zero or not finite.
    #[error("mix entry {entry:?}: the rate {value} is not a finite number of at least 0")]
    Invalid {
        /// The mix entry.
        entry: String,
        /// The offending value.
        value: f64,
    },
    /// A registry refused an update that the checks accepted.
    #[error("rate registry: {0}")]
    Internal(String),
}

/// The rate registries of one started plan.
#[derive(Debug, Clone)]
pub struct Rates {
    tcp: Option<Arc<RateControl>>,
    udp: Option<Arc<RateControl>>,
    /// The mix entries of the plan, to tell an unknown entry from one that
    /// this agent does not drive.
    entries: BTreeSet<String>,
}

impl Rates {
    /// Returns the registries of the TCP and UDP client workers of a run of
    /// a plan with the mix entries `entries`.
    pub fn new(
        tcp: Option<Arc<RateControl>>,
        udp: Option<Arc<RateControl>>,
        entries: BTreeSet<String>,
    ) -> Self {
        Self { tcp, udp, entries }
    }

    /// Returns the current total target of `entry` on this agent.
    pub fn total(&self, entry: &str) -> Option<Objective> {
        self.tcp
            .iter()
            .chain(self.udp.iter())
            .find_map(|r| r.total(entry))
    }

    /// Sets the total target of the whole agent for each named mix entry.
    ///
    /// Every entry, kind, and value is checked before anything is stored, so
    /// the update applies completely or not at all. A registry divides a
    /// total among its workers in proportion to their current targets, and an
    /// entry that the update does not name keeps its targets.
    pub fn update(&self, updates: &[(String, Objective)]) -> Result<(), RatesError> {
        let mut seen = BTreeSet::new();
        let mut tcp: Vec<(&str, Objective)> = Vec::new();
        let mut udp: Vec<(&str, Objective)> = Vec::new();
        for (entry, target) in updates {
            if !seen.insert(entry.as_str()) {
                return Err(RatesError::Duplicate(entry.clone()));
            }
            let (current, bucket) = if let Some(c) = self.tcp.as_ref().and_then(|r| r.total(entry))
            {
                (c, &mut tcp)
            } else if let Some(c) = self.udp.as_ref().and_then(|r| r.total(entry)) {
                (c, &mut udp)
            } else if self.entries.contains(entry) {
                return Err(RatesError::NotDriven(entry.clone()));
            } else {
                return Err(RatesError::Unknown(entry.clone()));
            };
            match (current, target) {
                (Objective::SessionsPerSecond(_), Objective::SessionsPerSecond(r)) => {
                    if !(r.is_finite() && *r >= 0.0) {
                        return Err(RatesError::Invalid {
                            entry: entry.clone(),
                            value: *r,
                        });
                    }
                }
                (Objective::ConcurrentSessions(_), Objective::ConcurrentSessions(_)) => {}
                (Objective::SessionsPerSecond(_), _) => {
                    return Err(RatesError::Kind {
                        entry: entry.clone(),
                        expected: "sessions_per_second",
                    })
                }
                (Objective::ConcurrentSessions(_), _) => {
                    return Err(RatesError::Kind {
                        entry: entry.clone(),
                        expected: "concurrent_sessions",
                    })
                }
            }
            bucket.push((entry.as_str(), *target));
        }
        // Everything is checked, so a registry cannot refuse its share.
        for (registry, list) in [(&self.tcp, &tcp), (&self.udp, &udp)] {
            if let (Some(r), false) = (registry, list.is_empty()) {
                r.update(list)
                    .map_err(|e| RatesError::Internal(e.to_string()))?;
            }
        }
        Ok(())
    }
}

struct CellInner {
    on: AtomicBool,
    capture: RwLock<Option<Capture>>,
}

/// The capture that new sessions are tapped into, which the workers read
/// when a session begins.
///
/// A worker calls [`CaptureCell::current`] once per session. While no capture
/// runs that is one relaxed atomic load and no lock. Cloning is cheap, and
/// every clone shares the cell.
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

    /// Wraps a UDP session of this agent in the tap of the current capture.
    /// A session with no capture, or with endpoints of two families, is
    /// wrapped without a recorder.
    pub fn tap_datagram<D: fps_io::DatagramIo>(
        &self,
        session_id: u64,
        side: Side,
        local: SocketAddr,
        peer: SocketAddr,
        io: D,
    ) -> TappedDatagram<D> {
        let handle = self.current().and_then(|capture| {
            let (client, server) = match side {
                Side::Client => (local, peer),
                Side::Server => (peer, local),
            };
            let flow = FlowEndpoints::new(client, server).ok()?;
            capture.begin(session_id, flow, Transport::Udp, side)
        });
        TappedDatagram::new(io, handle)
    }
}

/// Where the captures of the agent live and how large a ring may grow.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptureSettings {
    /// The directory of all captures. Each plan gets a subdirectory and each
    /// capture a file `<capture_id>.pcapng` in it.
    pub dir: PathBuf,
    /// The size at which the ring of a capture starts a new file.
    pub file_bytes: u64,
    /// The number of ring files that a capture keeps, which bounds its size
    /// to `file_bytes * max_files` plus the framing.
    pub max_files: usize,
}

impl Default for CaptureSettings {
    fn default() -> Self {
        Self {
            dir: std::env::temp_dir().join("fps-agent-captures"),
            file_bytes: 64 << 20,
            max_files: 8,
        }
    }
}

/// What a finished capture left on the agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Done {
    /// The number of packets in the file.
    pub packets: u64,
    /// The size of the file in bytes.
    pub bytes: u64,
    /// The number of sessions that the capture could not record, because the
    /// writer queue was full or a session did not fit a ring file. The agent
    /// cannot know how many packets they held.
    pub dropped: u64,
    /// `sha256:` and the lowercase hex digest of the bytes of the file.
    pub digest: String,
    /// The file.
    pub path: PathBuf,
}

/// Why a capture command failed. `Invalid`, `Precondition`, and
/// `Unimplemented` map to the codes of the same name, and `Io` to
/// `ERROR_CODE_RESOURCE_EXHAUSTED`, because `ERROR_CODE_INTERNAL` would move
/// a healthy run to `FAILED`.
#[derive(Debug, thiserror::Error)]
pub enum CaptureError {
    /// The command breaks a rule of the contract.
    #[error("{0}")]
    Invalid(String),
    /// The command is out of order.
    #[error("{0}")]
    Precondition(String),
    /// The agent does not implement what the command asks for.
    #[error("{0}")]
    Unimplemented(String),
    /// The agent could not read or write the capture files.
    #[error("{0}")]
    Io(String),
}

/// One capture of the plan.
pub struct Entry {
    id: String,
    out: PathBuf,
    ring: PathBuf,
    prefix: String,
    capture: Capture,
    started: Instant,
    started_at: SystemTime,
    max_bytes: u64,
    max_duration: Option<Duration>,
    /// Set once something started to end the capture, so a limit ends it once.
    finishing: AtomicBool,
    /// Set once the capture ended, and the lock serializes the finishing.
    done: Mutex<Option<Result<Done, String>>>,
}

impl std::fmt::Debug for Entry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Entry")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

impl Entry {
    /// The time at which the capture began.
    pub fn started_at(&self) -> SystemTime {
        self.started_at
    }

    fn limit_reached(&self) -> bool {
        self.max_duration
            .is_some_and(|d| self.started.elapsed() >= d)
            || (self.max_bytes > 0 && self.capture.stats().bytes >= self.max_bytes)
    }
}

#[derive(Default)]
struct Book {
    plan_id: String,
    entries: Vec<Arc<Entry>>,
    /// The id of the capture that the cell feeds.
    active: Option<String>,
}

/// The captures of the loaded plan.
pub struct Captures {
    settings: CaptureSettings,
    cell: CaptureCell,
    book: Mutex<Book>,
}

impl std::fmt::Debug for Captures {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Captures")
            .field("settings", &self.settings)
            .finish_non_exhaustive()
    }
}

/// Returns the directory of the captures of `plan_id` below `dir`.
///
/// A plan id such as `run-0001/agent-a` can hold a path separator, so the
/// name keeps its letters, digits, dots, underscores, and hyphens, turns every
/// other byte into an underscore, and ends with eight hex digits of the
/// SHA-256 of the whole id, so two ids never share a directory.
pub fn plan_dir(dir: &Path, plan_id: &str) -> PathBuf {
    let readable: String = plan_id
        .chars()
        .take(64)
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '_'
            }
        })
        .collect();
    let digest = hex(&Sha256::digest(plan_id.as_bytes()));
    dir.join(format!(
        "{}-{}",
        readable.trim_start_matches('.'),
        &digest[..8]
    ))
}

/// Returns the file that the capture `capture_id` of plan `plan_id` leaves
/// below `dir`, once `StopCaptureResult` has been sent. The controller knows
/// all three values, so it can locate the file without a field in the result.
pub fn capture_path(dir: &Path, plan_id: &str, capture_id: &str) -> PathBuf {
    plan_dir(dir, plan_id).join(format!("{capture_id}.pcapng"))
}

/// A name that is safe as a path component and as a file name prefix.
fn plain_name(what: &str, s: &str) -> Result<(), CaptureError> {
    let ok = !s.is_empty()
        && s.len() <= 128
        && !s.starts_with('.')
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'));
    if ok {
        Ok(())
    } else {
        Err(CaptureError::Invalid(format!(
            "{what} {s:?} must have 1 to 128 characters of letters, digits, dots, underscores, and hyphens, and not start with a dot"
        )))
    }
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Captures {
    /// Returns the captures of an agent with `settings`.
    pub fn new(settings: CaptureSettings) -> Self {
        Self {
            settings,
            cell: CaptureCell::new(),
            book: Mutex::new(Book::default()),
        }
    }

    /// Returns the cell that the workers read.
    pub fn cell(&self) -> CaptureCell {
        self.cell.clone()
    }

    /// Forgets the captures of an earlier plan and names the plan that is
    /// loaded now. The files stay on the agent.
    pub fn reset(&self, plan_id: &str) {
        self.cell.clear();
        let mut book = lock(&self.book);
        book.plan_id = plan_id.to_owned();
        book.entries.clear();
        book.active = None;
    }

    /// Starts a capture of every session of the agent.
    ///
    /// The engine sees sessions and no packets, so the capture records the
    /// sessions that the workers tap, with synthesized frames. A port name, a
    /// filter expression, or a snap length would narrow what the capture
    /// keeps, and the agent cannot do that yet, so it refuses them instead of
    /// recording more than the controller asked for.
    #[allow(clippy::too_many_arguments)]
    pub fn start(
        &self,
        plan_id: &str,
        capture_id: &str,
        port: &str,
        filter: &str,
        snap_length: u32,
        max_bytes: u64,
        max_duration: Option<Duration>,
        plan_ports: &[String],
        seed: u64,
    ) -> Result<Arc<Entry>, CaptureError> {
        plain_name("capture_id", capture_id)?;
        if !port.is_empty() {
            return Err(if plan_ports.iter().any(|p| p == port) {
                CaptureError::Unimplemented(format!(
                    "capture on port {port:?}: the agent captures sessions and cannot tell the ports of a plan apart, so start the capture with an empty port"
                ))
            } else {
                CaptureError::Invalid(format!("port {port:?} is not a port of the plan"))
            });
        }
        if !filter.is_empty() {
            return Err(CaptureError::Invalid(format!(
                "filter {filter:?}: this agent has no pcap-filter compiler, so it accepts only an empty filter"
            )));
        }
        if snap_length != 0 {
            return Err(CaptureError::Unimplemented(
                "snap_length: this agent keeps whole sessions, so it accepts only a snap length of 0"
                    .into(),
            ));
        }
        let mut book = lock(&self.book);
        if book.plan_id != plan_id {
            return Err(CaptureError::Precondition(format!(
                "plan {plan_id} is not loaded"
            )));
        }
        if book.entries.iter().any(|e| e.id == capture_id) {
            return Err(CaptureError::Precondition(format!(
                "capture {capture_id} already exists in plan {plan_id}"
            )));
        }
        if let Some(active) = &book.active {
            return Err(CaptureError::Precondition(format!(
                "capture {active} is running, and the agent runs one capture at a time"
            )));
        }
        let io = |what: &str, p: &Path, e: io::Error| {
            CaptureError::Io(format!("{what} {}: {e}", p.display()))
        };
        let plan_dir = plan_dir(&self.settings.dir, plan_id);
        create_private_dir(&plan_dir).map_err(|e| io("create", &plan_dir, e))?;
        let ring = plan_dir.join(format!("{capture_id}.ring"));
        let out = capture_path(&self.settings.dir, plan_id, capture_id);
        // A capture of an earlier plan with the same names would otherwise
        // join this one, because a ring adopts the files of its prefix.
        if ring.exists() {
            fs::remove_dir_all(&ring).map_err(|e| io("remove", &ring, e))?;
        }
        if out.exists() {
            fs::remove_file(&out).map_err(|e| io("remove", &out, e))?;
        }
        let mut ring_cfg = RingConfig::new(&ring);
        ring_cfg.prefix = capture_id.to_owned();
        ring_cfg.file_bytes = self.settings.file_bytes;
        ring_cfg.max_files = self.settings.max_files;
        let mut cfg = CaptureConfig::new(ring_cfg);
        cfg.sample = SamplePolicy::EveryNth(1);
        cfg.seed = seed;
        // One file that decrypts alone, with no key log beside it.
        cfg.key_log = KeyLogMode::Embedded;
        let capture = Capture::start(cfg, Arc::new(SystemClock))
            .map_err(|e| CaptureError::Io(format!("start the capture: {e}")))?;
        let entry = Arc::new(Entry {
            id: capture_id.to_owned(),
            out,
            ring,
            prefix: capture_id.to_owned(),
            capture: capture.clone(),
            started: Instant::now(),
            started_at: SystemTime::now(),
            max_bytes,
            max_duration,
            finishing: AtomicBool::new(false),
            done: Mutex::new(None),
        });
        book.entries.push(Arc::clone(&entry));
        book.active = Some(capture_id.to_owned());
        drop(book);
        self.cell.set(capture);
        Ok(entry)
    }

    /// Returns the capture `capture_id` of plan `plan_id`.
    pub fn find(&self, plan_id: &str, capture_id: &str) -> Option<Arc<Entry>> {
        let book = lock(&self.book);
        if book.plan_id != plan_id {
            return None;
        }
        book.entries.iter().find(|e| e.id == capture_id).cloned()
    }

    /// Returns each capture that reached its byte or time limit and that
    /// nothing is ending yet, once. The caller ends it.
    pub fn over_limit(&self) -> Vec<Arc<Entry>> {
        let book = lock(&self.book);
        book.entries
            .iter()
            .filter(|e| !e.finishing.load(Ordering::Acquire) && e.limit_reached())
            .filter(|e| !e.finishing.swap(true, Ordering::AcqRel))
            .cloned()
            .collect()
    }

    /// Ends the capture and returns what it left. Calling it again returns
    /// the same answer, and concurrent calls wait for the first one.
    pub fn finish(&self, entry: &Arc<Entry>) -> Result<Done, String> {
        entry.finishing.store(true, Ordering::Release);
        let mut done = lock(&entry.done);
        if let Some(r) = done.as_ref() {
            return r.clone();
        }
        // New sessions stop at once, and sessions that are open keep their
        // recorder and reach the writer when they end, or are dropped after
        // the stop and counted.
        {
            let mut book = lock(&self.book);
            if book.active.as_deref() == Some(entry.id.as_str()) {
                book.active = None;
                self.cell.clear();
            }
        }
        let stats = entry.capture.stop();
        let result = assemble(
            entry,
            stats.sessions_dropped_queue + stats.sessions_dropped_oversize,
        );
        if let Err(e) = &result {
            tracing::warn!(capture_id = %entry.id, error = %e, "capture ended without a file");
        } else if let Ok(d) = &result {
            tracing::info!(
                capture_id = %entry.id,
                path = %d.path.display(),
                packets = d.packets,
                bytes = d.bytes,
                dropped = d.dropped,
                digest = %d.digest,
                "capture file ready"
            );
        }
        *done = Some(result.clone());
        result
    }

    /// Ends every capture that still runs, for the end of the run.
    pub fn finish_all(&self) {
        let entries: Vec<_> = lock(&self.book).entries.clone();
        for e in entries {
            let _ = self.finish(&e);
        }
    }
}

#[cfg(unix)]
fn create_private_dir(p: &Path) -> io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    fs::DirBuilder::new().recursive(true).mode(0o700).create(p)
}

#[cfg(not(unix))]
fn create_private_dir(p: &Path) -> io::Result<()> {
    fs::create_dir_all(p)
}

/// Joins the ring files into one pcapng file and returns its digest.
///
/// The ring files are sections of one format, so concatenating them oldest
/// first gives one valid pcapng file with several sections. A ring with no
/// session gives a file with the section header and the interface description
/// only. The packets are counted from the blocks, which also checks every
/// block length.
fn assemble(entry: &Entry, dropped: u64) -> Result<Done, String> {
    let err =
        |what: &str, p: &Path, e: &dyn std::fmt::Display| format!("{what} {}: {e}", p.display());
    let mut files: Vec<(u64, PathBuf)> = Vec::new();
    if entry.ring.exists() {
        let read = fs::read_dir(&entry.ring).map_err(|e| err("read", &entry.ring, &e))?;
        for f in read.flatten() {
            let name = f.file_name().to_string_lossy().into_owned();
            if let Some(i) = name
                .strip_prefix(&format!("{}-", entry.prefix))
                .and_then(|r| r.strip_suffix(".pcapng"))
                .and_then(|n| n.parse::<u64>().ok())
            {
                files.push((i, f.path()));
            }
        }
    }
    files.sort();
    let tmp = entry.out.with_extension("pcapng.part");
    let mut out = BufWriter::new(File::create(&tmp).map_err(|e| err("create", &tmp, &e))?);
    let mut hasher = Sha256::new();
    let mut bytes = 0u64;
    let mut packets = 0u64;
    let mut emit = |chunk: &[u8], out: &mut BufWriter<File>| -> io::Result<()> {
        hasher.update(chunk);
        bytes += chunk.len() as u64;
        out.write_all(chunk)
    };
    if files.is_empty() {
        let mut header = section_header(concat!("fps-agent ", env!("CARGO_PKG_VERSION")));
        header.extend_from_slice(&interface_description("fps-synthetic"));
        emit(&header, &mut out).map_err(|e| err("write", &tmp, &e))?;
    }
    for (_, path) in &files {
        let mut input = BufReader::new(File::open(path).map_err(|e| err("open", path, &e))?);
        loop {
            let mut head = [0u8; 8];
            match read_full(&mut input, &mut head) {
                Ok(0) => break,
                Ok(8) => {}
                Ok(_) => return Err(err("read", path, &"a block header is cut")),
                Err(e) => return Err(err("read", path, &e)),
            }
            let kind = u32::from_le_bytes([head[0], head[1], head[2], head[3]]);
            let total = u32::from_le_bytes([head[4], head[5], head[6], head[7]]) as usize;
            if total < 12 || !total.is_multiple_of(4) || total > (16 << 20) {
                return Err(err(
                    "read",
                    path,
                    &format!("block length {total} is invalid"),
                ));
            }
            let mut block = vec![0u8; total];
            block[..8].copy_from_slice(&head);
            input
                .read_exact(&mut block[8..])
                .map_err(|e| err("read", path, &e))?;
            if kind == EPB_TYPE {
                packets += 1;
            }
            emit(&block, &mut out).map_err(|e| err("write", &tmp, &e))?;
        }
    }
    out.flush().map_err(|e| err("write", &tmp, &e))?;
    drop(out);
    fs::rename(&tmp, &entry.out).map_err(|e| err("rename", &tmp, &e))?;
    // The ring is spent. A failure to remove it leaves files and no wrong answer.
    let _ = fs::remove_dir_all(&entry.ring);
    Ok(Done {
        packets,
        bytes,
        dropped,
        digest: format!("sha256:{}", hex(&hasher.finalize())),
        path: entry.out.clone(),
    })
}

fn read_full(r: &mut impl Read, buf: &mut [u8]) -> io::Result<usize> {
    let mut n = 0;
    while n < buf.len() {
        match r.read(&mut buf[n..])? {
            0 => break,
            k => n += k,
        }
    }
    Ok(n)
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(String::with_capacity(64), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use fps_sched::{LoadProfile, Phase, Schedule, ScheduleConfig, Shape};

    fn handle(objective: Objective) -> fps_sched::RateHandle {
        let profile = LoadProfile::new(vec![Phase::new(
            "steady",
            Duration::from_secs(10),
            Shape::Hold,
            1.0,
            1.0,
        )])
        .unwrap();
        Schedule::new(ScheduleConfig::new(objective, profile, 1))
            .unwrap()
            .rate_handle()
    }

    fn rates() -> Rates {
        let tcp = RateControl::new();
        tcp.register("web", handle(Objective::SessionsPerSecond(30.0)))
            .unwrap();
        tcp.register("web", handle(Objective::SessionsPerSecond(10.0)))
            .unwrap();
        tcp.register("users", handle(Objective::ConcurrentSessions(8)))
            .unwrap();
        let udp = RateControl::new();
        udp.register("dns", handle(Objective::SessionsPerSecond(5.0)))
            .unwrap();
        let entries = ["web", "users", "dns", "idle"].map(String::from).into();
        Rates::new(Some(Arc::new(tcp)), Some(Arc::new(udp)), entries)
    }

    fn sps(entry: &str, r: f64) -> (String, Objective) {
        (entry.into(), Objective::SessionsPerSecond(r))
    }

    #[test]
    fn an_update_spans_both_registries_and_keeps_unnamed_entries() {
        let r = rates();
        r.update(&[sps("web", 80.0), sps("dns", 20.0)]).unwrap();
        assert_eq!(r.total("web"), Some(Objective::SessionsPerSecond(80.0)));
        assert_eq!(r.total("dns"), Some(Objective::SessionsPerSecond(20.0)));
        assert_eq!(r.total("users"), Some(Objective::ConcurrentSessions(8)));
        r.update(&[("users".into(), Objective::ConcurrentSessions(3))])
            .unwrap();
        assert_eq!(r.total("users"), Some(Objective::ConcurrentSessions(3)));
    }

    #[test]
    fn a_refused_update_changes_nothing() {
        let r = rates();
        let before = (r.total("web"), r.total("dns"));
        let bad: [Vec<(String, Objective)>; 6] = [
            vec![sps("web", 1.0), sps("nope", 1.0)],
            vec![sps("web", 1.0), sps("idle", 1.0)],
            vec![sps("dns", 1.0), sps("dns", 2.0)],
            vec![
                sps("dns", 1.0),
                ("web".into(), Objective::ConcurrentSessions(4)),
            ],
            vec![sps("web", 1.0), sps("dns", -1.0)],
            vec![sps("web", 1.0), sps("dns", f64::NAN)],
        ];
        let kinds: Vec<_> = bad
            .iter()
            .map(|u| match r.update(u).unwrap_err() {
                RatesError::Unknown(_) => "unknown",
                RatesError::NotDriven(_) => "not driven",
                RatesError::Duplicate(_) => "duplicate",
                RatesError::Kind { .. } => "kind",
                RatesError::Invalid { .. } => "invalid",
                RatesError::Internal(_) => "internal",
            })
            .collect();
        assert_eq!(
            kinds,
            [
                "unknown",
                "not driven",
                "duplicate",
                "kind",
                "invalid",
                "invalid"
            ]
        );
        assert_eq!((r.total("web"), r.total("dns")), before);
    }

    #[test]
    fn the_cell_is_empty_until_a_capture_is_set() {
        let cell = CaptureCell::new();
        assert!(cell.current().is_none());
        cell.set(Capture::disabled());
        assert!(cell.current().is_some());
        cell.clear();
        assert!(cell.current().is_none());
    }

    #[test]
    fn plan_ids_with_separators_get_distinct_flat_directories() {
        let base = Path::new("/captures");
        let a = plan_dir(base, "run-0001/agent-a");
        let b = plan_dir(base, "run-0001_agent-a");
        assert_ne!(a, b);
        assert_eq!(a.parent(), Some(base));
        assert!(a
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("run-0001_agent-a-"));
        assert_eq!(
            plan_dir(base, "../etc").parent(),
            Some(base),
            "a plan id cannot leave the capture directory"
        );
        assert_eq!(capture_path(base, "p", "c1").extension().unwrap(), "pcapng");
    }

    #[test]
    fn capture_names_must_be_plain() {
        assert!(plain_name("x", "cap-1.a_b").is_ok());
        for bad in ["", ".hidden", "a/b", "..", "a b", "é"] {
            assert!(plain_name("x", bad).is_err(), "{bad:?}");
        }
    }
}
