// SPDX-License-Identifier: GPL-3.0-only
//! The interpreter of one session side.
//!
//! A TCP side runs over a [`SessionIo`] byte stream and a UDP side over a
//! [`DatagramIo`] socket. Both run the same loop over the compiled ops and
//! differ only in `send`, `expect`, and `close`.

use std::future::Future;
use std::io::{self, IoSlice};
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

use bytes::{Buf, BytesMut};
use fps_flow::program::{CloseMode, Op, Program, StopRule};
use fps_flow::template::{Filler, Part, RenderCtx};
use fps_flow::{Template, TemplateFn, Value, Vars};
use fps_io::{DatagramIo, DatagramIoExt, SessionIo, MAX_DATAGRAM};
use rand::Rng;
use rand_pcg::Pcg64Mcg;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadBuf};
use tokio::time::error::Elapsed;
use tokio::time::{timeout_at, Instant};

use crate::delay;
use crate::failure::{Failure, FailureKind};
use crate::open::{DatagramOpener, OpenKind, OpenRequest, Opener};
use crate::rng::Side;

/// Growth step of the read buffer. Spike S1 measured that 16 KiB reads keep
/// small request and response exchanges cheap without per read reallocation.
pub const READ_CHUNK: usize = 16 * 1024;

/// Size of the scratch buffer through which `expect bytes` discards payload.
/// Spike S1 measured that 64 KiB reads keep bulk bodies near line rate, and
/// the payload never grows the read buffer.
pub const DISCARD_CHUNK: usize = 64 * 1024;

/// Smallest `fill()` body that `send` writes straight from the shared
/// filler. Shorter bodies are copied into the write buffer, which costs
/// less than an extra I/O slice. Spike S1 measured that copying 1 MiB
/// bodies cost about a quarter of the bulk session rate.
pub const VECTORED_FILL_MIN: usize = 16 * 1024;

/// Largest number of I/O slices that one vectored write passes, well below
/// the `IOV_MAX` of every supported kernel.
const MAX_SLICES: usize = 16;

/// Limits and timeouts of a session that the flow program does not set.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// Bound on `connect` and `accept`.
    pub open_timeout: Duration,
    /// Bound on the drain of `close: fin`, which waits for the peer to close.
    pub drain_timeout: Duration,
    /// Largest number of unmatched bytes that `expect pattern` buffers before
    /// it fails with reason `pattern_mismatch`.
    pub max_pattern_buffer: usize,
    /// Largest `last` that the session keeps for templates that refer to it.
    pub max_last: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            open_timeout: Duration::from_secs(5),
            drain_timeout: Duration::from_secs(10),
            max_pattern_buffer: 16 << 20,
            max_last: 16 << 20,
        }
    }
}

/// Everything one session side needs besides its program and transport.
pub struct SessionCtx<'a> {
    /// Scenario variables after overrides.
    pub vars: &'a Vars,
    /// The shared filler of the run.
    pub filler: &'a Filler,
    /// Flow id.
    pub flow: &'a str,
    /// The side that this session runs.
    pub side: Side,
    /// Session id that `{{ session.id }}` renders.
    pub session_id: u64,
    /// The session generator, normally from [`crate::session_rng`].
    pub rng: Pcg64Mcg,
    /// Module functions of the session.
    pub functions: &'a mut dyn TemplateFn,
    /// Limits and timeouts.
    pub limits: Limits,
}

/// Byte counters of one session side, counting application payload only.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SideStats {
    /// Bytes written.
    pub bytes_tx: u64,
    /// Bytes read, including bytes that `close: fin` drained.
    pub bytes_rx: u64,
    /// True once `connect` or `accept` completed.
    pub established: bool,
}

/// One side of one session.
///
/// A session owns its read, write, and scratch buffers and reuses them for
/// every action, so actions do not allocate except where a template or a
/// `set` needs it.
pub struct Session<'a> {
    vars: &'a Vars,
    filler: &'a Filler,
    flow: &'a str,
    side: Side,
    session_id: u64,
    rng: Pcg64Mcg,
    functions: &'a mut dyn TemplateFn,
    limits: Limits,
    overlay: Vars,
    last: Vec<u8>,
    keep_last: bool,
    rbuf: BytesMut,
    wbuf: Vec<u8>,
    parts: Vec<Part>,
    scratch: Vec<u8>,
    counters: Vec<i64>,
    pc: usize,
    stats: SideStats,
    max_datagram: usize,
}

