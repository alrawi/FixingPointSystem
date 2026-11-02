//! Flow program interpreter. It runs one side of one session over any
//! byte stream, which lets unit tests use in memory pipes.

use std::time::{Duration, Instant};

use bytes::{Buf, BytesMut};
use rand::rngs::SmallRng;
use rand::Rng;
use rand_distr::{Distribution, Exp, LogNormal, Pareto};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpStream;

use crate::program::{CloseMode, Delay, Op, StopRule};
use crate::template::{Filler, RenderCtx, Template, Vars};

/// Largest amount of unmatched data an `expect pattern` may buffer.
pub const MAX_PATTERN_BUFFER: usize = 16 << 20;
/// Growth step of the pattern buffer.
pub const READ_CHUNK: usize = 16 * 1024;
/// Scratch size for `expect bytes`, which discards payload without buffering it.
pub const DISCARD_CHUNK: usize = 64 * 1024;
const DRAIN_TIMEOUT: Duration = Duration::from_secs(10);

/// A session transport. `abort` turns the next close into a reset.
pub trait SessionIo: AsyncRead + AsyncWrite + Unpin + Send {
    fn abort(&mut self) {}
}

impl SessionIo for TcpStream {
    fn abort(&mut self) {
        // SO_LINGER with a zero timeout makes close() send RST.
        #[allow(deprecated)]
        let _ = self.set_linger(Some(Duration::ZERO));
    }
}

impl SessionIo for tokio::io::DuplexStream {}

/// Why a session failed. The label groups failures in statistics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fail {
    Timeout { op: usize },
    Eof { op: usize },
    Io { op: usize, kind: std::io::ErrorKind },
    Render { op: usize, msg: String },
    Overflow { op: usize },
}

impl Fail {
    pub fn label(&self) -> String {
        match self {
            Fail::Timeout { .. } => "timeout".into(),
            Fail::Eof { .. } => "unexpected_eof".into(),
            Fail::Io { kind, .. } => format!("io_{kind:?}").to_lowercase(),
            Fail::Render { .. } => "render".into(),
            Fail::Overflow { .. } => "buffer_overflow".into(),
        }
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct SideStats {
    pub bytes_tx: u64,
    pub bytes_rx: u64,
}

/// Shared, read only state for every session of a scenario.
pub struct Shared<'a> {
    pub vars: &'a Vars,
    pub filler: &'a Filler,
}

pub struct Session<'a, S: SessionIo> {
    io: S,
    rbuf: BytesMut,
    wbuf: Vec<u8>,
    scratch: Vec<u8>,
    overlay: Vars,
    shared: Shared<'a>,
    session_id: u64,
    flow: &'a str,
    rng: SmallRng,
    pub stats: SideStats,
}

impl<'a, S: SessionIo> Session<'a, S> {
    pub fn new(io: S, shared: Shared<'a>, session_id: u64, flow: &'a str, rng: SmallRng) -> Self {
        Session {
            io,
            rbuf: BytesMut::with_capacity(READ_CHUNK),
            wbuf: Vec::with_capacity(16 * 1024),
            scratch: Vec::new(),
            overlay: Vars::new(),
            shared,
            session_id,
            flow,
            rng,
            stats: SideStats::default(),
        }
    }

    fn render(&mut self, t: &Template, op: usize, into_wbuf: bool) -> Result<Option<i64>, Fail> {
        let mut ctx = RenderCtx {
            vars: self.shared.vars,
            overlay: &self.overlay,
            session_id: self.session_id,
            flow: self.flow,
            rng: &mut self.rng,
            filler: self.shared.filler,
        };
        let res = if into_wbuf {
            self.wbuf.clear();
            t.render_into(&mut ctx, &mut self.wbuf).map(|_| None)
        } else {
            t.render_int(&mut ctx).map(Some)
        };
        res.map_err(|e| Fail::Render {
            op,
            msg: e.to_string(),
        })
    }

    fn int(&mut self, t: &Template, op: usize) -> Result<i64, Fail> {
        Ok(self.render(t, op, false)?.unwrap_or(0))
    }

