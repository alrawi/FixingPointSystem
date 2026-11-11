// SPDX-License-Identifier: GPL-3.0-only
//! Interpreter tests of the hls scenarios. Every scenario runs as a scenario
//! instance, both sides of every flow against each other, over an in memory
//! network on a paused clock, so the gaps between requests are exact. The bytes
//! of each connection are parsed with the framing of RFC 9112 and the playlist
//! rules of RFC 8216.
//!
//! The tests check the acceptance of the module first: the segment requests
//! follow the segment cadence, and the bytes that the segments carry follow
//! the bit rate of the rung that the player picked.

mod common;

use std::time::Duration;

use common::{run, scenarios, scenarios_with, Conn, Run, SEED};
use fps_flow::Value;
use tokio::time::Instant;

/// A ladder whose segments stay small, with the bit rates of three rungs.
const SMALL_LADDER: &str = "80000@426x240,160000@640x360,320000@960x540";
const SMALL_RATES: [u64; 3] = [80_000, 160_000, 320_000];

fn small() -> Vec<(&'static str, Value)> {
    vec![
        ("ladder", Value::Str(SMALL_LADDER.into())),
        ("segment_ms", Value::Int(1000)),
    ]
}

/// One request of a client.
#[derive(Debug)]
struct Request {
    target: String,
    connection: Option<String>,
    host: String,
}

fn requests(bytes: &[u8]) -> Vec<Request> {
    let text = std::str::from_utf8(bytes).expect("requests are text");
    assert!(
        text.is_empty() || text.ends_with("\r\n\r\n"),
        "request framing"
    );
    text.split("\r\n\r\n")
        .filter(|m| !m.is_empty())
        .map(|m| {
            let mut lines = m.split("\r\n");
            let start = lines.next().unwrap();
            let mut parts = start.split(' ');
            assert_eq!(parts.next(), Some("GET"), "{start}");
            let target = parts.next().unwrap().to_string();
            assert_eq!(parts.next(), Some("HTTP/1.1"), "{start}");
            let header = |name: &str| {
                m.split("\r\n").skip(1).find_map(|l| {
                    let (n, v) = l.split_once(": ")?;
                    n.eq_ignore_ascii_case(name).then(|| v.to_string())
                })
            };
            Request {
                target,
                connection: header("connection"),
                host: header("host").expect("Host"),
            }
        })
        .collect()
}

/// One response of a server.
#[derive(Debug)]
struct Response {
    status: u16,
    content_type: String,
    connection: Option<String>,
    body: Vec<u8>,
}

fn responses(bytes: &[u8]) -> Vec<Response> {
    let mut out = Vec::new();
    let mut rest = bytes;
    while !rest.is_empty() {
        let end = rest
            .windows(4)
            .position(|w| w == b"\r\n\r\n")
            .expect("end of header block");
        let head = std::str::from_utf8(&rest[..end]).unwrap();
        let mut lines = head.split("\r\n");
        let status: u16 = lines
            .next()
            .unwrap()
            .strip_prefix("HTTP/1.1 ")
            .expect("status line")[..3]
            .parse()
            .unwrap();
        let header = |name: &str| {
            head.split("\r\n").skip(1).find_map(|l| {
                let (n, v) = l.split_once(": ")?;
                n.eq_ignore_ascii_case(name).then(|| v.to_string())
            })
        };
        let length: usize = header("content-length")
            .expect("Content-Length")
            .parse()
            .unwrap();
        let body = rest[end + 4..end + 4 + length].to_vec();
        out.push(Response {
            status,
            content_type: header("content-type").expect("Content-Type"),
            connection: header("connection"),
            body,
        });
        rest = &rest[end + 4 + length..];
    }
    out
}

fn text(body: &[u8]) -> &str {
    std::str::from_utf8(body).expect("playlists are text")
}

fn uris(playlist: &str) -> Vec<&str> {
    playlist
        .lines()
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .collect()
}

/// The size that a segment of the rung has, as the module defines it.
fn segment_size(rate: u64, segment_ms: u64) -> usize {
    let bytes = rate * segment_ms / 8000;
    (((bytes + 94) / 188).max(3) * 188) as usize
}