impl<'a> Session<'a> {
    /// Creates a session side.
    pub fn new(ctx: SessionCtx<'a>) -> Self {
        Session {
            vars: ctx.vars,
            filler: ctx.filler,
            flow: ctx.flow,
            side: ctx.side,
            session_id: ctx.session_id,
            rng: ctx.rng,
            functions: ctx.functions,
            limits: ctx.limits,
            overlay: Vars::new(),
            last: Vec::new(),
            keep_last: false,
            rbuf: BytesMut::with_capacity(READ_CHUNK),
            wbuf: Vec::with_capacity(READ_CHUNK),
            parts: Vec::new(),
            scratch: Vec::new(),
            counters: Vec::new(),
            pc: 0,
            stats: SideStats::default(),
            max_datagram: MAX_DATAGRAM,
        }
    }

    /// Sets the largest datagram that an `expect` of a UDP side accepts.
    ///
    /// The value is clamped to the range from 1 to
    /// [`fps_io::MAX_DATAGRAM`], which is also the default. A longer
    /// datagram fails the session with reason `pattern_mismatch`, and the
    /// interpreter never truncates it silently.
    pub fn set_max_datagram(&mut self, limit: usize) {
        self.max_datagram = limit.clamp(1, MAX_DATAGRAM);
    }

    /// Returns the datagram limit of `expect` on a UDP side.
    pub fn max_datagram(&self) -> usize {
        self.max_datagram
    }

    /// Returns the byte counters.
    pub fn stats(&self) -> SideStats {
        self.stats
    }

    /// Returns the variables that `set` actions bound.
    pub fn overlay(&self) -> &Vars {
        &self.overlay
    }

    /// Opens the session with `opener` and runs the body of `program`.
    ///
    /// When `stop` completes first, the session aborts its transport and
    /// fails with reason `aborted`. The transport is dropped at the end, so
    /// a program without `close` ends like a close by the process.
    pub async fn run_program<O: Opener, F: Future<Output = ()>>(
        &mut self,
        opener: &mut O,
        program: &Program,
        stop: F,
    ) -> Result<(), Failure> {
        tokio::pin!(stop);
        let mut io = self.open_pinned(opener, program, stop.as_mut()).await?;
        self.run_pinned(Wire::<_, NoDatagram>::Stream(&mut io), &program.ops, stop)
            .await
    }

    /// Opens a UDP session with `opener` and runs the body of `program`
    /// under the datagram rules.
    ///
    /// When `stop` completes first, the session closes its socket and fails
    /// with reason `aborted`. The socket is dropped at the end, so a program
    /// without `close` releases it too.
    pub async fn run_datagram_program<O: DatagramOpener, F: Future<Output = ()>>(
        &mut self,
        opener: &mut O,
        program: &Program,
        stop: F,
    ) -> Result<(), Failure> {
        tokio::pin!(stop);
        let mut io = self
            .open_datagram_pinned(opener, program, stop.as_mut())
            .await?;
        self.run_pinned(Wire::<NoStream, _>::Datagram(&mut io), &program.ops, stop)
            .await
    }

    /// Renders the opening action of a UDP `program` and opens the socket.
    pub async fn open_datagram<O: DatagramOpener, F: Future<Output = ()>>(
        &mut self,
        opener: &mut O,
        program: &Program,
        stop: F,
    ) -> Result<O::Io, Failure> {
        tokio::pin!(stop);
        self.open_datagram_pinned(opener, program, stop).await
    }

    /// Runs `ops` over an open UDP socket under the datagram rules.
    ///
    /// When `stop` completes first, the session closes `io` and fails with
    /// reason `aborted`.
    pub async fn run_datagram<D: DatagramIo, F: Future<Output = ()>>(
        &mut self,
        io: &mut D,
        ops: &[Op],
        stop: F,
    ) -> Result<(), Failure> {
        tokio::pin!(stop);
        self.run_pinned(Wire::<NoStream, _>::Datagram(io), ops, stop)
            .await
    }

    /// Renders the opening action of `program` and opens the transport.
    pub async fn open<O: Opener, F: Future<Output = ()>>(
        &mut self,
        opener: &mut O,
        program: &Program,
        stop: F,
    ) -> Result<O::Io, Failure> {
        tokio::pin!(stop);
        self.open_pinned(opener, program, stop).await
    }

