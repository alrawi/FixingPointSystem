// SPDX-License-Identifier: GPL-3.0-only
//! Runs every http2 scenario with both sides over the in memory duplex and
//! checks the frames that each side wrote, with a frame reader that does not
//! share code with the crate under test (`common/frames.rs`).
//!
//! The checks follow the acceptance of MOD-06: several requests in flight on
//! one connection with interleaved DATA, flow control with WINDOW_UPDATE, PING,
//! a graceful GOAWAY, and server push that stays disabled.

mod common;

use common::frames::{
    self, field, header_lists, split, split_client, Frame, ACK, DATA, END_STREAM, GOAWAY, HEADERS,
    PING, PUSH_PROMISE, SETTINGS, WINDOW_UPDATE,
};
use common::{run, scenarios, scenarios_with, tls_small, Run};
use fps_flow::{Scenario, Value};

const SETTINGS_ENABLE_PUSH: u16 = 0x2;
const SETTINGS_INITIAL_WINDOW_SIZE: u16 = 0x4;
const MAX_STREAM: u32 = 0x7FFF_FFFF;

fn var(s: &Scenario, name: &str) -> i64 {
    match s.vars.get(name) {
        Some(Value::Int(n)) => *n,
        other => panic!("variable {name} is {other:?}"),
    }
}

fn text_var(s: &Scenario, name: &str) -> String {
    match s.vars.get(name) {
        Some(Value::Str(t)) => t.clone(),
        other => panic!("variable {name} is {other:?}"),
    }
}

fn is_browse(name: &str) -> bool {
    name.ends_with("_browse")
}

fn is_tls(name: &str) -> bool {
    name.starts_with("https_")
}

/// The frames of both sides of a run that succeeded.
struct Frames {
    client: Vec<Frame>,
    server: Vec<Frame>,
}

fn frames_of(name: &str, r: &Run) -> Frames {
    assert!(
        r.client.is_ok() && r.server.is_ok(),
        "{name} client: {:?} server: {:?}",
        r.client,
        r.server
    );
    assert!(r.server.is_ok(), "{name} server: {:?}", r.server);
    Frames {
        client: split_client(&r.c2s),
        server: split(&r.s2c),
    }
}

