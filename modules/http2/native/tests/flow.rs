// SPDX-License-Identifier: GPL-3.0-only
//! Runs both sides of a scenario through the interpreter with the http2
//! functions, the way the agent runs a module scenario. The scenario in
//! `tests/data/h2c_flow.yaml` shows the idiom that a flow uses to read frames,
//! and the test checks the bytes that each side wrote with an independent
//! reading of the frames.

use std::future::Future;
use std::io;
use std::path::Path;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

use fps_flow::template::Filler;
use fps_flow::{FnCall, FnError, Manifest, ModuleSet, Scenario, TemplateFn};
use fps_interp::{session_rng, Limits, OpenRequest, Opener, Session, SessionCtx, Side};
use fps_io::memory::duplex;
use fps_io::SessionIo;
use fps_module_http2::catalog::manifest_functions;
use fps_module_http2::frame::{self, flag, ty, PREFACE};
use fps_module_http2::hpack::Decoder;
use fps_module_http2::Http2;
use rand::RngCore;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

struct Tap<S> {
    inner: S,
    written: Arc<Mutex<Vec<u8>>>,
}

impl<S: SessionIo> AsyncRead for Tap<S> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_read(cx, buf)
    }
}

impl<S: SessionIo> AsyncWrite for Tap<S> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        data: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        let r = Pin::new(&mut this.inner).poll_write(cx, data);
        if let Poll::Ready(Ok(n)) = r {
            this.written.lock().unwrap().extend_from_slice(&data[..n]);
        }
        r
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_shutdown(cx)
    }
}

impl<S: SessionIo> SessionIo for Tap<S> {
    fn abort(&mut self) {
        self.inner.abort();
    }
}

struct OneShot<S>(Option<S>);

impl<S: SessionIo> Opener for OneShot<S> {
    type Io = S;

    fn open(&mut self, _request: OpenRequest<'_>) -> impl Future<Output = io::Result<S>> {
        let io = self.0.take();
        async move { io.ok_or_else(|| io::Error::new(io::ErrorKind::NotConnected, "used")) }
    }
}

/// The module state of one side of the flow.
#[derive(Default)]
struct Fns(Http2);

impl TemplateFn for Fns {
    fn render(
        &mut self,
        call: &FnCall<'_>,
        rng: &mut dyn RngCore,
        out: &mut Vec<u8>,
    ) -> Result<(), FnError> {
        self.0.render(call, rng, out)
    }
}

fn load() -> Scenario {
    let manifest =
        Manifest::parse(&format!("name: http2\ntype: app\n{}", manifest_functions())).unwrap();
    let mut set = ModuleSet::default();
    set.insert(manifest);
    let ctx = set.context("http2").unwrap();
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("data")
        .join("h2c_flow.yaml");
    let mut s = Scenario::load_with(&path, Some(&ctx)).unwrap();
    s.fold_constants().unwrap();
    s
}

struct Outcome {
    client: Result<(), String>,
    server: Result<(), String>,
    c2s: Vec<u8>,
    s2c: Vec<u8>,
}

async fn run(s: &Scenario, seed: u64) -> Outcome {
    let filler = Filler::new(seed);
    let flow = &s.flows[0];
    let (a, b) = duplex(64 * 1024);
    let ca = Arc::new(Mutex::new(Vec::new()));
    let cw = Arc::new(Mutex::new(Vec::new()));
    let a = Tap {
        inner: a,
        written: Arc::clone(&ca),
    };
    let b = Tap {
        inner: b,
        written: Arc::clone(&cw),
    };
    let (mut cf, mut sf) = (Fns::default(), Fns::default());
    let mut client = Session::new(SessionCtx {
        vars: &s.vars,
        filler: &filler,
        flow: &flow.id,
        side: Side::Client,
        session_id: 1,
        rng: session_rng(seed, 1, Side::Client),
        functions: &mut cf,
        limits: Limits::default(),
    });
    let mut server = Session::new(SessionCtx {
        vars: &s.vars,
        filler: &filler,
        flow: &flow.id,
        side: Side::Server,
        session_id: 1,
        rng: session_rng(seed, 1, Side::Server),
        functions: &mut sf,
        limits: Limits::default(),
    });
    let (mut co, mut so) = (OneShot(Some(a)), OneShot(Some(b)));
    let (c, sv) = tokio::join!(
        client.run_program(&mut co, &flow.client, std::future::pending()),
        server.run_program(
            &mut so,
            flow.server.as_ref().unwrap(),
            std::future::pending()
        ),
    );
    let c2s = std::mem::take(&mut *ca.lock().unwrap());
    let s2c = std::mem::take(&mut *cw.lock().unwrap());
    Outcome {
        client: c.map_err(|e| format!("{e:?}")),
        server: sv.map_err(|e| format!("{e:?}")),
        c2s,
        s2c,
    }
}