    /// Runs `ops` over an open transport.
    ///
    /// When `stop` completes first, the session aborts `io` and fails with
    /// reason `aborted`.
    pub async fn run<S: SessionIo, F: Future<Output = ()>>(
        &mut self,
        io: &mut S,
        ops: &[Op],
        stop: F,
    ) -> Result<(), Failure> {
        tokio::pin!(stop);
        self.run_pinned(Wire::<_, NoDatagram>::Stream(io), ops, stop)
            .await
    }

    async fn open_pinned<O: Opener, F: Future<Output = ()>>(
        &mut self,
        opener: &mut O,
        program: &Program,
        stop: Pin<&mut F>,
    ) -> Result<O::Io, Failure> {
        let (kind, port, host) = self.render_open(program)?;
        let request = OpenRequest {
            kind,
            host: program.host.as_ref().map(|_| host.as_slice()),
            port,
        };
        let opened = race(stop, self.limits.open_timeout, opener.open(request)).await;
        self.wbuf = host;
        self.finish_open(opened)
    }

    async fn open_datagram_pinned<O: DatagramOpener, F: Future<Output = ()>>(
        &mut self,
        opener: &mut O,
        program: &Program,
        stop: Pin<&mut F>,
    ) -> Result<O::Io, Failure> {
        let (kind, port, host) = self.render_open(program)?;
        let request = OpenRequest {
            kind,
            host: program.host.as_ref().map(|_| host.as_slice()),
            port,
        };
        let opened = race(stop, self.limits.open_timeout, opener.open(request)).await;
        self.wbuf = host;
        self.finish_open(opened)
    }

    /// Renders the port and host of the opening action. The host lands in
    /// the returned buffer, which the caller hands back as the write buffer.
    fn render_open(&mut self, program: &Program) -> Result<(OpenKind, u16, Vec<u8>), Failure> {
        let fail = |s: &Self, kind| s.open_failure(kind);
        let port = self.render_int(&program.port).map_err(|k| fail(self, k))?;
        let port = u16::try_from(port).map_err(|_| {
            fail(
                self,
                FailureKind::Invalid(format!("port {port} is out of range")),
            )
        })?;
        let mut host = std::mem::take(&mut self.wbuf);
        host.clear();
        if let Some(t) = &program.host {
            let (mut ctx, _) = self.split();
            if let Err(source) = t.render_into(&mut ctx, &mut host) {
                self.wbuf = host;
                return Err(fail(self, FailureKind::Render { source }));
            }
        }
        let kind = match self.side {
            Side::Client => OpenKind::Connect,
            Side::Server => OpenKind::Accept,
        };
        Ok((kind, port, host))
    }

    /// Maps the outcome of an open race to the transport or a failure.
    fn finish_open<T>(
        &mut self,
        opened: Option<Result<io::Result<T>, Elapsed>>,
    ) -> Result<T, Failure> {
        let timeout_ms = millis(self.limits.open_timeout);
        match opened {
            None => Err(self.open_failure(FailureKind::Aborted)),
            Some(Err(_)) => Err(self.open_failure(FailureKind::OpenTimeout { timeout_ms })),
            Some(Ok(Err(e))) => Err(self.open_failure(FailureKind::from_open_io(e, timeout_ms))),
            Some(Ok(Ok(io))) => {
                self.stats.established = true;
                Ok(io)
            }
        }
    }

    async fn run_pinned<S: SessionIo, D: DatagramIo, F: Future<Output = ()>>(
        &mut self,
        mut wire: Wire<'_, S, D>,
        ops: &[Op],
        stop: Pin<&mut F>,
    ) -> Result<(), Failure> {
        self.keep_last = ops.iter().any(op_uses_last);
        self.pc = 0;
        self.counters.clear();
        let outcome = {
            let body = self.run_ops(&mut wire, ops);
            tokio::pin!(body);
            tokio::select! {
                biased;
                () = stop => None,
                r = &mut body => Some(r),
            }
        };
        match outcome {
            Some(Ok(())) => Ok(()),
            Some(Err(kind)) => Err(self.failure(ops, kind)),
            None => {
                match wire {
                    Wire::Stream(io) => io.abort(),
                    Wire::Datagram(io) => io.close(),
                }
                Err(self.failure(ops, FailureKind::Aborted))
            }
        }
    }

