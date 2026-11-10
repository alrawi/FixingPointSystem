// SPDX-License-Identifier: GPL-3.0-only
//! AGT-14: the https scenarios of the http1 module run through the agent, the
//! capture of such a run decrypts with the key log that the capture file
//! carries, and a criterion with labels counts the series that it names.
//!
//! The plans use the module trees of the repository, so the digests are the
//! real tree digests. No SMTP STARTTLS scenario exists yet (MOD-20 adds it), so
//! the STARTTLS path is covered by the tests of `fps-runtime` (ENG-20), which
//! upgrade the same way with the same registry type. The decrypter of this file
//! is built on `ring` and shares no code with the TLS module.
//!
//! Run it with `cargo test -p fps-agent --test tls`.

#[path = "../../fps-capture/tests/common/mod.rs"]
mod common;

use std::collections::{BTreeMap, BTreeSet};
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::Duration;

use fps_agent::assemble::Sides;
use fps_agent::control::{CaptureSettings, Captures};
use fps_agent::modules::{sha256_digest, tree_digest};
use fps_agent::plan::Plan;
use fps_agent::run::{prepare, prepare_with};
use fps_agent::tls::{self, KeyTap, Needs, SERVER_TLS12, SERVER_TLS13};
use fps_agent::trust::ModulePolicy;
use fps_agent::{run, Outcome, RunOptions};
use fps_flow::program::TlsRole;
use fps_interp::{BoxedIo, TlsUpgrader, UpgradeError, UpgradeRequest};
use fps_io::SessionIo;
use fps_stats::metric;
use ring::{aead, hkdf};
use serde_json::{json, Value as Json};
use tokio::io::{AsyncRead, AsyncWrite, DuplexStream, ReadBuf};

fn repo_modules() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../modules")
}

fn manifest_version(module: &str) -> String {
    let text = std::fs::read_to_string(repo_modules().join(module).join("manifest.yaml"))
        .expect("manifest");
    text.lines()
        .find_map(|l| l.strip_prefix("version:"))
        .map(|v| v.trim().trim_matches('"').to_owned())
        .expect("version")
}

/// Returns a loopback plan of http1 scenarios with one mix entry per
/// `(scenario name, port)`, client addresses in `127.<c>.0.0/24`, server
/// addresses in `127.<s>.0.0/24`, a load of `secs` seconds, and `criteria`.
fn https_plan(
    entries: &[(&str, u16)],
    (c, s): (u8, u8),
    secs: u32,
    criteria: &Json,
    edit_source: impl Fn(&str) -> String,
) -> Json {
    let text = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../contracts/examples/plans/http_dns_smoke.json"),
    )
    .expect("example plan");
    let mut plan: Json = serde_json::from_str(&text).expect("example plan JSON");
    plan["modules"] = json!([{
        "name": "http1",
        "version": manifest_version("http1"),
        "digest": tree_digest("http1", &repo_modules().join("http1")).expect("digest"),
    }]);
    let (mut scenarios, mut mix, mut rates) = (Vec::new(), Vec::new(), Vec::new());
    for (name, port) in entries {
        let source = std::fs::read_to_string(
            repo_modules()
                .join("http1/scenarios")
                .join(format!("{name}.yaml")),
        )
        .expect("scenario source");
        let source = edit_source(&source);
        scenarios.push(json!({
            "ref": format!("http1/{name}"),
            "module": "http1",
            "digest": sha256_digest(source.as_bytes()),
            "source": source,
        }));
        mix.push(json!({
            "id": name,
            "scenario": format!("http1/{name}"),
            "weight": 1,
            "params": { "server_port": port },
            "clientPool": "clients",
            "serverPool": "servers",
        }));
        rates.push(json!({ "mixEntry": name, "sessionsPerSecond": 4 }));
    }
    plan["scenarios"] = scenarios.into();
    plan["mix"] = mix.into();
    plan["hostPools"][0]["ipv4Prefix"] = format!("127.{c}.0.0/24").into();
    plan["hostPools"][1]["ipv4Prefix"] = format!("127.{s}.0.0/24").into();
    for w in plan["workers"].as_array_mut().expect("workers") {
        w["rates"] = rates.clone().into();
    }
    plan["load"]["phases"] = json!([
        { "name": "steady", "duration": format!("{secs}s"), "shape": "PHASE_SHAPE_HOLD",
          "startFraction": 1, "endFraction": 1 }
    ]);
    plan["warmup"] = "0s".into();
    plan["seed"] = "20261110".into();
    plan["criteria"] = criteria.clone();
    plan
}

