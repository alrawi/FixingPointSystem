// SPDX-License-Identifier: GPL-3.0-only
//! Runs both sides of a scenario through the interpreter with the smb2
//! functions, the way the agent runs a module scenario. The scenario in
//! `tests/data/smb2_flow.yaml` shows the idiom that a flow uses to read
//! messages and to number them in a loop, and the test checks the bytes that
//! each side wrote with the readers of the module and with the sizes that the
//! scenario sets.

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
use fps_module_smb2::catalog::manifest_functions;
use fps_module_smb2::Smb2;
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
struct Fns(Smb2);

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
        Manifest::parse(&format!("name: smb2\ntype: app\n{}", manifest_functions())).unwrap();
    let mut set = ModuleSet::default();
    set.insert(manifest);
    let ctx = set.context("smb2").unwrap();
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("data")
        .join("smb2_flow.yaml");
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

/// Splits a stream of messages with transport headers into messages.
fn messages(mut bytes: &[u8]) -> Vec<Vec<u8>> {
    let mut v = Vec::new();
    while !bytes.is_empty() {
        let len = fps_module_smb2::wire::transport_length(bytes).unwrap();
        v.push(bytes[4..4 + len].to_vec());
        bytes = &bytes[4 + len..];
    }
    v
}

fn field(m: &[u8], f: &str, header: bool) -> String {
    let m = fps_module_smb2::wire::Message::parse(m).unwrap();
    if header {
        fps_module_smb2::read::header_field(&m, f).unwrap()
    } else {
        fps_module_smb2::read::body_field(&m, f).unwrap()
    }
}

#[tokio::test(flavor = "current_thread")]
async fn a_flow_reads_numbers_and_writes_messages_with_the_smb2_functions() {
    let s = load();
    let o = run(&s, 42).await;
    assert_eq!(o.client, Ok(()), "client");
    assert_eq!(o.server, Ok(()), "server");

    let c = messages(&o.c2s);
    let r = messages(&o.s2c);
    let cmds =
        |v: &[Vec<u8>]| -> Vec<String> { v.iter().map(|m| field(m, "command", true)).collect() };
    // negotiate, three session setups, tree connect, create, 3 reads, close,
    // create, write, close, tree disconnect, logoff.
    let want: [&str; 14] = [
        "negotiate",
        "session_setup",
        "session_setup",
        "tree_connect",
        "create",
        "read",
        "read",
        "read",
        "close",
        "create",
        "write",
        "close",
        "tree_disconnect",
        "logoff",
    ];
    assert_eq!(cmds(&c), want);
    assert_eq!(cmds(&r).len(), c.len());
    for (q, a) in c.iter().zip(&r) {
        assert_eq!(field(q, "command", true), field(a, "command", true));
        assert_eq!(
            field(q, "message_id", true),
            field(a, "message_id", true),
            "a response carries the message id of its request"
        );
        assert_eq!(field(a, "response", true), "true");
    }

    // The message ids count up without a gap, which the loop does with add.
    let ids: Vec<String> = c.iter().map(|m| field(m, "message_id", true)).collect();
    assert_eq!(
        ids,
        ["0", "1", "2", "3", "4", "5", "6", "7", "8", "9", "10", "11", "12", "13"]
    );

    // The client used the session id and the tree id that the server assigned.
    let sid = "4611686018427387905";
    for m in &c[2..] {
        assert_eq!(field(m, "session_id", true), sid);
    }
    for m in &c[4..c.len() - 1] {
        assert_eq!(field(m, "tree_id", true), "7");
    }
    assert_eq!(field(&c[3], "path", false), "\\\\fpsserver\\share");
    assert_eq!(field(&c[4], "name", false), "docs\\report.bin");

    // The reads walk the file in chunks, and the replies carry them.
    let offsets: Vec<String> = c[5..8].iter().map(|m| field(m, "offset", false)).collect();
    assert_eq!(offsets, ["0", "65536", "131072"]);
    assert!(c[5..8]
        .iter()
        .all(|m| field(m, "file_id", false) == "0100000000000000ffffffff00000000"));
    let replied: usize = r[5..8]
        .iter()
        .map(|m| {
            fps_module_smb2::read::data(&fps_module_smb2::wire::Message::parse(m).unwrap())
                .unwrap()
                .len()
        })
        .sum();
    assert_eq!(replied, 196_608);
    assert_eq!(field(&r[4], "size", false), "196608");

    // The write carries the bytes of the scenario and the server acknowledges them all.
    assert_eq!(field(&c[10], "length", false), "40000");
    assert_eq!(field(&r[10], "count", false), "40000");
    assert_eq!(field(&c[9], "disposition", false), "2");

    // The NTLM exchange: the authenticate message names the user and the domain.
    let blob = |m: &Vec<u8>| {
        let m = fps_module_smb2::wire::Message::parse(m).unwrap();
        fps_module_smb2::read::security_blob(&m).unwrap().to_vec()
    };
    let auth = blob(&c[2]);
    assert_eq!(
        fps_module_smb2::ntlm::field(fps_module_smb2::ntlm::find_message(&auth).unwrap(), "user")
            .unwrap(),
        "alice"
    );
    assert_eq!(
        fps_module_smb2::ntlm::field(
            fps_module_smb2::ntlm::find_message(&auth).unwrap(),
            "domain"
        )
        .unwrap(),
        "FPS"
    );
    assert_eq!(field(&r[1], "status", true), "more_processing_required");
    assert_eq!(field(&r[2], "status", true), "success");
    assert_eq!(field(&r[0], "dialect", false), "2.1");
}

#[tokio::test(flavor = "current_thread")]
async fn the_transcript_is_reproducible_for_a_seed() {
    let s = load();
    let a = run(&s, 7).await;
    let b = run(&s, 7).await;
    assert_eq!((&a.c2s, &a.s2c), (&b.c2s, &b.s2c));
    // Another seed gives other random octets, the client challenge and the file data.
    let c = run(&s, 8).await;
    assert_ne!(a.c2s, c.c2s);
}