    async fn fill(&mut self, op: usize, deadline: Instant) -> Result<(), Fail> {
        let now = Instant::now();
        if now >= deadline {
            return Err(Fail::Timeout { op });
        }
        match tokio::time::timeout(deadline - now, self.io.read_buf(&mut self.rbuf)).await {
            Err(_) => Err(Fail::Timeout { op }),
            Ok(Err(e)) => Err(Fail::Io { op, kind: e.kind() }),
            Ok(Ok(0)) => Err(Fail::Eof { op }),
            Ok(Ok(n)) => {
                self.stats.bytes_rx += n as u64;
                if self.rbuf.capacity() - self.rbuf.len() < READ_CHUNK / 4 {
                    self.rbuf.reserve(READ_CHUNK);
                }
                Ok(())
            }
        }
    }

    async fn expect(&mut self, rule: &StopRule, timeout: Duration, op: usize) -> Result<(), Fail> {
        let deadline = Instant::now() + timeout;
        match rule {
            StopRule::Pattern(p) => {
                let finder = memchr::memmem::Finder::new(p);
                let mut from = 0;
                loop {
                    if let Some(i) = finder.find(&self.rbuf[from..]) {
                        self.rbuf.advance(from + i + p.len());
                        return Ok(());
                    }
                    from = self.rbuf.len().saturating_sub(p.len() - 1);
                    if self.rbuf.len() > MAX_PATTERN_BUFFER {
                        return Err(Fail::Overflow { op });
                    }
                    self.fill(op, deadline).await?;
                }
            }
            StopRule::Bytes(t) => {
                let mut remaining = self.int(t, op)?.max(0) as usize;
                let buffered = remaining.min(self.rbuf.len());
                self.rbuf.advance(buffered);
                remaining -= buffered;
                if remaining == 0 {
                    return Ok(());
                }
                // Discard the rest through a reused scratch buffer, so bulk
                // bodies never grow the pattern buffer.
                if self.scratch.is_empty() {
                    self.scratch = vec![0; DISCARD_CHUNK];
                }
                while remaining > 0 {
                    let now = Instant::now();
                    if now >= deadline {
                        return Err(Fail::Timeout { op });
                    }
                    let n =
                        match tokio::time::timeout(deadline - now, self.io.read(&mut self.scratch))
                            .await
                        {
                            Err(_) => return Err(Fail::Timeout { op }),
                            Ok(Err(e)) => return Err(Fail::Io { op, kind: e.kind() }),
                            Ok(Ok(0)) => return Err(Fail::Eof { op }),
                            Ok(Ok(n)) => n,
                        };
                    self.stats.bytes_rx += n as u64;
                    if n > remaining {
                        self.rbuf.extend_from_slice(&self.scratch[remaining..n]);
                        remaining = 0;
                    } else {
                        remaining -= n;
                    }
                }
                Ok(())
            }
            StopRule::MinBytes(t) => {
                let want = self.int(t, op)?.max(0) as usize;
                while self.rbuf.len() < want {
                    self.fill(op, deadline).await?;
                }
                self.rbuf.clear();
                Ok(())
            }
        }
    }

    fn sample_delay(&mut self, d: &Delay, op: usize) -> Result<Duration, Fail> {
        let ms = match d {
            Delay::Fixed(t) => self.int(t, op)?.max(0) as f64,
            Delay::Exponential { mean_ms } => Exp::new(1.0 / mean_ms)
                .map(|e| e.sample(&mut self.rng))
                .unwrap_or(0.0),
            Delay::Uniform { min_ms, max_ms } => {
                if max_ms > min_ms {
                    self.rng.gen_range(*min_ms..*max_ms)
                } else {
                    *min_ms
                }
            }
            Delay::LogNormal { mu, sigma, unit_ms } => LogNormal::new(*mu, *sigma)
                .map(|l| l.sample(&mut self.rng) * unit_ms)
                .unwrap_or(0.0),
            Delay::Pareto { alpha, scale_ms } => Pareto::new(*scale_ms, *alpha)
                .map(|p| p.sample(&mut self.rng))
                .unwrap_or(*scale_ms),
        };
        // Cap a single think time at one hour to keep runaway samples bounded.
        Ok(Duration::from_secs_f64((ms / 1000.0).clamp(0.0, 3600.0)))
    }