/// Splits a stream into (type, flags, stream, payload) tuples.
fn frames(mut bytes: &[u8]) -> Vec<(u8, u8, u32, Vec<u8>)> {
    let mut v = Vec::new();
    while !bytes.is_empty() {
        let (f, rest) = frame::split(bytes).unwrap();
        v.push((
            f.header.ty,
            f.header.flags,
            f.header.stream,
            f.payload.to_vec(),
        ));
        bytes = rest;
    }
    v
}

#[tokio::test(flavor = "current_thread")]
async fn a_flow_reads_and_writes_frames_with_the_http2_functions() {
    let s = load();
    let o = run(&s, 42).await;
    assert_eq!(o.client, Ok(()), "client");
    assert_eq!(o.server, Ok(()), "server");

    // What the client wrote: preface, SETTINGS, SETTINGS ACK, the request.
    assert_eq!(&o.c2s[..24], PREFACE);
    let cf = frames(&o.c2s[24..]);
    assert_eq!(cf[0].0, ty::SETTINGS);
    assert_eq!(cf[1].0, ty::SETTINGS);
    assert_eq!(cf[1].1, flag::ACK);
    assert_eq!(cf[2].0, ty::HEADERS);
    assert_eq!(cf[2].2, 1);
    let mut block = Vec::new();
    let mut i = 2;
    loop {
        let (t, f, _, p) = &cf[i];
        block.extend_from_slice(frame::block_fragment(*t, *f, p).unwrap());
        i += 1;
        if f & flag::END_HEADERS != 0 {
            break;
        }
        assert_eq!(cf[i].0, ty::CONTINUATION);
    }
    assert!(i > 3, "the header block must span CONTINUATION frames");
    let fields = Decoder::new().decode(&block).unwrap();
    let get = |n: &str| {
        fields
            .iter()
            .find(|f| f.name == n.as_bytes())
            .map(|f| f.value.clone())
    };
    assert_eq!(get(":path"), Some(b"/upload".to_vec()));
    assert_eq!(get("x-request-id").map(|v| v.len()), Some(32));
    let data = &cf[i..];
    assert!(data.iter().all(|f| f.0 == ty::DATA && f.2 == 1));
    let total: usize = data.iter().map(|f| f.3.len()).sum();
    assert_eq!(total, 40000);
    assert!(data.iter().all(|f| f.3.len() <= 100));
    assert_ne!(data.last().unwrap().1 & flag::END_STREAM, 0);

    // What the server wrote: SETTINGS, SETTINGS ACK, response, DATA, GOAWAY.
    let sf = frames(&o.s2c);
    assert_eq!(sf[0].0, ty::SETTINGS);
    assert_eq!(sf[1].0, ty::SETTINGS);
    assert_eq!(sf[2].0, ty::HEADERS);
    assert_eq!(sf[2].2, 1);
    let status = Decoder::new()
        .decode(frame::block_fragment(sf[2].0, sf[2].1, &sf[2].3).unwrap())
        .unwrap();
    assert_eq!(status[0].name, b":status");
    assert_eq!(status[0].value, b"200");
    assert_eq!(sf.last().unwrap().0, ty::GOAWAY);
    let reply: usize = sf
        .iter()
        .filter(|f| f.0 == ty::DATA)
        .map(|f| f.3.len())
        .sum();
    assert_eq!(reply, 5000);
}

#[tokio::test(flavor = "current_thread")]
async fn the_transcript_is_reproducible_for_a_seed() {
    let s = load();
    let a = run(&s, 7).await;
    let b = run(&s, 7).await;
    assert_eq!((a.c2s, a.s2c), (b.c2s, b.s2c));
}