fn write_plan(name: &str, plan: &Json) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("fps-agent-tls");
    std::fs::create_dir_all(&dir).expect("plan dir");
    let path = dir.join(format!("{name}.json"));
    std::fs::write(&path, serde_json::to_string_pretty(plan).unwrap()).expect("write plan");
    path
}

fn no_criteria() -> Json {
    json!([])
}

// Deliverable 1 and 3: https through the agent, and a server side criterion. -

#[test]
fn https_scenarios_run_through_the_agent_and_a_server_criterion_counts_the_server_series() {
    // Two scenarios cover both versions and two browser profiles: chrome-136
    // with server-tls13, and safari-18.5 with server-tls12.
    let criteria = json!([
        { "metric": "sessions.failed", "op": "CRITERION_OP_EQ", "value": 0 },
        { "metric": "sessions.succeeded", "op": "CRITERION_OP_GT", "value": 0,
          "labels": { "side": "server" } },
        // The servers send the bodies, the clients only the requests.
        { "metric": "bytes.tx", "op": "CRITERION_OP_GT", "value": 4000,
          "labels": { "side": "server" } },
    ]);
    let plan = https_plan(
        &[("https_fetch", 18_851), ("https_fetch_tls12", 18_852)],
        (101, 102),
        3,
        &criteria,
        str::to_owned,
    );
    let path = write_plan("https", &plan);
    let (mut out, mut log) = (Vec::new(), Vec::new());
    let summary = run(&RunOptions::new(path, repo_modules()), &mut out, &mut log).expect("run");
    let log = String::from_utf8(log).unwrap();
    assert_eq!(summary.outcome, Outcome::Pass, "{log}");

    let client = summary.client.as_ref().expect("client totals");
    let server = summary.server.as_ref().expect("server totals");
    assert!(
        client.counter_total(metric::SESSIONS_SUCCEEDED) > 0,
        "{log}"
    );
    assert_eq!(client.counter_total(metric::SESSIONS_FAILED), 0, "{log}");
    assert_eq!(server.counter_total(metric::SESSIONS_FAILED), 0, "{log}");

    // The labeled criterion measured the server series: its value is the
    // server total, which dwarfs the client total of the same metric.
    let (c_tx, s_tx) = (
        client.counter_total(metric::BYTES_TX),
        server.counter_total(metric::BYTES_TX),
    );
    assert!(s_tx > 10 * c_tx, "server {s_tx} client {c_tx}");
    let measured = &summary.criteria[2];
    assert_eq!(measured.value, s_tx as f64, "{measured:?}");
    assert_eq!(
        summary.criteria[1].value,
        server.counter_total(metric::SESSIONS_SUCCEEDED) as f64
    );
}

#[test]
fn a_labeled_criterion_that_the_server_side_fails_fails_the_run() {
    // Clients send little, so a server side threshold on the client volume
    // would hold wrongly if the label were ignored. Here the client side
    // default holds (the clients sent less than 10 MB) and the server side
    // criterion demands more than the servers can send in three seconds.
    let criteria = json!([
        { "metric": "bytes.tx", "op": "CRITERION_OP_LT", "value": 10_000_000 },
        { "metric": "bytes.tx", "op": "CRITERION_OP_GT", "value": 1_000_000_000,
          "labels": { "side": "server" } },
    ]);
    let plan = https_plan(
        &[("https_fetch", 18_853)],
        (103, 104),
        2,
        &criteria,
        str::to_owned,
    );
    let path = write_plan("https-fails", &plan);
    let (mut out, mut log) = (Vec::new(), Vec::new());
    let summary = run(&RunOptions::new(path, repo_modules()), &mut out, &mut log).expect("run");
    assert_eq!(summary.outcome, Outcome::Fail);
    assert!(summary.criteria[0].holds && !summary.criteria[1].holds);
}