    async fn close(&mut self, mode: CloseMode, op: usize) -> Result<(), Fail> {
        match mode {
            CloseMode::Rst => {
                self.io.abort();
                Ok(())
            }
            CloseMode::Half => self
                .io
                .shutdown()
                .await
                .map_err(|e| Fail::Io { op, kind: e.kind() }),
            CloseMode::Fin => {
                self.io
                    .shutdown()
                    .await
                    .map_err(|e| Fail::Io { op, kind: e.kind() })?;
                // Drain until the peer closes, so neither side leaves data unread.
                let deadline = Instant::now() + DRAIN_TIMEOUT;
                loop {
                    self.rbuf.clear();
                    match self.fill(op, deadline).await {
                        Ok(()) => continue,
                        Err(Fail::Eof { .. }) => return Ok(()),
                        // A reset after our FIN still ends the session cleanly.
                        Err(Fail::Io {
                            kind: std::io::ErrorKind::ConnectionReset,
                            ..
                        }) => return Ok(()),
                        Err(e) => return Err(e),
                    }
                }
            }
        }
    }

    /// Run the body of one side. Returns when the ops are exhausted.
    pub async fn run(&mut self, ops: &[Op]) -> Result<(), Fail> {
        let mut pc = 0;
        let mut counters: Vec<i64> = Vec::new();
        while pc < ops.len() {
            match &ops[pc] {
                Op::Send(t) => {
                    self.render(t, pc, true)?;
                    self.io.write_all(&self.wbuf).await.map_err(|e| Fail::Io {
                        op: pc,
                        kind: e.kind(),
                    })?;
                    self.stats.bytes_tx += self.wbuf.len() as u64;
                }
                Op::Expect { rule, timeout } => self.expect(rule, *timeout, pc).await?,
                Op::Delay(d) => {
                    let dur = self.sample_delay(d, pc)?;
                    if !dur.is_zero() {
                        tokio::time::sleep(dur).await;
                    }
                }
                Op::Set(binds) => {
                    for (name, t) in binds {
                        self.render(t, pc, true)?;
                        let text = String::from_utf8_lossy(&self.wbuf).into_owned();
                        self.overlay
                            .insert(name.clone(), crate::template::Value::parse_override(&text));
                    }
                }
                Op::LoopStart { count, end } => {
                    let n = self.int(count, pc)?;
                    if n <= 0 {
                        pc = end + 1;
                        continue;
                    }
                    counters.push(n);
                }
                Op::LoopEnd { start } => {
                    let top = counters.last_mut().expect("loop counter");
                    *top -= 1;
                    if *top > 0 {
                        pc = start + 1;
                        continue;
                    }
                    counters.pop();
                }
                Op::Branch { arms } => {
                    let x: f64 = self.rng.gen();
                    let target = arms
                        .iter()
                        .find(|(c, _)| x < *c)
                        .or(arms.last())
                        .map(|a| a.1)
                        .unwrap_or(pc + 1);
                    pc = target;
                    continue;
                }
                Op::Jump(to) => {
                    pc = *to;
                    continue;
                }
                Op::Close(mode) => self.close(*mode, pc).await?,
            }
            pc += 1;
        }
        Ok(())
    }