    async fn run_ops<S: SessionIo, D: DatagramIo>(
        &mut self,
        wire: &mut Wire<'_, S, D>,
        ops: &[Op],
    ) -> Result<(), FailureKind> {
        while let Some(op) = ops.get(self.pc) {
            match op {
                Op::Send(t) => match wire {
                    Wire::Stream(io) => self.send(*io, t).await?,
                    Wire::Datagram(io) => self.send_datagram(*io, t).await?,
                },
                Op::Expect { rule, timeout } => match wire {
                    Wire::Stream(io) => self.expect(*io, rule, *timeout).await?,
                    Wire::Datagram(io) => self.expect_datagram(*io, rule, *timeout).await?,
                },
                Op::Delay(d) => {
                    let fixed_ms = match d {
                        fps_flow::program::Delay::Fixed(t) => self.render_int(t)?,
                        _ => 0,
                    };
                    let dur =
                        delay::sample(d, fixed_ms, &mut self.rng).map_err(FailureKind::Invalid)?;
                    if !dur.is_zero() {
                        tokio::time::sleep(dur).await;
                    }
                }
                Op::Set(binds) => {
                    for (name, t) in binds {
                        self.wbuf.clear();
                        let (mut ctx, out) = self.split();
                        t.render_into(&mut ctx, out)
                            .map_err(|source| FailureKind::Render { source })?;
                        let text = String::from_utf8_lossy(&self.wbuf);
                        let value = Value::parse_override(&text);
                        self.overlay.insert(name.clone(), value);
                    }
                }
                Op::LoopStart { count, end } => {
                    let n = self.render_int(count)?;
                    if n < 0 {
                        return Err(FailureKind::Invalid(format!(
                            "loop count is negative, got {n}"
                        )));
                    }
                    if n == 0 {
                        self.pc = end + 1;
                        continue;
                    }
                    self.counters.push(n);
                }
                Op::LoopEnd { start } => {
                    let Some(top) = self.counters.last_mut() else {
                        return Err(FailureKind::Invalid(
                            "invariant: loop end without a running loop".into(),
                        ));
                    };
                    *top -= 1;
                    if *top > 0 {
                        self.pc = start + 1;
                        continue;
                    }
                    self.counters.pop();
                }
                Op::Branch { arms } => {
                    let x: f64 = self.rng.gen();
                    self.pc = arms
                        .iter()
                        .find(|(c, _)| x < *c)
                        .or(arms.last())
                        .map_or(self.pc + 1, |a| a.1);
                    continue;
                }
                Op::Jump(to) => {
                    self.pc = *to;
                    continue;
                }
                Op::Close(mode) => match wire {
                    Wire::Stream(io) => self.close(*io, *mode).await?,
                    // UDP has no close handshake, so every mode releases the socket.
                    Wire::Datagram(io) => io.close(),
                },
            }
            self.pc += 1;
        }
        Ok(())
    }

    /// Renders a template and writes it. Large `fill()` bodies go out as
    /// slices of the shared filler in one vectored write, so the session
    /// copies only the bytes around them.
    async fn send<S: SessionIo>(&mut self, io: &mut S, t: &Template) -> Result<(), FailureKind> {
        self.wbuf.clear();
        self.parts.clear();
        let mut parts = std::mem::take(&mut self.parts);
        let rendered = {
            let (mut ctx, out) = self.split();
            t.render_parts(&mut ctx, out, &mut parts, VECTORED_FILL_MIN)
        };
        self.parts = parts;
        rendered.map_err(|source| FailureKind::Render { source })?;
        let sent = if self.parts.iter().any(|p| matches!(p, Part::Filler { .. })) {
            write_parts(io, &self.wbuf, self.filler.as_bytes(), &self.parts).await
        } else {
            io.write_all(&self.wbuf).await.map(|()| self.wbuf.len())
        };
        let sent = sent.map_err(FailureKind::from_io)?;
        // A TLS layer buffers records, so the bytes leave only on flush.
        io.flush().await.map_err(FailureKind::from_io)?;
        self.stats.bytes_tx += sent as u64;
        Ok(())
    }