#[test]
fn a_criterion_label_that_the_metric_lacks_ends_the_run_with_an_error() {
    let criteria = json!([
        { "metric": "sessions.succeeded", "op": "CRITERION_OP_GT", "value": 0,
          "labels": { "port": "p0" } },
    ]);
    let plan = https_plan(
        &[("https_fetch", 18_854)],
        (105, 106),
        2,
        &criteria,
        str::to_owned,
    );
    let path = write_plan("https-bad-label", &plan);
    let (mut out, mut log) = (Vec::new(), Vec::new());
    let e = run(&RunOptions::new(path, repo_modules()), &mut out, &mut log)
        .expect_err("a bad label")
        .to_string();
    assert!(
        e.contains("port") && e.contains("sessions.succeeded"),
        "{e}"
    );
}

#[test]
fn a_plan_that_names_an_unknown_tls_profile_is_refused_at_prepare() {
    let plan = https_plan(
        &[("https_fetch", 18_855)],
        (107, 108),
        2,
        &no_criteria(),
        |s| s.replace("chrome-136", "lynx-2.8"),
    );
    let plan = Plan::parse(&plan.to_string()).expect("plan");
    let e = prepare(plan, &repo_modules(), Sides::Both, &ModulePolicy::default())
        .expect_err("an unknown profile")
        .to_string();
    assert!(
        e.contains("lynx-2.8") && e.contains("chrome-136") && e.contains("https_fetch"),
        "{e}"
    );
    // A server profile name cannot serve a client action either.
    let plan = https_plan(
        &[("https_fetch", 18_855)],
        (107, 108),
        2,
        &no_criteria(),
        |s| s.replace("chrome-136", "server-tls13"),
    );
    let plan = Plan::parse(&plan.to_string()).expect("plan");
    assert!(prepare(plan, &repo_modules(), Sides::Both, &ModulePolicy::default()).is_err());
}

#[test]
fn a_client_process_and_a_server_process_with_one_seed_trust_each_other() {
    // Each prepare builds its own authority from the seed, as two agents do.
    let plan = https_plan(
        &[("https_fetch", 18_856)],
        (109, 110),
        2,
        &no_criteria(),
        str::to_owned,
    );
    let text = plan.to_string();
    let policy = ModulePolicy::default();
    let server = prepare(
        Plan::parse(&text).expect("plan"),
        &repo_modules(),
        Sides::Server,
        &policy,
    )
    .expect("server prepare")
    .start(Duration::from_millis(500))
    .expect("server start");
    let client = prepare(
        Plan::parse(&text).expect("plan"),
        &repo_modules(),
        Sides::Client,
        &policy,
    )
    .expect("client prepare")
    .start(Duration::from_millis(500))
    .expect("client start");
    let done = client.finish(Duration::ZERO).expect("client finish");
    let served = server
        .finish(Duration::from_secs(1))
        .expect("server finish");
    let c = done.client.expect("client totals");
    let s = served.server.expect("server totals");
    assert!(c.counter_total(metric::SESSIONS_SUCCEEDED) > 0);
    assert_eq!(c.counter_total(metric::SESSIONS_FAILED), 0);
    assert_eq!(
        s.counter_total(metric::SESSIONS_SUCCEEDED),
        c.counter_total(metric::SESSIONS_SUCCEEDED)
    );
}

// Deliverable 2: the key log in the capture download. -----------------------

/// Runs `plan` with a capture that starts before the first session and
/// returns the file of the capture and the number of client sessions.
fn captured_run(
    caps: &Captures,
    plan_id: &str,
    capture_id: &str,
    export: bool,
    plan: &Json,
) -> (PathBuf, u64) {
    let plan = Plan::parse(&plan.to_string()).expect("plan");
    let seed = plan.seed;
    caps.reset(plan_id);
    let prepared = prepare_with(
        plan,
        &repo_modules(),
        Sides::Both,
        &ModulePolicy::default(),
        &caps.key_tap(),
    )
    .expect("prepare");
    let entry = caps
        .start(
            plan_id,
            capture_id,
            "",
            "",
            0,
            0,
            None,
            &[],
            &[],
            seed,
            export,
        )
        .expect("start capture");
    let active = prepared
        .start_with(Duration::from_millis(500), caps.cell())
        .expect("start");
    while !active.clients_finished() {
        std::thread::sleep(Duration::from_millis(50));
    }
    let finished = active.finish(Duration::from_secs(1)).expect("finish");
    let done = caps.finish(&entry).expect("capture file");
    let attempted = finished
        .client
        .expect("client totals")
        .counter_total(metric::SESSIONS_ATTEMPTED);
    (done.path, attempted)
}

