// SPDX-License-Identifier: GPL-3.0-only
//! Records one short session of the own stack per OS profile into a pcapng
//! file, so the checks that score a capture against a profile, namely
//! `engine/fps-backend-kernel/tests/netns/handshakes.py` and
//! `python3 -m realism score --os-profile`, run on the own stack without
//! root and without a network namespace (TRN-28).
//!
//! ```text
//! os_profile_capture OUTDIR [PROFILE...]
//! ```
//!
//! With no profile name it records `windows11`, `linux6`, and `macos14`. Both
//! sides of the stack get the profile, the client connects to port 8080, the
//! two sides exchange a few bytes, and the session closes. The capture is the
//! point to point link, so no router lowers the TTL: score it with
//! `--client-hops 0 --server-hops 0`.

use std::io;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use fps_backend_afxdp::memory::frame_pair;
use fps_backend_afxdp::{FrameIo, QueueStats};
use fps_tcp_own::clock::ManualClock;
use fps_tcp_own::profile::BUILTIN_PROFILES;
use fps_tcp_own::{OsProfile, OwnConfig, OwnStack, Pools};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const PORT: u16 = 8080;

/// The frames that both ends sent, in order.
type Log = Arc<Mutex<Vec<Vec<u8>>>>;

/// An end of the link that records what it sends.
struct Tap {
    inner: Box<dyn FrameIo>,
    log: Log,
}

impl FrameIo for Tap {
    fn queue_id(&self) -> u32 {
        self.inner.queue_id()
    }
    fn max_frame_len(&self) -> usize {
        self.inner.max_frame_len()
    }
    fn recv_batch(&mut self, max: usize, frame: &mut dyn FnMut(&[u8])) -> io::Result<usize> {
        self.inner.recv_batch(max, frame)
    }
    fn send_batch_with(
        &mut self,
        count: usize,
        fill: &mut dyn FnMut(usize, &mut [u8]) -> usize,
    ) -> io::Result<usize> {
        let log = Arc::clone(&self.log);
        let mut record = |index: usize, buffer: &mut [u8]| {
            let len = fill(index, buffer);
            if len > 0 && len <= buffer.len() {
                log.lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .push(buffer[..len].to_vec());
            }
            len
        };
        self.inner.send_batch_with(count, &mut record)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
    fn wait_readable(&mut self, timeout: Duration) -> io::Result<bool> {
        self.inner.wait_readable(timeout)
    }
    fn stats(&self) -> QueueStats {
        self.inner.stats()
    }
}

/// Appends one pcapng block of `kind` with `body` (padded to 4 bytes).
fn block(out: &mut Vec<u8>, kind: u32, body: &[u8]) {
    let padded = body.len().div_ceil(4) * 4;
    let total = u32::try_from(12 + padded).unwrap_or(u32::MAX);
    out.extend_from_slice(&kind.to_le_bytes());
    out.extend_from_slice(&total.to_le_bytes());
    out.extend_from_slice(body);
    out.resize(out.len() + padded - body.len(), 0);
    out.extend_from_slice(&total.to_le_bytes());
}

/// The frames as a little endian pcapng file with one Ethernet interface,
/// 100 microseconds apart.
fn pcapng(frames: &[Vec<u8>]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut shb = Vec::new();
    shb.extend_from_slice(&0x1A2B_3C4Du32.to_le_bytes());
    shb.extend_from_slice(&[1, 0, 0, 0]);
    shb.extend_from_slice(&u64::MAX.to_le_bytes());
    block(&mut out, 0x0A0D_0D0A, &shb);
    let mut idb = Vec::new();
    idb.extend_from_slice(&1u16.to_le_bytes());
    idb.extend_from_slice(&[0, 0]);
    idb.extend_from_slice(&65535u32.to_le_bytes());
    block(&mut out, 1, &idb);
    for (i, frame) in frames.iter().enumerate() {
        let micros = 1_000_000u64 + 100 * i as u64;
        let mut epb = Vec::new();
        epb.extend_from_slice(&0u32.to_le_bytes());
        epb.extend_from_slice(&((micros >> 32) as u32).to_le_bytes());
        epb.extend_from_slice(&(micros as u32).to_le_bytes());
        let len = u32::try_from(frame.len()).unwrap_or(u32::MAX);
        epb.extend_from_slice(&len.to_le_bytes());
        epb.extend_from_slice(&len.to_le_bytes());
        epb.extend_from_slice(frame);
        block(&mut out, 6, &epb);
    }
    out
}

/// Runs one session with `profile` on both sides and returns the frames.
async fn record(profile: OsProfile) -> io::Result<Vec<Vec<u8>>> {
    let config = OwnConfig {
        client_os_profile: Some(profile),
        server_os_profile: Some(profile),
        ..OwnConfig::default()
    };
    let (client, server) = frame_pair(config.link_depth, config.mtu + 18);
    let log = Log::default();
    let tap = |end| -> Box<dyn FrameIo> {
        Box::new(Tap {
            inner: Box::new(end),
            log: Arc::clone(&log),
        })
    };
    let clock = Arc::new(ManualClock::new());
    let stack = OwnStack::with_links_and_pools(
        tap(client),
        tap(server),
        config,
        Pools::pair(),
        clock.clone(),
    )
    .map_err(io::Error::other)?;
    stack.listen(PORT)?;
    let driver = {
        let stack = stack.clone();
        tokio::spawn(async move {
            loop {
                stack.pump();
                clock.advance_micros(1_000);
                tokio::task::yield_now().await;
            }
        })
    };
    let (client, server) = tokio::join!(stack.connect(PORT), stack.accept(PORT));
    let (mut client, mut server) = (client?, server?);
    client.write_all(b"ping").await?;
    let mut buf = [0u8; 4];
    server.read_exact(&mut buf).await?;
    server.write_all(b"pong").await?;
    client.read_exact(&mut buf).await?;
    client.shutdown().await?;
    drop((client, server));
    for _ in 0..50 {
        tokio::task::yield_now().await;
    }
    driver.abort();
    let frames = log.lock().unwrap_or_else(PoisonError::into_inner).clone();
    Ok(frames)
}

async fn run(dir: &Path, names: &[String]) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    for name in names {
        let profile = OsProfile::builtin(name).map_err(|e| e.to_string())?;
        let frames = record(profile).await.map_err(|e| format!("{name}: {e}"))?;
        let path = dir.join(format!("{name}.pcapng"));
        std::fs::write(&path, pcapng(&frames)).map_err(|e| format!("{}: {e}", path.display()))?;
        println!("{}: {} frames", path.display(), frames.len());
    }
    Ok(())
}

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let Some(dir) = args.next().map(PathBuf::from) else {
        eprintln!("usage: os_profile_capture OUTDIR [PROFILE...]");
        return ExitCode::from(2);
    };
    let mut names: Vec<String> = args.collect();
    if names.is_empty() {
        names = BUILTIN_PROFILES.iter().map(|n| (*n).to_owned()).collect();
    }
    let runtime = match tokio::runtime::Builder::new_current_thread().build() {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("os_profile_capture: {e}");
            return ExitCode::from(2);
        }
    };
    match runtime.block_on(run(&dir, &names)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("os_profile_capture: {e}");
            ExitCode::FAILURE
        }
    }
}