/// Checks that a body is a transport stream: whole packets, a sync byte on each, and a PAT first.
fn assert_transport_stream(body: &[u8]) {
    assert_eq!(body.len() % 188, 0, "whole packets");
    for (i, p) in body.chunks(188).enumerate() {
        assert_eq!(p[0], 0x47, "sync byte of packet {i}");
    }
    assert_eq!(&body[1..3], &[0x40, 0x00], "the PAT opens the segment");
}

fn gaps(times: &[Instant]) -> Vec<Duration> {
    times.windows(2).map(|w| w[1] - w[0]).collect()
}

async fn run_one(name: &str, s: &fps_flow::Scenario) -> Run {
    let r = run(s, SEED, 1).await;
    r.result.as_ref().unwrap_or_else(|e| panic!("{name}: {e}"));
    assert!(
        r.records.iter().all(|x| x.failed.is_none()),
        "{name}: a side failed"
    );
    r
}

/// The request times of the segment requests on one connection.
fn segment_request_times(c: &Conn, reqs: &[Request]) -> Vec<Instant> {
    let events = c.c2s_events();
    assert_eq!(events.len(), reqs.len(), "one write for each request");
    reqs.iter()
        .zip(events)
        .filter(|(r, _)| r.target.ends_with(".ts"))
        .map(|(_, (t, _))| t)
        .collect()
}

#[tokio::test(start_paused = true)]
async fn every_scenario_completes_with_both_sides() {
    for (name, s) in scenarios_with(&small()) {
        let r = run_one(&name, &s).await;
        let sides = r.records.len();
        assert!(sides >= 2, "{name}: {sides} sides");
    }
}

#[tokio::test(start_paused = true)]
async fn vod_ladder_walks_up_the_ladder_on_one_connection() {
    let (_, s) = scenarios_with(&small())
        .into_iter()
        .find(|(n, _)| n == "vod_ladder")
        .unwrap();
    let r = run_one("vod_ladder", &s).await;
    let conns = r.net.conns();
    assert_eq!(conns.len(), 1, "one keepalive connection");
    let reqs = requests(&conns[0].c2s());
    let resps = responses(&conns[0].s2c());
    // The master playlist, then three stages of a media playlist and four segments.
    assert_eq!(reqs.len(), 1 + 3 * 5);
    assert_eq!(resps.len(), reqs.len());
    assert!(reqs.iter().all(|q| q.host == "video.example.test"));
    assert!(reqs.iter().all(|q| q.connection.is_none()), "keepalive");
    assert!(resps
        .iter()
        .all(|p| p.status == 200 && p.connection.is_none()));
    assert_eq!(reqs[0].target, "/master.m3u8");
    let master = text(&resps[0].body);
    assert!(master.starts_with("#EXTM3U\n"));
    assert_eq!(
        uris(master),
        ["v0/index.m3u8", "v1/index.m3u8", "v2/index.m3u8"]
    );
    for (stage, rate) in SMALL_RATES.iter().enumerate() {
        let first = 1 + stage * 5;
        assert_eq!(reqs[first].target, format!("/v{stage}/index.m3u8"));
        let playlist = text(&resps[first].body);
        assert!(
            playlist.contains("#EXT-X-PLAYLIST-TYPE:VOD") && playlist.contains("#EXT-X-ENDLIST")
        );
        assert!(playlist.contains("#EXT-X-TARGETDURATION:1\n"));
        assert_eq!(
            uris(playlist).len(),
            12,
            "the VOD playlist names every segment"
        );
        for k in 0..4 {
            let n = stage * 4 + k;
            let (q, p) = (&reqs[first + 1 + k], &resps[first + 1 + k]);
            assert_eq!(q.target, format!("/v{stage}/seg{n:05}.ts"));
            assert_eq!(p.content_type, "video/mp2t");
            assert_eq!(p.body.len(), segment_size(*rate, 1000));
            assert_transport_stream(&p.body);
        }
    }
}