/// Checks that hold for every scenario.
fn check_common(name: &str, s: &Scenario, f: &Frames) {
    // Both sides open with a SETTINGS frame, and the client SETTINGS disables
    // server push, which is SETTINGS_ENABLE_PUSH set to 0.
    for (who, frames) in [("client", &f.client), ("server", &f.server)] {
        assert_eq!(frames[0].ty, SETTINGS, "{name}: {who} opens with SETTINGS");
        assert!(
            !frames[0].has(ACK),
            "{name}: {who} first SETTINGS is no ACK"
        );
        assert_eq!(frames[0].stream, 0);
    }
    let push = |frames: &[Frame]| -> Vec<u32> {
        frames[0]
            .settings()
            .iter()
            .filter(|(id, _)| *id == SETTINGS_ENABLE_PUSH)
            .map(|(_, v)| *v)
            .collect()
    };
    assert_eq!(push(&f.client), vec![0], "{name}: the client disables push");
    assert!(
        push(&f.server).iter().all(|v| *v == 0),
        "{name}: the server never enables push"
    );
    // No PUSH_PROMISE in either direction.
    for frames in [&f.client, &f.server] {
        assert!(
            frames.iter().all(|x| x.ty != PUSH_PROMISE),
            "{name}: a PUSH_PROMISE frame"
        );
    }
    // Each side acknowledges the SETTINGS of the other exactly once.
    for frames in [&f.client, &f.server] {
        let acks = frames
            .iter()
            .filter(|x| x.ty == SETTINGS && x.has(ACK))
            .count();
        assert_eq!(acks, 1, "{name}: SETTINGS ACKs");
        assert!(frames
            .iter()
            .filter(|x| x.ty == SETTINGS && x.has(ACK))
            .all(|x| x.payload.is_empty()));
    }
    // One PING and its acknowledgment with the same 8 octets, in opposite directions.
    let (pinger, ponger) = if is_browse(name) {
        (&f.client, &f.server)
    } else {
        (&f.server, &f.client)
    };
    let pings: Vec<&Frame> = pinger
        .iter()
        .filter(|x| x.ty == PING && !x.has(ACK))
        .collect();
    let pongs: Vec<&Frame> = ponger
        .iter()
        .filter(|x| x.ty == PING && x.has(ACK))
        .collect();
    assert_eq!((pings.len(), pongs.len()), (1, 1), "{name}: PING");
    assert_eq!(pings[0].payload.len(), 8);
    assert_eq!(pings[0].payload, pongs[0].payload, "{name}: PING payload");
    assert_eq!(pings[0].stream, 0);
    // A graceful shutdown: the first GOAWAY names the largest stream id, the
    // second the last stream that the server processed, both with NO_ERROR.
    let goaways: Vec<&Frame> = f.server.iter().filter(|x| x.ty == GOAWAY).collect();
    assert_eq!(goaways.len(), 2, "{name}: GOAWAY frames");
    assert_eq!(goaways[0].goaway(), (MAX_STREAM, 0), "{name}");
    let last_stream = f
        .client
        .iter()
        .filter(|x| x.ty == HEADERS)
        .map(|x| x.stream)
        .max()
        .expect("requests");
    assert_eq!(goaways[1].goaway(), (last_stream, 0), "{name}");
    assert!(
        f.server.last().expect("frames").ty == GOAWAY,
        "{name}: GOAWAY ends the connection"
    );
    assert!(f.client.iter().all(|x| x.ty != GOAWAY));
    // WINDOW_UPDATE frames carry an increment of 1 to 2^31 - 1, and the total
    // credit of the connection stays within 2^31 - 1.
    for frames in [&f.client, &f.server] {
        let mut connection: u64 = 65_535;
        for w in frames.iter().filter(|x| x.ty == WINDOW_UPDATE) {
            let inc = w.increment();
            assert!(inc >= 1, "{name}: a zero WINDOW_UPDATE");
            if w.stream == 0 {
                connection += u64::from(inc);
            }
        }
        assert!(
            connection <= u64::from(MAX_STREAM),
            "{name}: window overflow"
        );
    }
    // Client streams are odd and rise, and every request has the pseudo
    // headers of its scheme.
    let lists = header_lists(&f.client);
    let ids: Vec<u32> = lists.iter().map(|(id, _)| *id).collect();
    assert!(ids.iter().all(|id| id % 2 == 1), "{name}: {ids:?}");
    assert!(ids.windows(2).all(|w| w[0] < w[1]), "{name}: {ids:?}");
    let scheme = if is_tls(name) { "https" } else { "http" };
    let host = text_var(s, "host");
    for (_, list) in &lists {
        assert_eq!(field(list, ":scheme"), Some(scheme), "{name}");
        assert_eq!(field(list, ":authority"), Some(host.as_str()), "{name}");
        assert!(field(list, ":path").is_some_and(|p| p.starts_with('/')));
        // Pseudo headers come first, as RFC 9113 section 8.3 requires.
        let first_regular = list.iter().position(|(n, _)| !n.starts_with(':'));
        let last_pseudo = list.iter().rposition(|(n, _)| n.starts_with(':'));
        assert!(first_regular > last_pseudo, "{name}: pseudo headers first");
    }
    // Every response is on a stream that the client opened, and says 200 or 201.
    let answers = header_lists(&f.server);
    assert_eq!(
        answers.len(),
        lists.len(),
        "{name}: one response per request"
    );
    for ((req_id, _), (rsp_id, list)) in lists.iter().zip(&answers) {
        assert_eq!(req_id, rsp_id, "{name}");
        assert!(matches!(field(list, ":status"), Some("200" | "201")));
    }
    // The server never sends DATA on a stream that is not a request stream.
    for d in f.server.iter().filter(|x| x.ty == DATA) {
        assert!(
            ids.contains(&d.stream),
            "{name}: DATA on stream {}",
            d.stream
        );
    }
}

fn data_of(frames: &[Frame], stream: u32) -> usize {
    frames
        .iter()
        .filter(|x| x.ty == DATA && x.stream == stream)
        .map(|x| x.payload.len())
        .sum()
}