fn scratch_captures(name: &str) -> Captures {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("fps-agent-tls-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    Captures::new(CaptureSettings {
        dir,
        retention: None,
        ..CaptureSettings::default()
    })
}

const DSB: u32 = 0x0A;
const EPB: u32 = 6;
const SECRETS_TLS: u32 = 0x544c_534b;

/// Groups the TCP packets to `port` by client address and port.
fn connections(packets: &[common::Packet], port: u16) -> Vec<common::Conn> {
    let clients: BTreeSet<(IpAddr, u16)> = packets
        .iter()
        .filter(|p| !p.udp && p.dport == port)
        .map(|p| (p.src, p.sport))
        .collect();
    clients
        .into_iter()
        .map(|(ip, cport)| {
            let one: Vec<common::Packet> = packets
                .iter()
                .filter(|p| {
                    !p.udp
                        && ((p.src == ip && p.sport == cport && p.dport == port)
                            || (p.dst == ip && p.dport == cport && p.sport == port))
                })
                .cloned()
                .collect();
            common::reassemble(&one, port)
                .into_values()
                .next()
                .expect("one connection")
        })
        .collect()
}

#[test]
fn a_capture_that_exports_keys_decrypts_alone_and_one_that_does_not_holds_no_keys() {
    let caps = scratch_captures("keys");
    let port = 18_857;
    let plan = https_plan(
        &[("https_fetch", port)],
        (111, 112),
        2,
        &no_criteria(),
        str::to_owned,
    );
    let (path, attempted) = captured_run(&caps, "plan-keys", "with", true, &plan);
    assert!(attempted > 0);

    let file = common::read_pcapng(&path);
    // One section, one decryption secrets block of the TLS key log, and it
    // stands ahead of the first packet that it decrypts.
    assert_eq!(file.section_headers, 1, "{:?}", file.order);
    assert_eq!(file.secrets.len(), 1, "{:?}", file.order);
    assert_eq!(file.secrets[0].0, SECRETS_TLS);
    let dsb_at = file.order.iter().position(|k| *k == DSB).expect("a DSB");
    let first_packet = file.order.iter().position(|k| *k == EPB).expect("packets");
    assert!(dsb_at < first_packet, "{:?}", file.order);
    let keylog = String::from_utf8(file.secrets[0].1.clone()).expect("key log text");
    assert!(keylog.contains("CLIENT_TRAFFIC_SECRET_0"), "{keylog}");

    // Decrypt every captured session with the lines of the file alone.
    // The agent writes whole sessions one after the other, so the timestamps
    // of the file do not rise, and the helper that checks them does not apply.
    let parsed: Vec<common::Packet> = file
        .packets
        .iter()
        .map(|(ts, frame)| common::parse_frame(*ts, frame))
        .collect();
    let conns = connections(&parsed, port);
    assert!(!conns.is_empty() && conns.len() as u64 <= attempted);
    for conn in &conns {
        assert!(!contains(&conn.to_server, "GET /index.html"));
        assert!(!contains(&conn.to_client, "HTTP/1.1 200"));
        assert_eq!(
            conn.to_server[..3],
            [0x16, 0x03, 0x01],
            "ClientHello record"
        );
        let (c2s, s2c) = decrypt_connection(&keylog, &conn.to_server, &conn.to_client)
            .expect("the key log of the file decrypts the capture");
        let c2s = String::from_utf8_lossy(&c2s);
        let s2c = String::from_utf8_lossy(&s2c);
        assert!(
            c2s.starts_with("GET /index.html HTTP/1.1\r\n")
                && c2s.contains("Host: www.example.test\r\n"),
            "{c2s}"
        );
        assert!(s2c.starts_with("HTTP/1.1 200 "), "{s2c}");
        // A key log with flipped digits decrypts nothing.
        assert!(decrypt_connection(&broken(&keylog), &conn.to_server, &conn.to_client).is_none());
    }

    // Without export_tls_keys the file holds no key, so the controller gets
    // no secret that it did not ask for.
    let plan = https_plan(
        &[("https_fetch", 18_858)],
        (113, 114),
        2,
        &no_criteria(),
        str::to_owned,
    );
    let (path, _) = captured_run(&caps, "plan-nokeys", "without", false, &plan);
    let file = common::read_pcapng(&path);
    assert!(file.secrets.is_empty(), "{:?}", file.order);
    assert!(file.order.contains(&EPB));
}