    async fn expect<S: SessionIo>(
        &mut self,
        io: &mut S,
        rule: &StopRule,
        timeout: Duration,
    ) -> Result<(), FailureKind> {
        let deadline = Instant::now() + timeout;
        let timeout_ms = millis(timeout);
        match rule {
            StopRule::Pattern(p) => {
                let finder = memchr::memmem::Finder::new(p);
                let mut from = 0;
                loop {
                    if let Some(i) = self.rbuf.get(from..).and_then(|b| finder.find(b)) {
                        let end = from + i + p.len();
                        self.consume_into_last(end)?;
                        return Ok(());
                    }
                    // A match may start in the last pattern length minus one bytes.
                    from = self.rbuf.len().saturating_sub(p.len().saturating_sub(1));
                    if self.rbuf.len() > self.limits.max_pattern_buffer {
                        return Err(FailureKind::PatternOverflow {
                            buffered: self.rbuf.len(),
                            limit: self.limits.max_pattern_buffer,
                        });
                    }
                    self.fill(io, deadline, timeout_ms).await?;
                }
            }
            StopRule::MinBytes(t) => {
                let want = self.render_count(t, "expect min_bytes")?;
                while self.rbuf.len() < want {
                    self.fill(io, deadline, timeout_ms).await?;
                }
                self.consume_into_last(self.rbuf.len())
            }
            StopRule::Bytes(t) => {
                let want = self.render_count(t, "expect bytes")?;
                if self.keep_last && want > self.limits.max_last {
                    return Err(FailureKind::Invalid(format!(
                        "expect bytes {want} exceeds the last limit of {} bytes",
                        self.limits.max_last
                    )));
                }
                let buffered = want.min(self.rbuf.len());
                self.consume_into_last(buffered)?;
                let mut remaining = want - buffered;
                if remaining > 0 && self.scratch.len() < DISCARD_CHUNK {
                    self.scratch.resize(DISCARD_CHUNK, 0);
                }
                while remaining > 0 {
                    let n = match timeout_at(deadline, io.read(&mut self.scratch)).await {
                        Err(_) => {
                            return Err(FailureKind::ExpectTimeout {
                                timeout_ms,
                                received: want - remaining,
                            })
                        }
                        Ok(Err(e)) => return Err(FailureKind::from_io(e)),
                        Ok(Ok(0)) => {
                            return Err(FailureKind::PeerClosed {
                                received: want - remaining,
                            })
                        }
                        Ok(Ok(n)) => n,
                    };
                    self.stats.bytes_rx += n as u64;
                    let used = n.min(remaining);
                    if self.keep_last {
                        self.last.extend_from_slice(&self.scratch[..used]);
                    }
                    if n > used {
                        // Bytes past the count stay for the next expect.
                        self.rbuf.extend_from_slice(&self.scratch[used..n]);
                    }
                    remaining -= used;
                }
                Ok(())
            }
        }
    }

    /// Renders a template and sends it as one datagram.
    async fn send_datagram<D: DatagramIo>(
        &mut self,
        io: &mut D,
        t: &Template,
    ) -> Result<(), FailureKind> {
        self.wbuf.clear();
        let (mut ctx, out) = self.split();
        t.render_into(&mut ctx, out)
            .map_err(|source| FailureKind::Render { source })?;
        if self.wbuf.len() > MAX_DATAGRAM {
            return Err(FailureKind::Invalid(format!(
                "send rendered {} bytes, above the datagram limit of {MAX_DATAGRAM} bytes",
                self.wbuf.len()
            )));
        }
        io.send_datagram(&self.wbuf)
            .await
            .map_err(FailureKind::from_datagram_io)?;
        self.stats.bytes_tx += self.wbuf.len() as u64;
        Ok(())
    }

