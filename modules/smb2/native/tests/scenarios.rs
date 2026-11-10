// SPDX-License-Identifier: GPL-3.0-only
//! Runs both sides of every scenario of `modules/smb2/manifest.yaml` against
//! each other through the interpreter, the way the agent runs a module
//! scenario, and checks the messages that each side wrote with the readers of
//! the module. It checks the three exchanges (browse, download, upload), the
//! file sizes that the variables configure with their tail reads and writes,
//! the dialects, the parameter schema of the manifest against the variables of
//! the scenarios, and golden transcripts of a seeded run.
//!
//! Rerun with `UPDATE_GOLDEN=1` to record intended changes of the transcripts.

use std::collections::BTreeMap;
use std::future::Future;
use std::io;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

use fps_flow::template::Filler;
use fps_flow::{FnCall, FnError, Manifest, ModuleSet, Scenario, TemplateFn, Value};
use fps_interp::{session_rng, Limits, OpenRequest, Opener, Session, SessionCtx, Side};
use fps_io::memory::duplex;
use fps_io::SessionIo;
use fps_module_smb2::wire::{transport_length, Message};
use fps_module_smb2::{ntlm, read, Smb2};
use rand::RngCore;
use serde_yaml::Value as Yaml;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

const SCENARIOS: [&str; 3] = ["browse", "download", "upload"];

fn module_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
}

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

fn manifest() -> Manifest {
    Manifest::load(&module_dir().join("manifest.yaml")).unwrap()
}

/// Loads a scenario of the manifest with replaced variables.
fn load(name: &str, overrides: &[(&str, Value)]) -> Scenario {
    let mut set = ModuleSet::default();
    set.insert(manifest());
    let ctx = set.context("smb2").unwrap();
    let path = module_dir().join("scenarios").join(format!("{name}.yaml"));
    let mut s = Scenario::load_with(&path, Some(&ctx)).unwrap();
    let own: Vec<(String, Value)> = overrides
        .iter()
        .map(|(k, v)| ((*k).to_owned(), v.clone()))
        .collect();
    s.override_vars(&own).unwrap();
    s.fold_constants().unwrap();
    s
}

fn int(v: i64) -> Value {
    Value::Int(v)
}

fn text(v: &str) -> Value {
    Value::Str(v.to_owned())
}

struct Outcome {
    client: Result<(), String>,
    server: Result<(), String>,
    c2s: Vec<u8>,
    s2c: Vec<u8>,
}