fn browse(name: &str, s: &Scenario, f: &Frames) {
    let rounds = var(s, "rounds") as usize;
    let chunk = var(s, "chunk_size") as usize;
    let (lo, hi) = (var(s, "tail_min") as usize, var(s, "tail_max") as usize);
    let streams = [3u32, 5, 7, 9, 11, 13];
    let requests: Vec<&Frame> = f.client.iter().filter(|x| x.ty == HEADERS).collect();
    assert_eq!(
        requests.iter().map(|x| x.stream).collect::<Vec<_>>(),
        [1, 3, 5, 7, 9, 11, 13],
        "{name}"
    );
    assert!(requests.iter().all(|x| x.has(END_STREAM)));
    // The six object requests follow each other without a frame in between,
    // which is six requests in flight before the first answer.
    let at = f
        .client
        .iter()
        .position(|x| x.ty == HEADERS && x.stream == 3)
        .expect("request 3");
    let burst: Vec<u32> = f.client[at..at + 6].iter().map(|x| x.stream).collect();
    assert_eq!(burst, streams, "{name}");
    assert!(f.client[at..at + 6].iter().all(|x| x.ty == HEADERS));
    // The answers: the page on stream 1, and six HEADERS frames before the
    // first DATA frame of an object.
    let first_object_data = f
        .server
        .iter()
        .position(|x| x.ty == DATA && x.stream >= 3)
        .expect("object data");
    let object_headers = f
        .server
        .iter()
        .enumerate()
        .filter(|(_, x)| x.ty == HEADERS && x.stream >= 3)
        .map(|(i, _)| i)
        .collect::<Vec<_>>();
    assert_eq!(object_headers.len(), 6, "{name}");
    assert!(object_headers.iter().all(|i| *i < first_object_data));
    let page = var(s, "page_size") as usize;
    assert_eq!(data_of(&f.server, 1), page, "{name}: page size");
    let answers = header_lists(&f.server);
    assert_eq!(
        field(&answers[0].1, "content-length"),
        Some(page.to_string().as_str())
    );
    // The DATA frames of the objects: `rounds` rounds with one frame per
    // stream in turn, so the streams interleave, and then the last frame of
    // each stream with END_STREAM.
    let object_data: Vec<&Frame> = f
        .server
        .iter()
        .filter(|x| x.ty == DATA && x.stream >= 3)
        .collect();
    assert_eq!(object_data.len(), 6 * (rounds + 1), "{name}");
    for (i, d) in object_data[..6 * rounds].iter().enumerate() {
        assert_eq!(d.stream, streams[i % 6], "{name}: round order");
        assert_eq!(d.payload.len(), chunk);
        assert!(!d.has(END_STREAM));
    }
    if rounds > 0 {
        let interleaved = object_data[..6 * rounds]
            .windows(2)
            .all(|w| w[0].stream != w[1].stream);
        assert!(interleaved, "{name}: DATA frames of one stream in a row");
    }
    for (i, d) in object_data[6 * rounds..].iter().enumerate() {
        assert_eq!(d.stream, streams[i]);
        assert!(d.has(END_STREAM), "{name}: the last frame ends the stream");
        assert!((lo..=hi).contains(&d.payload.len()), "{name}: tail size");
    }
    // The page ends with END_STREAM on its last frame.
    let page_frames: Vec<&Frame> = f
        .server
        .iter()
        .filter(|x| x.ty == DATA && x.stream == 1)
        .collect();
    assert!(page_frames.last().is_some_and(|d| d.has(END_STREAM)) || page == 0);
    // The client tops up the connection window with its own WINDOW_UPDATE.
    let bump = var(s, "window_bump") as u32;
    assert!(f
        .client
        .iter()
        .any(|x| x.ty == WINDOW_UPDATE && x.stream == 0 && x.increment() == bump));
    // The server announces 128 concurrent streams, so six are within the limit.
    let max = f.server[0]
        .settings()
        .iter()
        .find(|(id, _)| *id == 0x3)
        .map(|(_, v)| *v);
    assert!(
        max.is_some_and(|m| m >= 6),
        "{name}: max concurrent streams"
    );
}