    /// Reads exactly one datagram, checks it against the stop rule, and
    /// consumes all of it.
    async fn expect_datagram<D: DatagramIo>(
        &mut self,
        io: &mut D,
        rule: &StopRule,
        timeout: Duration,
    ) -> Result<(), FailureKind> {
        let deadline = Instant::now() + timeout;
        // Render the size before the read, so a render error does not eat a datagram.
        let want = match rule {
            StopRule::Pattern(_) => 0,
            StopRule::Bytes(t) => self.render_count(t, "expect bytes")?,
            StopRule::MinBytes(t) => self.render_count(t, "expect min_bytes")?,
        };
        let limit = self.max_datagram;
        if self.scratch.len() < limit {
            self.scratch.resize(limit.max(DISCARD_CHUNK), 0);
        }
        let Some(buf) = self.scratch.get_mut(..limit) else {
            return Err(FailureKind::Invalid(
                "invariant: the scratch buffer holds the datagram limit".into(),
            ));
        };
        let size = match timeout_at(deadline, io.recv_datagram(buf)).await {
            Err(_) => {
                return Err(FailureKind::ExpectTimeout {
                    timeout_ms: millis(timeout),
                    received: 0,
                })
            }
            Ok(Err(e)) => return Err(FailureKind::from_datagram_io(e)),
            Ok(Ok(n)) => n,
        };
        self.stats.bytes_rx += size as u64;
        if size > limit {
            return Err(FailureKind::DatagramTooLarge { size, limit });
        }
        let datagram = self.scratch.get(..size).unwrap_or_default();
        let matched = match rule {
            StopRule::Pattern(p) => memchr::memmem::find(datagram, p).is_some(),
            StopRule::Bytes(_) => size == want,
            StopRule::MinBytes(_) => size >= want,
        };
        if !matched {
            let rule = match rule {
                StopRule::Pattern(p) => format!("pattern {:?}", String::from_utf8_lossy(p)),
                StopRule::Bytes(_) => format!("bytes {want}"),
                StopRule::MinBytes(_) => format!("min_bytes {want}"),
            };
            return Err(FailureKind::DatagramMismatch { size, rule });
        }
        if self.keep_last {
            if size > self.limits.max_last {
                return Err(FailureKind::Invalid(format!(
                    "expect consumed {size} bytes, above the last limit of {} bytes",
                    self.limits.max_last
                )));
            }
            self.last.clear();
            self.last.extend_from_slice(datagram);
        }
        Ok(())
    }

    /// Consumes `n` buffered bytes, which become `last` when a template of
    /// the program refers to it.
    fn consume_into_last(&mut self, n: usize) -> Result<(), FailureKind> {
        if self.keep_last {
            if n > self.limits.max_last {
                return Err(FailureKind::Invalid(format!(
                    "expect consumed {n} bytes, above the last limit of {} bytes",
                    self.limits.max_last
                )));
            }
            self.last.clear();
            self.last.extend_from_slice(&self.rbuf[..n]);
        }
        self.rbuf.advance(n);
        Ok(())
    }

    /// Reads once into the read buffer before `deadline`.
    async fn fill<S: SessionIo>(
        &mut self,
        io: &mut S,
        deadline: Instant,
        timeout_ms: u64,
    ) -> Result<(), FailureKind> {
        if self.rbuf.capacity() - self.rbuf.len() < READ_CHUNK / 4 {
            self.rbuf.reserve(READ_CHUNK);
        }
        let received = self.rbuf.len();
        match timeout_at(deadline, io.read_buf(&mut self.rbuf)).await {
            Err(_) => Err(FailureKind::ExpectTimeout {
                timeout_ms,
                received,
            }),
            Ok(Err(e)) => Err(FailureKind::from_io(e)),
            Ok(Ok(0)) => Err(FailureKind::PeerClosed { received }),
            Ok(Ok(n)) => {
                self.stats.bytes_rx += n as u64;
                Ok(())
            }
        }
    }

    async fn close<S: SessionIo>(
        &mut self,
        io: &mut S,
        mode: CloseMode,
    ) -> Result<(), FailureKind> {
        match mode {
            CloseMode::Rst => {
                io.abort();
                Ok(())
            }
            CloseMode::Half => io.shutdown().await.map_err(FailureKind::from_io),
            CloseMode::Fin => {
                io.shutdown().await.map_err(FailureKind::from_io)?;
                // Drain until the peer closes, so neither side leaves data unread.
                self.rbuf.clear();
                if self.scratch.len() < DISCARD_CHUNK {
                    self.scratch.resize(DISCARD_CHUNK, 0);
                }
                let deadline = Instant::now() + self.limits.drain_timeout;
                loop {
                    match timeout_at(deadline, io.read(&mut self.scratch)).await {
                        Err(_) => {
                            return Err(FailureKind::DrainTimeout {
                                timeout_ms: millis(self.limits.drain_timeout),
                            })
                        }
                        Ok(Ok(0)) => return Ok(()),
                        Ok(Ok(n)) => self.stats.bytes_rx += n as u64,
                        Ok(Err(e)) => match FailureKind::from_io(e) {
                            // A reset after our FIN still ends the session cleanly.
                            FailureKind::Reset { .. } => return Ok(()),
                            other => return Err(other),
                        },
                    }
                }
            }
        }
    }