    pub fn into_io(self) -> S {
        self.io
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::program::Scenario;
    use rand::SeedableRng;

    const PINGPONG: &str = r#"
scenario: pingpong
vars: { n: 3, size: 70000 }
flows:
  - id: p
    module: m
    client:
      - connect: { port: 1 }
      - set: { tag: "{{ rand_int(5, 5) }}" }
      - loop:
          count: "{{ vars.n }}"
          do:
            - send: "REQ {{ vars.tag }}\r\n\r\n"
            - expect: { pattern: "\r\n\r\n", timeout_ms: 2000 }
            - expect: { bytes: "{{ vars.size }}", timeout_ms: 2000 }
      - delay: { dist: uniform, min_ms: 1, max_ms: 2 }
      - close: fin
    server:
      - accept: { port: 1 }
      - loop:
          count: "{{ vars.n }}"
          do:
            - expect: { pattern: "\r\n\r\n", timeout_ms: 2000 }
            - send: "RESP\r\n\r\n{{ fill(vars.size) }}"
      - close: fin
"#;

    async fn run_pair(text: &str) -> (Result<(), Fail>, Result<(), Fail>, SideStats, SideStats) {
        let s = Scenario::parse(text).unwrap();
        let filler = Filler::new(3);
        let (a, b) = tokio::io::duplex(8192);
        let flow = &s.flows[0];
        let mut c = Session::new(
            a,
            Shared {
                vars: &s.vars,
                filler: &filler,
            },
            1,
            &flow.id,
            SmallRng::seed_from_u64(1),
        );
        let mut sv = Session::new(
            b,
            Shared {
                vars: &s.vars,
                filler: &filler,
            },
            1,
            &flow.id,
            SmallRng::seed_from_u64(2),
        );
        let server_ops = &flow.server.as_ref().unwrap().ops;
        let (rc, rs) = tokio::join!(c.run(&flow.client.ops), sv.run(server_ops));
        (rc, rs, c.stats, sv.stats)
    }

    #[tokio::test]
    async fn client_and_server_complete() {
        let (rc, rs, cs, ss) = run_pair(PINGPONG).await;
        assert_eq!(rc, Ok(()));
        assert_eq!(rs, Ok(()));
        assert_eq!(cs.bytes_tx, 3 * "REQ 5\r\n\r\n".len() as u64);
        assert_eq!(cs.bytes_rx, 3 * (70000 + 8));
        assert_eq!(ss.bytes_rx, cs.bytes_tx);
    }

    #[tokio::test]
    async fn expect_times_out() {
        let text = r#"
scenario: t
flows:
  - id: p
    module: m
    client:
      - connect: { port: 1 }
      - expect: { pattern: "never", timeout_ms: 50 }
    server:
      - accept: { port: 1 }
      - delay: { ms: 300 }
"#;
        let (rc, _, _, _) = run_pair(text).await;
        assert_eq!(rc, Err(Fail::Timeout { op: 0 }));
    }

    #[tokio::test]
    async fn early_close_is_eof() {
        let text = r#"
scenario: t
flows:
  - id: p
    module: m
    client:
      - connect: { port: 1 }
      - expect: { bytes: 10, timeout_ms: 1000 }
    server:
      - accept: { port: 1 }
      - send: "short"
      - close: half
"#;
        let (rc, rs, _, _) = run_pair(text).await;
        assert_eq!(rs, Ok(()));
        assert_eq!(rc, Err(Fail::Eof { op: 0 }));
    }

    #[tokio::test]
    async fn pattern_split_across_reads() {
        // A 1 byte pipe forces the pattern to arrive one byte at a time.
        let s = Scenario::parse(PINGPONG).unwrap();
        let filler = Filler::new(3);
        let (a, b) = tokio::io::duplex(1);
        let flow = &s.flows[0];
        let mut c = Session::new(
            a,
            Shared {
                vars: &s.vars,
                filler: &filler,
            },
            1,
            &flow.id,
            SmallRng::seed_from_u64(1),
        );
        let mut sv = Session::new(
            b,
            Shared {
                vars: &s.vars,
                filler: &filler,
            },
            1,
            &flow.id,
            SmallRng::seed_from_u64(2),
        );
        let (rc, rs) = tokio::join!(
            c.run(&flow.client.ops),
            sv.run(&flow.server.as_ref().unwrap().ops)
        );
        assert_eq!((rc, rs), (Ok(()), Ok(())));
    }
}