fn upload(name: &str, s: &Scenario, f: &Frames) {
    let waits = var(s, "window_waits") as usize;
    let round_frames = var(s, "round_frames") as usize;
    let frame_size = var(s, "frame_size") as usize;
    let window = var(s, "window") as usize;
    let tail = var(s, "tail_size") as usize;
    let requests: Vec<&Frame> = f.client.iter().filter(|x| x.ty == HEADERS).collect();
    assert_eq!(
        requests.iter().map(|x| x.stream).collect::<Vec<_>>(),
        [1, 3],
        "{name}"
    );
    assert!(requests.iter().all(|x| !x.has(END_STREAM)));
    let lists = header_lists(&f.client);
    assert!(lists
        .iter()
        .all(|(_, l)| field(l, ":method") == Some("POST")));
    // The client DATA: rounds of one frame per stream in turn, then the rest of each body.
    let data: Vec<&Frame> = f.client.iter().filter(|x| x.ty == DATA).collect();
    let round_total = 2 * waits * round_frames;
    for (i, d) in data[..round_total].iter().enumerate() {
        assert_eq!(d.stream, [1, 3][i % 2], "{name}: interleaving");
        assert_eq!(d.payload.len(), frame_size);
        assert!(!d.has(END_STREAM));
    }
    let rest = &data[round_total..];
    let per_stream = tail.div_ceil(frame_size);
    assert_eq!(rest.len(), 2 * per_stream, "{name}: tail frames");
    for (n, stream) in [1u32, 3].into_iter().enumerate() {
        let mine = &rest[n * per_stream..(n + 1) * per_stream];
        assert!(mine.iter().all(|d| d.stream == stream));
        assert!(mine[..per_stream - 1].iter().all(|d| !d.has(END_STREAM)));
        assert!(mine[per_stream - 1].has(END_STREAM), "{name}");
        assert_eq!(mine.iter().map(|d| d.payload.len()).sum::<usize>(), tail);
        assert_eq!(
            data_of(&f.client, stream),
            waits * round_frames * frame_size + tail
        );
    }
    // Flow control: the server announces the stream window, and a stream never
    // sent more than the window plus the WINDOW_UPDATE credit of the server.
    let announced = f.server[0]
        .settings()
        .iter()
        .find(|(id, _)| *id == SETTINGS_INITIAL_WINDOW_SIZE)
        .map(|(_, v)| *v as usize);
    assert_eq!(announced, Some(window), "{name}: announced window");
    let mut connection_credit: usize = 65_535;
    for w in f.server.iter().filter(|x| x.ty == WINDOW_UPDATE) {
        if w.stream == 0 {
            connection_credit += w.increment() as usize;
        }
    }
    for stream in [1u32, 3] {
        let credit: usize = f
            .server
            .iter()
            .filter(|x| x.ty == WINDOW_UPDATE && x.stream == stream)
            .map(|x| x.increment() as usize)
            .sum();
        assert_eq!(credit, waits * window, "{name}: credit of stream {stream}");
        assert!(data_of(&f.client, stream) <= window + credit, "{name}");
    }
    let sent = data_of(&f.client, 1) + data_of(&f.client, 3);
    assert!(sent <= connection_credit, "{name}: connection credit");
    // The server answers both requests with 201 and no body.
    let answers = header_lists(&f.server);
    assert!(answers
        .iter()
        .all(|(_, l)| field(l, ":status") == Some("201")));
    assert!(f
        .server
        .iter()
        .filter(|x| x.ty == HEADERS)
        .all(|x| x.has(END_STREAM)));
    assert!(f.server.iter().all(|x| x.ty != DATA), "{name}");
}

fn check(name: &str, s: &Scenario, r: &Run) {
    let f = frames_of(name, r);
    check_common(name, s, &f);
    if is_browse(name) {
        browse(name, s, &f);
    } else {
        upload(name, s, &f);
    }
}

#[tokio::test(start_paused = true)]
async fn every_scenario_runs_with_its_default_variables() {
    let defaults = scenarios();
    assert_eq!(defaults.len(), 4);
    let small = scenarios_with(&tls_small());
    for ((name, s), (_, small_s)) in defaults.iter().zip(&small) {
        // The cleartext scenarios run with their defaults, the https scenarios
        // with the small variant, see tls_small.
        let s = if is_tls(name) { small_s } else { s };
        for session_id in 1..=5 {
            let r = run(s, 42, session_id).await;
            check(name, s, &r);
        }
    }
}

#[tokio::test(start_paused = true)]
async fn the_variables_change_the_exchange() {
    // Smaller bodies, more rounds, and a different window.
    let cleartext = [
        ("page_size", Value::Int(5000)),
        ("chunk_size", Value::Int(1000)),
        ("rounds", Value::Int(5)),
        ("tail_min", Value::Int(10)),
        ("tail_max", Value::Int(20)),
        ("window", Value::Int(8000)),
        ("frame_size", Value::Int(2000)),
        ("round_frames", Value::Int(4)),
        ("window_waits", Value::Int(3)),
        ("tail_size", Value::Int(7000)),
    ];
    let tls = [
        ("page_size", Value::Int(3000)),
        ("chunk_size", Value::Int(800)),
        ("rounds", Value::Int(2)),
        ("tail_min", Value::Int(10)),
        ("tail_max", Value::Int(20)),
        ("window", Value::Int(3000)),
        ("frame_size", Value::Int(1000)),
        ("round_frames", Value::Int(3)),
        ("window_waits", Value::Int(1)),
        ("tail_size", Value::Int(2000)),
    ];
    for (overrides, tls_run) in [(&cleartext, false), (&tls, true)] {
        for (name, s) in scenarios_with(overrides) {
            if is_tls(&name) != tls_run {
                continue;
            }
            for session_id in 1..=3 {
                let r = run(&s, 7, session_id).await;
                check(&name, &s, &r);
            }
        }
    }
}

