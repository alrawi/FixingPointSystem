// SPDX-License-Identifier: GPL-3.0-only
//! Vectored `send` of large `fill()` bodies, which writes slices of the
//! shared filler in place of a copy and must put the same bytes on the wire
//! as a copying render.

mod common;

use std::io::{self, IoSlice};
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

use common::{compile, run_with, OneShot, Opts};
use fps_flow::function::NoFunctions;
use fps_flow::template::{Filler, RenderCtx};
use fps_flow::Scenario;
use fps_interp::{session_rng, Side, VECTORED_FILL_MIN};
use fps_io::memory::duplex;
use fps_io::SessionIo;
use proptest::prelude::*;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

/// What a [`Vectored`] transport saw.
#[derive(Debug, Default)]
struct Seen {
    bytes: Vec<u8>,
    vectored_calls: usize,
    slices: Vec<usize>,
}

/// A transport that accepts vectored writes of at most `limit` bytes per
/// call and records every byte and call.
struct Vectored<S> {
    inner: S,
    limit: usize,
    seen: Arc<Mutex<Seen>>,
    gather: Vec<u8>,
}

impl<S> Vectored<S> {
    fn new(inner: S, limit: usize) -> (Self, Arc<Mutex<Seen>>) {
        let seen = Arc::new(Mutex::new(Seen::default()));
        let io = Vectored {
            inner,
            limit,
            seen: Arc::clone(&seen),
            gather: Vec::new(),
        };
        (io, seen)
    }
}

impl<S: SessionIo> AsyncRead for Vectored<S> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_read(cx, buf)
    }
}

impl<S: SessionIo> AsyncWrite for Vectored<S> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        data: &[u8],
    ) -> Poll<io::Result<usize>> {
        self.poll_write_vectored(cx, &[IoSlice::new(data)])
    }

    fn poll_write_vectored(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        this.gather.clear();
        for b in bufs {
            let room = this.limit - this.gather.len();
            this.gather.extend_from_slice(&b[..b.len().min(room)]);
            if this.gather.len() == this.limit {
                break;
            }
        }
        let r = Pin::new(&mut this.inner).poll_write(cx, &this.gather);
        if let Poll::Ready(Ok(n)) = r {
            let mut seen = this.seen.lock().unwrap();
            seen.bytes.extend_from_slice(&this.gather[..n]);
            seen.vectored_calls += 1;
            seen.slices.push(bufs.len());
        }
        r
    }

    fn is_write_vectored(&self) -> bool {
        true
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_shutdown(cx)
    }
}

impl<S: SessionIo> SessionIo for Vectored<S> {
    fn abort(&mut self) {
        self.inner.abort();
    }
}

const BULK: &str = r#"
scenario: bulk
vars: { n: 2, size: 200000, port: 8080 }
flows:
  - id: b
    module: m
    client:
      - connect: { port: "{{ vars.port }}" }
      - loop:
          count: "{{ vars.n }}"
          do:
            - send: "GET\r\n\r\n"
            - expect: { pattern: "\r\n\r\n", timeout_ms: 20000 }
            - expect: { bytes: "{{ vars.size }}", timeout_ms: 20000 }
      - close: fin
    server:
      - accept: { port: "{{ vars.port }}" }
      - loop:
          count: "{{ vars.n }}"
          do:
            - expect: { pattern: "\r\n\r\n", timeout_ms: 20000 }
            - send: "HTTP/1.1 200 OK\r\nContent-Length: {{ vars.size }}\r\n\r\n{{ fill(vars.size) }}"
      - close: fin
"#;

/// Renders the server responses of `s` with the copying `render_into`, in
/// the order the server sends them.
fn expected_responses(s: &Scenario, seed: u64, session_id: u64, n: usize) -> Vec<u8> {
    let filler = Filler::new(seed);
    let mut rng = session_rng(seed, session_id, Side::Server);
    let mut none = NoFunctions;
    let overlay = fps_flow::Vars::new();
    let send = s.flows[0]
        .server
        .as_ref()
        .unwrap()
        .ops
        .iter()
        .find_map(|op| match op {
            fps_flow::program::Op::Send(t) => Some(t.clone()),
            _ => None,
        })
        .unwrap();
    let mut out = Vec::new();
    for _ in 0..n {
        let mut ctx = RenderCtx {
            vars: &s.vars,
            overlay: &overlay,
            session_id,
            flow: &s.flows[0].id,
            last: &[],
            rng: &mut rng,
            filler: &filler,
            functions: &mut none,
            clock: &fps_flow::SystemClock,
        };
        send.render_into(&mut ctx, &mut out).unwrap();
    }
    out
}