#[tokio::test(start_paused = true)]
async fn vod_ladder_fetches_segments_at_the_segment_cadence() {
    let (_, s) = scenarios_with(&small())
        .into_iter()
        .find(|(n, _)| n == "vod_ladder")
        .unwrap();
    let r = run_one("vod_ladder", &s).await;
    let c = &r.net.conns()[0];
    let reqs = requests(&c.c2s());
    let times = segment_request_times(c, &reqs);
    assert_eq!(times.len(), 12);
    // The paused clock makes the schedule exact, also across the media playlist fetch of a stage.
    for g in gaps(&times) {
        assert_eq!(g, Duration::from_millis(1000));
    }
}

#[tokio::test(start_paused = true)]
async fn vod_ladder_bytes_follow_the_bit_rate_of_the_rung() {
    let (_, s) = scenarios_with(&small())
        .into_iter()
        .find(|(n, _)| n == "vod_ladder")
        .unwrap();
    let r = run_one("vod_ladder", &s).await;
    let c = &r.net.conns()[0];
    let reqs = requests(&c.c2s());
    let resps = responses(&c.s2c());
    // The share of each rung in the segment bytes equals its share in the ladder plan, and the
    // mean bit rate of the playback equals the mean bit rate of the rungs within 5%.
    let mut per_rung = [0usize; 3];
    for (q, p) in reqs.iter().zip(&resps) {
        if q.target.ends_with(".ts") {
            let rung: usize = q.target[2..3].parse().unwrap();
            per_rung[rung] += p.body.len();
        }
    }
    let total: usize = per_rung.iter().sum();
    let want_total: f64 = SMALL_RATES
        .iter()
        .map(|r| *r as f64 * 4.0 * 1.0 / 8.0)
        .sum();
    assert!(
        (total as f64 - want_total).abs() / want_total < 0.05,
        "{total} vs {want_total}"
    );
    for (i, rate) in SMALL_RATES.iter().enumerate() {
        let share = per_rung[i] as f64 / total as f64;
        let want = *rate as f64 / SMALL_RATES.iter().sum::<u64>() as f64;
        assert!(
            (share - want).abs() / want < 0.05,
            "rung {i}: {share} vs {want}"
        );
    }
    let rate = total as f64 * 8.0 / 12.0;
    let mean = SMALL_RATES.iter().sum::<u64>() as f64 / 3.0;
    assert!((rate - mean).abs() / mean < 0.05, "{rate} vs {mean}");
}

#[tokio::test(start_paused = true)]
async fn vod_segments_opens_one_connection_per_segment_at_the_cadence() {
    let (_, s) = scenarios_with(&small())
        .into_iter()
        .find(|(n, _)| n == "vod_segments")
        .unwrap();
    let r = run_one("vod_segments", &s).await;
    let playlists = r.conns_of("playlists");
    let segments = r.conns_of("segment");
    assert_eq!(playlists.len(), 1);
    assert_eq!(segments.len(), 12, "one connection per segment");
    assert!(segments.iter().all(|c| c.port == 8092));
    assert_eq!(playlists[0].port, 8091);
    // The playlist connection fetches the master playlist and one media playlist per stage.
    let preqs = requests(&playlists[0].c2s());
    let targets: Vec<&str> = preqs.iter().map(|q| q.target.as_str()).collect();
    assert_eq!(
        targets,
        [
            "/master.m3u8",
            "/v0/index.m3u8",
            "/v1/index.m3u8",
            "/v2/index.m3u8"
        ]
    );
    // Each segment connection carries one GET with Connection close and one 200 with a transport
    // stream of the rung that the stage picked.
    for (n, c) in segments.iter().enumerate() {
        let q = requests(&c.c2s());
        let p = responses(&c.s2c());
        assert_eq!((q.len(), p.len()), (1, 1), "segment {n}");
        let stage = n / 4;
        assert_eq!(q[0].target, format!("/v{stage}/seg{n:05}.ts"));
        assert_eq!(q[0].connection.as_deref(), Some("close"));
        assert_eq!(p[0].connection.as_deref(), Some("close"));
        assert_eq!(p[0].body.len(), segment_size(SMALL_RATES[stage], 1000));
        assert_transport_stream(&p[0].body);
    }
    // The starts of the connections are the segment cadence, which is what the temporal check of
    // the realism runner compares on the capture.
    let opened: Vec<Instant> = segments.iter().map(|c| c.opened).collect();
    for g in gaps(&opened) {
        assert_eq!(g, Duration::from_millis(1000));
    }
}