#[tokio::test(start_paused = true)]
async fn rounds_and_waits_may_be_zero() {
    let overrides = [
        ("rounds", Value::Int(0)),
        ("window_waits", Value::Int(0)),
        ("tail_size", Value::Int(100)),
    ];
    for (name, s) in scenarios_with(&overrides) {
        let r = run(&s, 3, 1).await;
        check(&name, &s, &r);
    }
}

#[tokio::test(start_paused = true)]
async fn every_session_draws_its_own_paths_and_sizes() {
    let all = scenarios();
    for (name, s) in all.iter().filter(|(n, _)| is_browse(n)) {
        let paths = |r: &Run| -> Vec<String> {
            header_lists(&frames_of(name, r).client)
                .into_iter()
                .map(|(_, l)| field(&l, ":path").unwrap_or_default().to_owned())
                .collect()
        };
        let a = run(s, 42, 1).await;
        let b = run(s, 42, 2).await;
        assert_ne!(paths(&a), paths(&b), "{name}");
        // The tails differ too, because the server draws them.
        let tails = |r: &Run| -> Vec<usize> {
            frames_of(name, r)
                .server
                .iter()
                .filter(|x| x.ty == DATA && x.has(END_STREAM) && x.stream >= 3)
                .map(|x| x.payload.len())
                .collect()
        };
        assert_ne!(tails(&a), tails(&b), "{name}");
    }
}

#[tokio::test(start_paused = true)]
async fn the_frame_reader_rejects_a_cut_frame() {
    // A guard for the reader itself: a stream that ends inside a frame fails.
    let mut bytes = frames::PREFACE.to_vec();
    bytes.extend_from_slice(&[0, 0, 6, 4, 0, 0, 0, 0, 0, 0, 3]);
    let result = std::panic::catch_unwind(|| split_client(&bytes));
    assert!(result.is_err());
}

/// The sessions that the estimate averages over. Sizes that the session
/// generator draws vary from session to session, so one session says little
/// about the mean that the controller needs.
const ESTIMATE_SESSIONS: u64 = 20;

/// A session of each scenario at the default parameters moves the payload
/// bytes of `expected_bytes` in the manifest, both directions of the stream
/// above any TLS layer, within 2 percent. A scenario whose size depends on the
/// session generator is averaged over [`ESTIMATE_SESSIONS`] session ids. The
/// controller turns byte weights into session shares with these estimates, so a
/// change of a template or a fixture that alters the size of a session must
/// change the manifest.
#[tokio::test(start_paused = true)]
async fn default_parameters_move_the_expected_bytes() {
    let doc: serde_yaml::Value = serde_yaml::from_str(
        &std::fs::read_to_string(common::module_dir().join("manifest.yaml")).unwrap(),
    )
    .unwrap();
    let all = common::scenarios();
    let mut bad = Vec::new();
    if let Some(keys) = doc["expected_bytes"].as_mapping() {
        for key in keys.keys() {
            let key = key.as_str().unwrap_or_default();
            if !all.iter().any(|(name, _)| name == key) {
                bad.push(format!("{key}: an estimate for no scenario"));
            }
        }
    }
    for (name, s) in all {
        let mut sizes = Vec::new();
        for id in 1..=ESTIMATE_SESSIONS {
            let r = common::run(&s, common::SEED, id).await;
            assert!(r.client.is_ok(), "{name}: {:?}", r.client);
            assert!(r.server.is_ok(), "{name}: {:?}", r.server);
            sizes.push(r.c2s.len() + r.s2c.len());
        }
        let mean = sizes.iter().sum::<usize>() as f64 / sizes.len() as f64;
        match doc["expected_bytes"][name.as_str()].as_f64() {
            Some(want) if (mean - want).abs() / want < 0.02 => {}
            Some(want) => bad.push(format!(
                "{name}: {mean:.0} bytes against the estimate {want}"
            )),
            None => bad.push(format!("{name}: {mean:.0} bytes and no estimate")),
        }
        eprintln!(
            "MEASURED {name} mean {mean:.1} min {} max {}",
            sizes.iter().min().unwrap(),
            sizes.iter().max().unwrap()
        );
    }
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}