/// Runs the bulk flow with the server behind a [`Vectored`] transport and
/// returns what that transport saw and the expected bytes.
async fn run_bulk(size: usize, n: usize, limit: usize, seed: u64) -> (Seen, Vec<u8>, u64) {
    let text = BULK
        .replace("size: 200000", &format!("size: {size}"))
        .replace("n: 2,", &format!("n: {n},"));
    let s = compile(&text);
    let (a, b) = duplex(1 << 20);
    let (b, seen) = Vectored::new(b, limit);
    let opts = Opts {
        seed,
        ..Opts::default()
    };
    let session_id = opts.session_id;
    let pair = run_with(&s, &mut OneShot::new(a), &mut OneShot::new(b), opts).await;
    pair.client.unwrap();
    pair.server.unwrap();
    let expected = expected_responses(&s, seed, session_id, n);
    let seen = std::mem::take(&mut *seen.lock().unwrap());
    (seen, expected, pair.server_stats.bytes_tx)
}

#[tokio::test]
async fn a_large_fill_body_goes_out_as_slices_with_the_copying_bytes() {
    let (seen, expected, bytes_tx) = run_bulk(200_000, 2, usize::MAX, 42).await;
    assert_eq!(seen.bytes, expected);
    assert_eq!(bytes_tx, expected.len() as u64);
    // The header and the body, and a second filler slice when the body wraps.
    assert!(
        seen.slices.iter().all(|&n| (2..=3).contains(&n)),
        "{seen:?}"
    );
    assert_eq!(seen.vectored_calls, 2, "one write per response");
}

#[tokio::test]
async fn partial_vectored_writes_resume_inside_a_slice() {
    let (seen, expected, _) = run_bulk(200_000, 2, 1000, 7).await;
    assert_eq!(seen.bytes, expected);
    assert!(seen.vectored_calls >= 400);
}

#[tokio::test]
async fn a_body_above_sixteen_filler_lengths_spans_several_writes() {
    // 20 MiB wraps the 1 MiB filler into 21 slices, more than one write takes.
    let size = 20 * Filler::SIZE;
    let (seen, expected, _) = run_bulk(size, 1, usize::MAX, 3).await;
    assert_eq!(seen.bytes.len(), expected.len());
    assert!(seen.bytes == expected, "bytes differ");
    assert_eq!(seen.slices.first(), Some(&16));
    assert!(seen.vectored_calls >= 2);
}

#[tokio::test]
async fn a_small_fill_body_is_copied_into_one_write() {
    let (seen, expected, _) = run_bulk(VECTORED_FILL_MIN - 1, 3, usize::MAX, 42).await;
    assert_eq!(seen.bytes, expected);
    assert_eq!(seen.slices, vec![1, 1, 1]);
}

#[tokio::test]
async fn a_transport_without_vectored_writes_gets_the_same_bytes() {
    let text = BULK.replace("size: 200000", "size: 1500000");
    let s = compile(&text);
    let (a, b) = duplex(1 << 16);
    let (b, written) = common::Tap::new(b);
    let opts = Opts::default();
    let (seed, session_id) = (opts.seed, opts.session_id);
    let pair = run_with(&s, &mut OneShot::new(a), &mut OneShot::new(b), opts).await;
    pair.client.unwrap();
    pair.server.unwrap();
    let written = written.lock().unwrap().clone();
    assert!(written == expected_responses(&s, seed, session_id, 2));
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(12))]

    #[test]
    fn vectored_send_matches_the_copying_render(
        size in 0usize..(3 * Filler::SIZE),
        limit in 1usize..300_000,
        seed in any::<u64>(),
    ) {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let (seen, expected, bytes_tx) = rt.block_on(run_bulk(size, 1, limit, seed));
        prop_assert_eq!(seen.bytes.len(), expected.len());
        prop_assert!(seen.bytes == expected);
        prop_assert_eq!(bytes_tx, expected.len() as u64);
    }
}