// Deliverable 1: authorities and one ClientHello shape. ---------------------

fn needs(client: &str, server: &str) -> Needs {
    Needs {
        clients: BTreeSet::from([client.to_owned()]),
        servers: BTreeSet::from([server.to_owned()]),
        names: BTreeSet::from(["www.example.test".to_owned()]),
    }
}

/// An in memory pipe that records what its owner wrote.
struct Pipe {
    inner: DuplexStream,
    written: Arc<Mutex<Vec<u8>>>,
}

impl AsyncRead for Pipe {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_read(cx, buf)
    }
}

impl AsyncWrite for Pipe {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        data: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        let n = match Pin::new(&mut self.inner).poll_write(cx, data) {
            Poll::Ready(Ok(n)) => n,
            other => return other,
        };
        self.written.lock().unwrap().extend_from_slice(&data[..n]);
        Poll::Ready(Ok(n))
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

impl SessionIo for Pipe {
    fn abort(&mut self) {}
}

fn pipes() -> (Pipe, Arc<Mutex<Vec<u8>>>, Pipe) {
    let (a, b) = tokio::io::duplex(64 * 1024);
    let written = Arc::new(Mutex::new(Vec::new()));
    let client = Pipe {
        inner: a,
        written: Arc::clone(&written),
    };
    let server = Pipe {
        inner: b,
        written: Arc::default(),
    };
    (client, written, server)
}

/// Runs one handshake between the client profile and the server profile of
/// two registries and returns the result of the client and the bytes that the
/// client sent.
async fn handshake(
    client: &impl TlsUpgrader,
    server: &impl TlsUpgrader,
    client_profile: &str,
    server_profile: &str,
) -> (Result<(), UpgradeError>, Vec<u8>) {
    let (c, sent, s) = pipes();
    let c_req = UpgradeRequest {
        role: TlsRole::Client,
        profile: client_profile,
        server_name: Some("www.example.test"),
    };
    let s_req = UpgradeRequest {
        role: TlsRole::Server,
        profile: server_profile,
        server_name: None,
    };
    let (c_res, s_res) = tokio::join!(
        client.upgrade(Box::new(c) as BoxedIo<'_>, c_req),
        server.upgrade(Box::new(s) as BoxedIo<'_>, s_req)
    );
    let result = match (c_res, s_res) {
        (Ok(_), Ok(_)) => Ok(()),
        (Err(e), _) | (_, Err(e)) => Err(e),
    };
    let sent = sent.lock().unwrap().clone();
    (result, sent)
}

/// Returns the extension types of the ClientHello at the start of `bytes`.
fn hello_extensions(bytes: &[u8]) -> Vec<u16> {
    assert_eq!(bytes[..3], [0x16, 0x03, 0x01], "a ClientHello record");
    let be16 = |at: usize| usize::from(u16::from_be_bytes([bytes[at], bytes[at + 1]]));
    // Record header 5, handshake header 4, version 2, random 32.
    let mut at = 43;
    at += 1 + usize::from(bytes[at]); // session id
    at += 2 + be16(at); // cipher suites
    at += 1 + usize::from(bytes[at]); // compression
    let end = at + 2 + be16(at);
    at += 2;
    let mut out = Vec::new();
    while at < end {
        out.push(u16::try_from(be16(at)).unwrap());
        at += 4 + be16(at + 2);
    }
    out
}

#[tokio::test]
async fn every_handshake_of_a_shared_connector_keeps_the_client_hello_of_the_profile() {
    let tap = KeyTap::new();
    let profiles = tls::build(7, &needs("chrome-136", SERVER_TLS13), &tap)
        .expect("profiles")
        .expect("some");
    let mut shapes = Vec::new();
    for _ in 0..3 {
        let (result, sent) = handshake(&*profiles, &*profiles, "chrome-136", SERVER_TLS13).await;
        result.expect("the handshake succeeds");
        let mut ext = hello_extensions(&sent);
        ext.sort_unstable();
        shapes.push(ext);
    }
    // session_ticket (35) is in the first ClientHello, and no later one adds
    // pre_shared_key (41), because no server of the run issues a ticket.
    assert!(shapes[0].contains(&35), "{:?}", shapes[0]);
    for s in &shapes {
        assert!(!s.contains(&41), "{s:?}");
        assert_eq!(s, &shapes[0]);
    }
}

#[tokio::test]
async fn one_seed_gives_agents_the_same_trust_and_another_seed_does_not() {
    let tap = KeyTap::new();
    // The client side of agent one and the server side of agent two.
    let client = tls::build(11, &needs("firefox-139", SERVER_TLS12), &tap)
        .unwrap()
        .unwrap();
    let same = tls::build(11, &needs("firefox-139", SERVER_TLS12), &tap)
        .unwrap()
        .unwrap();
    let other = tls::build(12, &needs("firefox-139", SERVER_TLS12), &tap)
        .unwrap()
        .unwrap();
    let (ok, _) = handshake(&*client, &*same, "firefox-139", SERVER_TLS12).await;
    ok.expect("one seed, one trust anchor");
    let (refused, _) = handshake(&*client, &*other, "firefox-139", SERVER_TLS12).await;
    assert!(
        matches!(refused, Err(UpgradeError::Handshake(_))),
        "{refused:?}"
    );
    // A profile that the run does not build is unavailable, not a failure.
    let (missing, _) = handshake(&*client, &*same, "chrome-136", SERVER_TLS12).await;
    assert!(matches!(missing, Err(UpgradeError::Unavailable(_))));
}

// Independent decryption of a TLS 1.3 record stream with an NSS key log. ----

struct Len(usize);

impl hkdf::KeyType for Len {
    fn len(&self) -> usize {
        self.0
    }
}

/// HKDF-Expand-Label of RFC 8446 section 7.1 with an empty context.
fn expand_label(alg: hkdf::Algorithm, secret: &[u8], label: &str, len: usize) -> Vec<u8> {
    let prk = hkdf::Prk::new_less_safe(alg, secret);
    let full = format!("tls13 {label}");
    let length = u16::try_from(len).unwrap().to_be_bytes();
    let label_len = [u8::try_from(full.len()).unwrap()];
    let info: [&[u8]; 4] = [&length, &label_len, full.as_bytes(), &[0]];
    let okm = prk.expand(&info, Len(len)).unwrap();
    let mut out = vec![0u8; len];
    okm.fill(&mut out).unwrap();
    out
}

/// Looks up the secrets of the handshake whose ClientHello carries `random`.
fn secrets_of(keylog: &str, random: &[u8]) -> BTreeMap<String, Vec<u8>> {
    let random = hex(random);
    keylog
        .lines()
        .filter_map(|l| {
            let mut parts = l.split(' ');
            let (label, r, secret) = (parts.next()?, parts.next()?, parts.next()?);
            (r == random).then(|| (label.to_owned(), unhex(secret)))
        })
        .collect()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(text: &str) -> Vec<u8> {
    (0..text.len() / 2)
        .map(|i| u8::from_str_radix(&text[2 * i..2 * i + 2], 16).unwrap())
        .collect()
}

/// Decrypts the application data records of one direction of a TLS 1.3
/// stream. The first records use the handshake keys, as the encrypted server
/// flight does, and the rest the application keys. Returns `None` when a
/// record fails with both key sets.
fn decrypt_direction(
    stream: &[u8],
    suite: u16,
    handshake_secret: &[u8],
    traffic_secret: &[u8],
) -> Option<Vec<u8>> {
    let (hash, algorithm, key_len) = match suite {
        0x1301 => (hkdf::HKDF_SHA256, &aead::AES_128_GCM, 16),
        0x1302 => (hkdf::HKDF_SHA384, &aead::AES_256_GCM, 32),
        0x1303 => (hkdf::HKDF_SHA256, &aead::CHACHA20_POLY1305, 32),
        other => panic!("unexpected cipher suite {other:#x}"),
    };
    let keys: Vec<(aead::LessSafeKey, Vec<u8>)> = [handshake_secret, traffic_secret]
        .iter()
        .map(|s| {
            let key = expand_label(hash, s, "key", key_len);
            let iv = expand_label(hash, s, "iv", 12);
            (
                aead::LessSafeKey::new(aead::UnboundKey::new(algorithm, &key).unwrap()),
                iv,
            )
        })
        .collect();
    let (mut stage, mut seq, mut pos, mut out) = (0usize, 0u64, 0usize, Vec::new());
    while pos + 5 <= stream.len() {
        let length = usize::from(u16::from_be_bytes([stream[pos + 3], stream[pos + 4]]));
        let header = &stream[pos..pos + 5];
        let body = &stream[pos + 5..pos + 5 + length];
        pos += 5 + length;
        if header[0] != 23 {
            continue;
        }
        let plain = loop {
            let (key, iv) = &keys[stage];
            let mut nonce = [0u8; 12];
            nonce.copy_from_slice(iv);
            for (n, s) in nonce[4..].iter_mut().zip(seq.to_be_bytes()) {
                *n ^= s;
            }
            let mut buf = body.to_vec();
            let opened = key.open_in_place(
                aead::Nonce::assume_unique_for_key(nonce),
                aead::Aad::from(header),
                &mut buf,
            );
            match opened {
                Ok(p) => break p.to_vec(),
                Err(_) if stage == 0 => {
                    stage = 1;
                    seq = 0;
                }
                Err(_) => return None,
            }
        };
        seq += 1;
        let end = plain.iter().rposition(|b| *b != 0)?;
        // The last byte of the inner plaintext is the real content type.
        if plain[end] == 23 {
            out.extend_from_slice(&plain[..end]);
        }
    }
    Some(out)
}

/// Decrypts both directions of a TLS 1.3 connection that starts with its
/// ClientHello and its ServerHello.
fn decrypt_connection(
    keylog: &str,
    client_tls: &[u8],
    server_tls: &[u8],
) -> Option<(Vec<u8>, Vec<u8>)> {
    // ClientHello: record header (5), handshake header (4), version (2), random (32).
    let random = client_tls.get(11..43)?;
    // ServerHello: the cipher suite follows the session id.
    let sid_len = usize::from(*server_tls.get(43)?);
    let suite = u16::from_be_bytes([
        *server_tls.get(44 + sid_len)?,
        *server_tls.get(45 + sid_len)?,
    ]);
    let secrets = secrets_of(keylog, random);
    let get = |name: &str| secrets.get(name).map(Vec::as_slice);
    let c2s = decrypt_direction(
        client_tls,
        suite,
        get("CLIENT_HANDSHAKE_TRAFFIC_SECRET")?,
        get("CLIENT_TRAFFIC_SECRET_0")?,
    )?;
    let s2c = decrypt_direction(
        server_tls,
        suite,
        get("SERVER_HANDSHAKE_TRAFFIC_SECRET")?,
        get("SERVER_TRAFFIC_SECRET_0")?,
    )?;
    Some((c2s, s2c))
}

/// Flips the first digit of every secret, which no key derivation survives.
fn broken(keylog: &str) -> String {
    keylog
        .lines()
        .map(|l| {
            let (head, secret) = l.rsplit_once(' ').unwrap();
            let flipped = if secret.starts_with('0') { "1" } else { "0" };
            format!("{head} {flipped}{}", &secret[1..])
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn contains(haystack: &[u8], needle: &str) -> bool {
    haystack
        .windows(needle.len())
        .any(|w| w == needle.as_bytes())
}