    /// Splits the session into a render context and the write buffer.
    fn split(&mut self) -> (RenderCtx<'_, Pcg64Mcg>, &mut Vec<u8>) {
        let Session {
            vars,
            filler,
            flow,
            session_id,
            rng,
            functions,
            overlay,
            last,
            wbuf,
            ..
        } = self;
        (
            RenderCtx {
                vars,
                overlay,
                session_id: *session_id,
                flow,
                last,
                rng,
                filler,
                functions: &mut **functions,
            },
            wbuf,
        )
    }

    fn render_int(&mut self, t: &Template) -> Result<i64, FailureKind> {
        let (mut ctx, _) = self.split();
        t.render_int(&mut ctx)
            .map_err(|source| FailureKind::Render { source })
    }

    fn render_count(&mut self, t: &Template, what: &str) -> Result<usize, FailureKind> {
        let n = self.render_int(t)?;
        usize::try_from(n).map_err(|_| FailureKind::Invalid(format!("{what} is negative, got {n}")))
    }

    fn open_failure(&self, kind: FailureKind) -> Failure {
        Failure {
            flow: self.flow.to_string(),
            side: self.side,
            action_index: 0,
            action_type: self.side.opener(),
            reason: kind.reason(),
            kind: Box::new(kind),
        }
    }

    fn failure(&self, ops: &[Op], kind: FailureKind) -> Failure {
        let (action_index, action_type) = locate(ops, self.pc);
        Failure {
            flow: self.flow.to_string(),
            side: self.side,
            action_index,
            action_type,
            reason: kind.reason(),
            kind: Box::new(kind),
        }
    }
}

/// The transport of a running side, a byte stream or a datagram socket.
enum Wire<'w, S, D> {
    Stream(&'w mut S),
    Datagram(&'w mut D),
}

/// The stream type of a UDP side, which has no stream.
enum NoStream {}

impl AsyncRead for NoStream {
    fn poll_read(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        _buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        match *self.get_mut() {}
    }
}

impl AsyncWrite for NoStream {
    fn poll_write(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        _data: &[u8],
    ) -> Poll<io::Result<usize>> {
        match *self.get_mut() {}
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match *self.get_mut() {}
    }

    fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match *self.get_mut() {}
    }
}

impl SessionIo for NoStream {
    fn abort(&mut self) {
        match *self {}
    }
}

/// The datagram type of a TCP side, which has no datagram socket.
enum NoDatagram {}

impl DatagramIo for NoDatagram {
    fn poll_send(&mut self, _cx: &mut Context<'_>, _datagram: &[u8]) -> Poll<io::Result<()>> {
        match *self {}
    }

    fn poll_recv(&mut self, _cx: &mut Context<'_>, _buf: &mut [u8]) -> Poll<io::Result<usize>> {
        match *self {}
    }

    fn close(&mut self) {
        match *self {}
    }
}

/// Returns the bytes of `part`, which index `out` or the filler `filler`.
fn part_bytes<'b>(part: &Part, out: &'b [u8], filler: &'b [u8]) -> io::Result<&'b [u8]> {
    let (buf, start, end) = match *part {
        Part::Out { start, end } => (out, start, end),
        Part::Filler { start, end } => (filler, start, end),
    };
    buf.get(start..end).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("invariant: render part {start}..{end} lies outside its buffer"),
        )
    })
}