async fn run(s: &Scenario, seed: u64, session_id: u64) -> Outcome {
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
        session_id,
        rng: session_rng(seed, session_id, Side::Client),
        functions: &mut cf,
        limits: Limits::default(),
    });
    let mut server = Session::new(SessionCtx {
        vars: &s.vars,
        filler: &filler,
        flow: &flow.id,
        side: Side::Server,
        session_id,
        rng: session_rng(seed, session_id, Side::Server),
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

/// Runs a scenario and returns the messages of both directions.
async fn exchange(
    name: &str,
    overrides: &[(&str, Value)],
) -> (Vec<Message<'static>>, Vec<Message<'static>>) {
    let s = load(name, overrides);
    let o = run(&s, 42, 1).await;
    assert_eq!(o.client, Ok(()), "{name} client");
    assert_eq!(o.server, Ok(()), "{name} server");
    // The messages borrow the bytes, and a test process may leak them.
    let (c2s, s2c): (&'static [u8], &'static [u8]) =
        (Box::leak(o.c2s.into()), Box::leak(o.s2c.into()));
    (messages(c2s), messages(s2c))
}

/// Splits a stream of messages with transport headers.
fn messages(mut bytes: &[u8]) -> Vec<Message<'_>> {
    let mut v = Vec::new();
    while !bytes.is_empty() {
        let len = transport_length(bytes).unwrap();
        v.push(Message::parse(&bytes[4..4 + len]).unwrap());
        bytes = &bytes[4 + len..];
    }
    v
}

fn header(m: &Message, f: &str) -> String {
    read::header_field(m, f).unwrap()
}

fn body(m: &Message, f: &str) -> String {
    read::body_field(m, f).unwrap()
}

fn commands(v: &[Message]) -> Vec<String> {
    v.iter().map(|m| header(m, "command")).collect()
}

fn data_len(m: &Message) -> usize {
    read::data(m).unwrap().len()
}

/// Checks what every scenario shares: the session setup, the pairing of
/// requests and responses, the counted message ids, and the ids that the
/// server assigned.
fn assert_session(c: &[Message], r: &[Message]) {
    assert_eq!(c.len(), r.len(), "one response per request");
    assert_eq!(
        commands(&c[..4]),
        [
            "negotiate",
            "session_setup",
            "session_setup",
            "tree_connect"
        ]
    );
    let last = c.len() - 1;
    assert_eq!(header(&c[last - 1], "command"), "tree_disconnect");
    assert_eq!(header(&c[last], "command"), "logoff");
    for (i, (q, a)) in c.iter().zip(r).enumerate() {
        assert_eq!(header(q, "command"), header(a, "command"), "message {i}");
        assert_eq!(header(a, "response"), "true");
        assert_eq!(header(q, "response"), "false");
        // The ids count up without a gap, and a response echoes its request.
        assert_eq!(header(q, "message_id"), i.to_string(), "message {i}");
        assert_eq!(header(a, "message_id"), i.to_string(), "message {i}");
    }
    // The session setup is the NTLM exchange of two requests.
    assert_eq!(header(&r[1], "status"), "more_processing_required");
    assert_eq!(header(&r[2], "status"), "success");
    let sid = header(&r[1], "session_id");
    assert_ne!(sid, "0");
    for m in &c[2..] {
        assert_eq!(header(m, "session_id"), sid);
    }
    let tid = header(&r[3], "tree_id");
    assert_ne!(tid, "0");
    for m in &c[4..last] {
        assert_eq!(header(m, "tree_id"), tid);
    }
    assert_eq!(body(&c[3], "path"), "\\\\fps-files.example.test\\data");
    // The authenticate message names the user, the domain, and the workstation.
    let blob = read::security_blob(&c[2]).unwrap().to_vec();
    let auth = ntlm::find_message(&blob).unwrap();
    assert_eq!(ntlm::field(auth, "user").unwrap(), "alice");
    assert_eq!(ntlm::field(auth, "domain").unwrap(), "FPS");
    assert_eq!(ntlm::field(auth, "workstation").unwrap(), "WS-0417");
}

#[tokio::test(start_paused = true)]
async fn browse_lists_the_root_and_a_subdirectory() {
    let (c, r) = exchange("browse", &[]).await;
    assert_session(&c, &r);
    let want = [
        "negotiate",
        "session_setup",
        "session_setup",
        "tree_connect",
        "create",
        "query_directory",
        "query_directory",
        "close",
        "create",
        "query_directory",
        "query_directory",
        "close",
        "tree_disconnect",
        "logoff",
    ];
    assert_eq!(commands(&c), want);
    // Both directories open as directories, the second one by name.
    assert_eq!(body(&c[4], "name"), "");
    assert_eq!(body(&c[8], "name"), "Projects");
    assert_eq!(body(&r[4], "attributes"), "16");
    // The first listing returns every entry, and the scan then ends.
    assert_eq!(body(&r[5], "entry_count"), "8");
    assert!(body(&r[5], "entries").contains("site-survey.pdf"));
    assert_eq!(header(&r[6], "status"), "no_more_files");
    assert_eq!(body(&r[9], "entry_count"), "5");
    assert!(body(&r[9], "entries").contains("plan-v2.docx"));
    assert_eq!(header(&r[10], "status"), "no_more_files");
    // The scan restarts on the first request of a directory only.
    assert_eq!(body(&c[5], "flags"), body(&c[9], "flags"));
    assert_ne!(body(&c[5], "flags"), body(&c[6], "flags"));
    // Every directory request names the handle that its CREATE returned.
    assert_eq!(body(&c[5], "file_id"), body(&c[7], "file_id"));
    assert_eq!(body(&c[6], "file_id"), body(&c[7], "file_id"));
    assert_eq!(body(&c[9], "file_id"), body(&c[11], "file_id"));
}

#[tokio::test(start_paused = true)]
async fn download_reads_the_file_in_chunks() {
    let (c, r) = exchange("download", &[]).await;
    assert_session(&c, &r);
    // 1 MiB in 64 KiB reads is sixteen requests.
    assert_eq!(c.len(), 4 + 1 + 16 + 1 + 2);
    assert_eq!(body(&r[4], "size"), "1048576");
    let reads: Vec<&Message> = c
        .iter()
        .filter(|m| header(m, "command") == "read")
        .collect();
    assert_eq!(reads.len(), 16);
    let offsets: Vec<String> = reads.iter().map(|m| body(m, "offset")).collect();
    let expected: Vec<String> = (0..16).map(|i| (i * 65536).to_string()).collect();
    assert_eq!(offsets, expected);
    let got: usize = r
        .iter()
        .filter(|m| header(m, "command") == "read")
        .map(data_len)
        .sum();
    assert_eq!(got, 1_048_576);
    // A read names the handle of the CREATE response.
    let fid = body(&r[4], "file_id");
    assert!(reads.iter().all(|m| body(m, "file_id") == fid));
    assert_eq!(body(&c[4], "name"), "Projects\\site-survey.pdf");
}

async fn download_sizes(file_size: i64, read_size: i64, tail: i64) {
    let (c, r) = exchange(
        "download",
        &[
            ("file_size", int(file_size)),
            ("read_size", int(read_size)),
            ("tail_size", int(tail)),
        ],
    )
    .await;
    assert_session(&c, &r);
    let lens: Vec<usize> = c
        .iter()
        .filter(|m| header(m, "command") == "read")
        .map(|m| body(m, "length").parse().unwrap())
        .collect();
    let full = usize::try_from((file_size - tail) / read_size).unwrap();
    assert_eq!(lens.len(), full + 1, "{file_size} in {read_size}");
    assert!(lens[..full].iter().all(|l| *l == read_size as usize));
    assert_eq!(lens[full], tail as usize);
    let got: usize = r
        .iter()
        .filter(|m| header(m, "command") == "read")
        .map(data_len)
        .sum();
    assert_eq!(got, file_size as usize, "{file_size} in {read_size}");
    assert_eq!(body(&r[4], "size"), file_size.to_string());
}

#[tokio::test(start_paused = true)]
async fn download_honors_file_sizes_that_are_not_a_multiple_of_the_read_size() {
    // A tail of its own, a file that fits one read, one read of an exact
    // multiple, and a larger read size.
    download_sizes(150_000, 65_536, 18_928).await;
    download_sizes(1_000, 65_536, 1_000).await;
    download_sizes(196_608, 65_536, 65_536).await;
    download_sizes(5_000_000, 1_048_576, 805_696).await;
}

#[tokio::test(start_paused = true)]
async fn upload_writes_the_file_in_chunks() {
    let (c, r) = exchange("upload", &[]).await;
    assert_session(&c, &r);
    assert_eq!(c.len(), 4 + 1 + 16 + 1 + 2);
    // FILE_CREATE with write access, and the server reports FILE_CREATED.
    assert_eq!(body(&c[4], "disposition"), "2");
    assert_eq!(body(&r[4], "action"), "2");
    assert_eq!(body(&c[4], "name"), "Projects\\backup-2026-11.bin");
    let writes: Vec<&Message> = c
        .iter()
        .filter(|m| header(m, "command") == "write")
        .collect();
    assert_eq!(writes.len(), 16);
    let offsets: Vec<String> = writes.iter().map(|m| body(m, "offset")).collect();
    let expected: Vec<String> = (0..16).map(|i| (i * 65536).to_string()).collect();
    assert_eq!(offsets, expected);
    let sent: usize = writes.iter().map(|m| data_len(m)).sum();
    assert_eq!(sent, 1_048_576);
    let acked: usize = r
        .iter()
        .filter(|m| header(m, "command") == "write")
        .map(|m| body(m, "count").parse::<usize>().unwrap())
        .sum();
    assert_eq!(acked, 1_048_576);
    let fid = body(&r[4], "file_id");
    assert!(writes.iter().all(|m| body(m, "file_id") == fid));
}

#[tokio::test(start_paused = true)]
async fn upload_honors_sizes_that_are_not_a_multiple_of_the_write_size() {
    for (size, write, tail) in [
        (150_000, 65_536, 18_928),
        (700, 65_536, 700),
        (131_072, 65_536, 65_536),
    ] {
        let (c, r) = exchange(
            "upload",
            &[
                ("upload_size", int(size)),
                ("write_size", int(write)),
                ("upload_tail", int(tail)),
            ],
        )
        .await;
        assert_session(&c, &r);
        let lens: Vec<usize> = c
            .iter()
            .filter(|m| header(m, "command") == "write")
            .map(data_len)
            .collect();
        let full = usize::try_from((size - tail) / write).unwrap();
        assert_eq!(lens.len(), full + 1, "{size} in {write}");
        assert_eq!(lens.iter().sum::<usize>(), size as usize);
        assert_eq!(*lens.last().unwrap(), tail as usize);
    }
}

#[tokio::test(start_paused = true)]
async fn every_scenario_runs_with_smb_3_1_1() {
    let dialects = ("dialects", text("2.1,3.0.2,3.1.1"));
    let dialect = ("dialect", text("3.1.1"));
    for name in SCENARIOS {
        let (c, r) = exchange(name, &[dialects.clone(), dialect.clone()]).await;
        assert_session(&c, &r);
        assert_eq!(body(&r[0], "dialect"), "3.1.1", "{name}");
        assert_eq!(body(&c[0], "dialects"), "2.1,3.0.2,3.1.1", "{name}");
    }
}

#[tokio::test(start_paused = true)]
async fn the_dialect_variables_choose_the_negotiated_dialect() {
    let (c, r) = exchange("download", &[]).await;
    assert_eq!(body(&c[0], "dialects"), "2.0.2,2.1");
    assert_eq!(body(&r[0], "dialect"), "2.1");
}

#[tokio::test(start_paused = true)]
async fn sessions_draw_their_ids_from_the_session_generator() {
    let s = load("download", &[]);
    let mut ids = std::collections::BTreeSet::new();
    for session_id in 1..=6 {
        let o = run(&s, 42, session_id).await;
        assert_eq!(o.server, Ok(()));
        let r = messages(&o.s2c);
        ids.insert((header(&r[1], "session_id"), header(&r[3], "tree_id")));
    }
    assert_eq!(
        ids.len(),
        6,
        "every session has its own session and tree id"
    );
}

/// Sizes that keep transcripts small: files of 10000 octets in reads of 4096.
fn small_files() -> [(&'static str, Value); 6] {
    [
        ("file_size", int(10_000)),
        ("read_size", int(4_096)),
        ("tail_size", int(1_808)),
        ("upload_size", int(10_000)),
        ("write_size", int(4_096)),
        ("upload_tail", int(1_808)),
    ]
}

/// Loads a scenario with the small sizes of those variables that it declares.
fn load_small(name: &str) -> Scenario {
    let probe = load(name, &[]);
    let own: Vec<(&str, Value)> = small_files()
        .into_iter()
        .filter(|(k, _)| probe.vars.contains_key(*k))
        .collect();
    load(name, &own)
}

#[tokio::test(start_paused = true)]
async fn the_same_seed_renders_the_same_bytes() {
    for name in SCENARIOS {
        let s = load_small(name);
        let a = run(&s, 42, 1).await;
        let b = run(&s, 42, 1).await;
        assert_eq!((&a.c2s, &a.s2c), (&b.c2s, &b.s2c), "{name}");
        let c = run(&s, 43, 1).await;
        // The client draws its challenge and the server draws its challenge
        // and its ids, so the seed changes both directions.
        assert_ne!(a.c2s, c.c2s, "{name} ignores the seed");
        assert_ne!(a.s2c, c.s2c, "{name} ignores the seed");
    }
}

#[tokio::test(start_paused = true)]
async fn every_scenario_matches_its_golden_transcript() {
    let testdata = module_dir().join("native/tests/testdata");
    for name in SCENARIOS {
        let o = run(&load_small(name), 42, 1).await;
        assert_eq!(o.client, Ok(()), "{name} client");
        assert_eq!(o.server, Ok(()), "{name} server");
        fps_golden::assert_golden(testdata.join(format!("{name}.seed42.c2s.bin")), &o.c2s);
        fps_golden::assert_golden(testdata.join(format!("{name}.seed42.s2c.bin")), &o.s2c);
    }
}

/// The manifest lists the scenarios, the fixture, and a parameter for every
/// variable of every scenario with the default that the scenario sets.
#[test]
fn the_parameter_schema_describes_the_variables_of_the_scenarios() {
    let text = std::fs::read_to_string(module_dir().join("manifest.yaml")).unwrap();
    let doc: Yaml = serde_yaml::from_str(&text).unwrap();
    let listed: Vec<&str> = doc["scenarios"]
        .as_sequence()
        .unwrap()
        .iter()
        .map(|s| s.as_str().unwrap())
        .collect();
    let want: Vec<String> = SCENARIOS
        .iter()
        .map(|n| format!("scenarios/{n}.yaml"))
        .collect();
    assert_eq!(listed, want);

    let props = doc["parameters"]["properties"].as_mapping().unwrap();
    let mut used: BTreeMap<String, Value> = BTreeMap::new();
    for name in SCENARIOS {
        let s = load(name, &[]);
        for (k, v) in &s.vars {
            // The ids that the server assigns are set by actions, not parameters.
            if s.set_targets().contains(k.as_str()) {
                continue;
            }
            let p = props
                .get(k.as_str())
                .unwrap_or_else(|| panic!("{name}: variable {k} is not a parameter"));
            let default = &p["default"];
            let same = match v {
                Value::Int(i) => default.as_i64() == Some(*i),
                Value::Str(t) => default.as_str() == Some(t.as_str()),
                other => panic!("{name}: {k} is {other:?}"),
            };
            assert!(
                same,
                "{name}: default of {k} is {default:?}, the scenario has {v:?}"
            );
            if let Some(old) = used.insert(k.clone(), v.clone()) {
                assert_eq!(&old, v, "{k} has two defaults");
            }
        }
    }
    for (k, _) in props {
        let k = k.as_str().unwrap();
        assert!(used.contains_key(k), "parameter {k} is used by no scenario");
    }
}

/// The default sizes of the manifest satisfy the rule of the tail variables.
#[test]
fn the_default_tails_follow_from_the_default_sizes() {
    let s = load("download", &[]);
    let (Some(Value::Int(size)), Some(Value::Int(read)), Some(Value::Int(tail))) = (
        s.vars.get("file_size"),
        s.vars.get("read_size"),
        s.vars.get("tail_size"),
    ) else {
        panic!("download variables");
    };
    assert_eq!(*tail, if size % read == 0 { *read } else { size % read });
    let s = load("upload", &[]);
    let (Some(Value::Int(size)), Some(Value::Int(write)), Some(Value::Int(tail))) = (
        s.vars.get("upload_size"),
        s.vars.get("write_size"),
        s.vars.get("upload_tail"),
    ) else {
        panic!("upload variables");
    };
    assert_eq!(
        *tail,
        if size % write == 0 {
            *write
        } else {
            size % write
        }
    );
}