#[tokio::test(start_paused = true)]
async fn live_refresh_slides_the_window_one_segment_per_cycle() {
    let (_, s) = scenarios_with(&small())
        .into_iter()
        .find(|(n, _)| n == "live_refresh")
        .unwrap();
    let r = run_one("live_refresh", &s).await;
    let conns = r.net.conns();
    assert_eq!(conns.len(), 1);
    let reqs = requests(&conns[0].c2s());
    let resps = responses(&conns[0].s2c());
    assert_eq!(reqs.len(), 1 + 8 * 2);
    assert_eq!(reqs[0].target, "/live/master.m3u8");
    let mut reloads = Vec::new();
    let events = conns[0].c2s_events();
    for cycle in 0..8usize {
        let (q, p) = (&reqs[1 + cycle * 2], &resps[1 + cycle * 2]);
        assert_eq!(q.target, "/live/v1/index.m3u8");
        let playlist = text(&p.body);
        assert!(!playlist.contains("ENDLIST") && !playlist.contains("PLAYLIST-TYPE"));
        assert!(playlist.contains(&format!("#EXT-X-MEDIA-SEQUENCE:{cycle}\n")));
        let names = uris(playlist);
        assert_eq!(names.len(), 3, "a window of three segments");
        // The newest segment of the window is the one that the player fetches.
        let newest = cycle + 2;
        assert_eq!(names[2], format!("seg{newest:05}.ts"));
        assert_eq!(
            reqs[2 + cycle * 2].target,
            format!("/live/v1/seg{newest:05}.ts")
        );
        let seg = &resps[2 + cycle * 2];
        assert_eq!(seg.body.len(), segment_size(SMALL_RATES[1], 1000));
        assert_transport_stream(&seg.body);
        reloads.push(events[1 + cycle * 2].0);
    }
    for g in gaps(&reloads) {
        assert_eq!(g, Duration::from_millis(1000));
    }
}

#[tokio::test(start_paused = true)]
async fn default_parameters_move_the_expected_bytes() {
    // The defaults of the manifest, with real segment sizes: three rungs at six seconds.
    let all = scenarios();
    let doc: serde_yaml::Value = serde_yaml::from_str(
        &std::fs::read_to_string(common::module_dir().join("manifest.yaml")).unwrap(),
    )
    .unwrap();
    for (name, s) in all {
        let r = run_one(&name, &s).await;
        let moved: usize = r
            .net
            .conns()
            .iter()
            .map(|c| c.c2s().len() + c.s2c().len())
            .sum();
        let want = doc["expected_bytes"][name.as_str()].as_f64().unwrap();
        assert!(
            (moved as f64 - want).abs() / want < 0.02,
            "{name}: {moved} bytes against the estimate {want}"
        );
    }
}

#[tokio::test(start_paused = true)]
async fn a_ladder_with_one_rung_plays_every_stage_on_it() {
    let (_, s) = scenarios_with(&[
        ("ladder", Value::Str("80000@426x240".into())),
        ("segment_ms", Value::Int(1000)),
    ])
    .into_iter()
    .find(|(n, _)| n == "vod_ladder")
    .unwrap();
    let r = run_one("vod_ladder", &s).await;
    let reqs = requests(&r.net.conns()[0].c2s());
    assert!(
        reqs[1..].iter().all(|q| q.target.starts_with("/v0/")),
        "{reqs:?}"
    );
}

#[tokio::test(start_paused = true)]
async fn a_bad_ladder_fails_the_session_with_the_extension_reason() {
    let (_, s) = scenarios_with(&[("ladder", Value::Str("nonsense".into()))])
        .into_iter()
        .find(|(n, _)| n == "vod_ladder")
        .unwrap();
    let r = run(&s, SEED, 1).await;
    let e = r.result.expect_err("a ladder that does not parse fails");
    assert!(e.to_string().contains("ladder"), "{e}");
}