/// Writes every part in order and returns the number of bytes written.
///
/// A transport with vectored writes receives up to [`MAX_SLICES`] parts per
/// call, and other transports receive one part per call, so neither path
/// copies the filler slices.
async fn write_parts<W: AsyncWrite + Unpin>(
    io: &mut W,
    out: &[u8],
    filler: &[u8],
    parts: &[Part],
) -> io::Result<usize> {
    let mut total = 0;
    if !io.is_write_vectored() {
        for part in parts {
            let bytes = part_bytes(part, out, filler)?;
            io.write_all(bytes).await?;
            total += bytes.len();
        }
        return Ok(total);
    }
    // The next part to write and the bytes of it already written.
    let (mut next, mut skip) = (0, 0);
    while next < parts.len() {
        let mut slices = [IoSlice::new(&[]); MAX_SLICES];
        let mut count = 0;
        for (i, part) in parts.iter().enumerate().skip(next).take(MAX_SLICES) {
            let bytes = part_bytes(part, out, filler)?;
            let bytes = if i == next {
                bytes.get(skip..).unwrap_or_default()
            } else {
                bytes
            };
            slices[count] = IoSlice::new(bytes);
            count += 1;
        }
        let mut n = io.write_vectored(&slices[..count]).await?;
        if n == 0 {
            return Err(io::ErrorKind::WriteZero.into());
        }
        total += n;
        // Advance past the parts that the write finished.
        while n > 0 && next < parts.len() {
            let left = part_bytes(&parts[next], out, filler)?.len() - skip;
            if n < left {
                skip += n;
                n = 0;
            } else {
                n -= left;
                next += 1;
                skip = 0;
            }
        }
    }
    Ok(total)
}

/// Runs `open` under `timeout` and returns `None` when `stop` completes first.
async fn race<T, F: Future<Output = ()>>(
    stop: Pin<&mut F>,
    timeout: Duration,
    open: impl Future<Output = io::Result<T>>,
) -> Option<Result<io::Result<T>, Elapsed>> {
    let open = tokio::time::timeout(timeout, open);
    tokio::pin!(open);
    tokio::select! {
        biased;
        () = stop => None,
        r = &mut open => Some(r),
    }
}

/// Maps an op index to the catalog action index and action type.
///
/// The compiled program omits the opening action and adds `LoopEnd` and
/// `Jump` ops, which are not actions. The action index therefore counts the
/// ops before `pc` that are actions and adds one for the opening action.
fn locate(ops: &[Op], pc: usize) -> (usize, &'static str) {
    let pc = match ops.get(pc) {
        Some(Op::LoopEnd { start }) => *start,
        _ => pc,
    };
    let index = 1 + ops
        .iter()
        .take(pc)
        .filter(|op| !matches!(op, Op::LoopEnd { .. } | Op::Jump(_)))
        .count();
    let kind = match ops.get(pc) {
        Some(Op::Send(_)) => "send",
        Some(Op::Expect { .. }) => "expect",
        Some(Op::Delay(_)) => "delay",
        Some(Op::Set(_)) => "set",
        Some(Op::LoopStart { .. } | Op::LoopEnd { .. }) => "loop",
        Some(Op::Branch { .. } | Op::Jump(_)) => "branch",
        Some(Op::Close(_)) => "close",
        // A failure after the last op, such as an abort at the end.
        None => "close",
    };
    (index, kind)
}

fn op_uses_last(op: &Op) -> bool {
    match op {
        Op::Send(t)
        | Op::Expect {
            rule: StopRule::Bytes(t) | StopRule::MinBytes(t),
            ..
        }
        | Op::Delay(fps_flow::program::Delay::Fixed(t))
        | Op::LoopStart { count: t, .. } => t.uses_last(),
        Op::Set(binds) => binds.iter().any(|(_, t)| t.uses_last()),
        _ => false,
    }
}

fn millis(d: Duration) -> u64 {
    u64::try_from(d.as_millis()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use fps_flow::Scenario;

    use super::*;

    #[test]
    fn locate_counts_actions_in_document_order() {
        let s = Scenario::parse(
            r#"
scenario: t
flows:
  - id: a
    module: m
    client:
      - connect: { port: 1 }
      - loop:
          count: 2
          do:
            - send: "x"
            - branch:
                - { p: 0.5, do: [ { delay: { ms: 1 } } ] }
                - { p: 0.5, do: [ { send: "y" } ] }
      - close: fin
"#,
        )
        .unwrap();
        let ops = &s.flows[0].client.ops;
        // connect 0, loop 1, send 2, branch 3, delay 4, send 5, close 6
        assert_eq!(locate(ops, 0), (1, "loop"));
        assert_eq!(locate(ops, 1), (2, "send"));
        assert_eq!(locate(ops, 2), (3, "branch"));
        assert_eq!(locate(ops, 3), (4, "delay"));
        assert_eq!(locate(ops, 5), (5, "send"));
        assert_eq!(locate(ops, 7), (1, "loop"));
        assert_eq!(locate(ops, 8), (6, "close"));
    }
}
