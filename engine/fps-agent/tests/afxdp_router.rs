// SPDX-License-Identifier: GPL-3.0-only
//! A forced AF_XDP run through the agent reaches its peers behind a kernel
//! router with no neighbor entry configured (AGT-26).
//!
//! Two network namespaces make the layout of `fps-datapath/tests/router.rs`
//! (TRN-35), with the agent as the own stack:
//!
//! ```text
//!   own namespace (the agent, AF_XDP on oa and ob)          router namespace
//!     client pool 10.0.0.0/24, gateway 10.0.0.1  oa ---- ra  10.0.0.1  (forwarding)
//!     server pool 10.0.1.0/24, gateway 10.0.1.1  ob ---- rb  10.0.1.1
//! ```
//!
//! The client hosts and the server hosts of the plan sit on two different
//! prefixes, so no frame between them can pass without the router. The only
//! thing the plan says about the router is the `gateway` of each host pool.
//! The router namespace starts with an empty neighbor table and the own
//! namespace holds no address at all. The agent therefore has to find the
//! router with ARP requests (`DatapathConfig::neighbors`, which
//! `backend::datapath_config` turns on because the pools carry a gateway),
//! and the kernel router finds the hosts of the agent through the answers of
//! the splitter. The agent runs the http1 and dns entries of the example plan
//! for 3 seconds, and the test checks that every session passed, that each
//! arm asked for its gateway and got an answer, and that no resolution failed.
//!
//! The plan of the agent can only run the own stack on both arms, because the
//! module a worker runs is the one of the pool on the other side. A kernel
//! socket as the peer is therefore covered by the router test of the
//! datapath (the same cache, driven directly); here the kernel is the router.
//!
//! The test needs root, `ip`, `sysctl`, and AF_XDP. As a normal user it runs
//! itself again through `sudo -n` and skips with a message when that is not
//! possible. `CI=true` or `FPS_REQUIRE_AFXDP=1` turns every skip into a
//! failure. `FPS_ROUTER_RESULTS` names a directory that receives
//! `agent-router.txt`.

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use fps_agent::capabilities::AfXdpSupport;
use fps_agent::{detect, run, Outcome, RunOptions, SystemRoot};
use fps_stats::{metric, MetricId, SeriesValue, Snapshot};
use serde_json::Value as Json;

const MARK: &str = "fps-agent-router ";
const TEST: &str = "a_forced_af_xdp_run_reaches_its_peers_behind_a_kernel_router";

fn run_cmd(cmd: &str, args: &[&str]) -> Result<String, String> {
    let out = Command::new(cmd)
        .args(args)
        .output()
        .map_err(|e| format!("{cmd}: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(format!(
            "{cmd} {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr)
        ))
    }
}

fn required() -> bool {
    std::env::var("CI").is_ok_and(|v| v == "true")
        || std::env::var("FPS_REQUIRE_AFXDP").is_ok_and(|v| v == "1")
}

fn skip(why: &str) {
    assert!(
        !required(),
        "the agent router test cannot run here and CI requires it: {why}"
    );
    eprintln!("skipping the agent router test: {why}");
}

/// The two namespaces, which `Drop` removes again.
struct Topology {
    own: String,
    router: String,
}

impl Topology {
    fn up(tag: &str) -> Result<Self, String> {
        let t = Self {
            own: format!("a26o{tag}"),
            router: format!("a26r{tag}"),
        };
        // Built after the value exists, so that `Drop` cleans up a failure
        // half way.
        for ns in [&t.own, &t.router] {
            run_cmd("ip", &["netns", "add", ns])?;
        }
        let (o, r) = (t.own.as_str(), t.router.as_str());
        for (own_end, router_end) in [("oa", "ra"), ("ob", "rb")] {
            run_cmd(
                "ip",
                &[
                    "-n", o, "link", "add", own_end, "type", "veth", "peer", "name", router_end,
                    "netns", r,
                ],
            )?;
        }
        for (ns, dev) in [(o, "oa"), (o, "ob"), (r, "ra"), (r, "rb")] {
            run_cmd("ip", &["-n", ns, "link", "set", dev, "up"])?;
        }
        // No neighbor entry is configured anywhere: the router knows its two
        // addresses and nothing else.
        for (dev, address) in [("ra", "10.0.0.1/24"), ("rb", "10.0.1.1/24")] {
            run_cmd("ip", &["-n", r, "addr", "add", address, "dev", dev])?;
        }
        run_cmd(
            "ip",
            &[
                "netns",
                "exec",
                r,
                "sysctl",
                "-q",
                "-w",
                "net.ipv4.ip_forward=1",
            ],
        )?;
        Ok(t)
    }

    /// The neighbor entries of the router namespace, for the failure message.
    fn router_neighbors(&self) -> String {
        run_cmd("ip", &["-n", &self.router, "neigh", "show"]).unwrap_or_default()
    }
}

impl Drop for Topology {
    fn drop(&mut self) {
        for ns in [&self.own, &self.router] {
            let _ = run_cmd("ip", &["netns", "del", ns]);
        }
    }
}

fn contracts() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../contracts")
}

#[test]
fn a_forced_af_xdp_run_reaches_its_peers_behind_a_kernel_router() {
    let root = run_cmd("id", &["-u"]).is_ok_and(|s| s.trim() == "0");
    if !root {
        if run_cmd("sudo", &["-n", "true"]).is_err() {
            return skip("not root and no passwordless sudo");
        }
        let exe = std::env::current_exe().expect("the test binary");
        let mut cmd = Command::new("sudo");
        cmd.args(["-n", "env"]);
        for key in ["CI", "FPS_REQUIRE_AFXDP", "FPS_ROUTER_RESULTS"] {
            if let Ok(v) = std::env::var(key) {
                cmd.arg(format!("{key}={v}"));
            }
        }
        let status = cmd
            .arg(exe)
            .args(["--exact", TEST, "--nocapture"])
            .status()
            .expect("sudo starts");
        assert!(status.success(), "the test failed under sudo");
        return;
    }
    if let Err(e) = run_cmd("ip", &["-V"]) {
        return skip(&e);
    }
    match detect(&SystemRoot::host()).map(|h| h.af_xdp) {
        Ok(AfXdpSupport::Unsupported(_)) => return skip("the kernel has no AF_XDP sockets"),
        Ok(_) => {}
        Err(e) => return skip(&format!("capability detection failed: {e}")),
    }
    let tag = format!("{}", std::process::id() % 100_000);
    let topology = match Topology::up(&tag) {
        Ok(t) => t,
        Err(e) => return skip(&format!("the namespaces did not come up: {e}")),
    };
    assert!(
        topology.router_neighbors().trim().is_empty(),
        "the router starts with an empty neighbor table: {}",
        topology.router_neighbors()
    );

    let exe = std::env::current_exe().expect("the test binary");
    let mut child = Command::new("ip")
        .args(["netns", "exec", &topology.own])
        .arg(exe)
        .args(["--ignored", "--exact", "helper_router", "--nocapture"])
        .env("FPS_AGENT_ROUTER_ROLE", "run")
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("ip netns exec starts");
    let stdout = child.stdout.take().expect("piped stdout");
    let lines: Vec<String> = BufReader::new(stdout)
        .lines()
        .map_while(Result::ok)
        .filter_map(|l| l.strip_prefix(MARK).map(str::to_owned))
        .collect();
    let status = child.wait().expect("the helper ends");
    for l in &lines {
        eprintln!("{MARK}{l}");
    }
    let neighbors = topology.router_neighbors();
    eprintln!("{MARK}router neighbors after the run:\n{neighbors}");
    assert!(status.success(), "the helper failed: {lines:?}");
    for want in [
        "run outcome=pass sessions http1=",
        "client arm requests=",
        "server arm requests=",
    ] {
        assert!(
            lines.iter().any(|l| l.contains(want)),
            "no line with {want:?} in {lines:?}"
        );
    }
    // The router learned the hosts of both arms from the traffic of the run:
    // it holds entries for the hosts of the pools, which only exist because
    // the frames passed through it.
    assert!(neighbors.contains("10.0.0."), "{neighbors}");
    assert!(neighbors.contains("10.0.1."), "{neighbors}");
    if let Ok(dir) = std::env::var("FPS_ROUTER_RESULTS") {
        let dir = PathBuf::from(dir);
        std::fs::create_dir_all(&dir).expect("results dir");
        let mut text = lines.join("\n");
        text.push('\n');
        std::fs::write(dir.join("agent-router.txt"), text).expect("results file");
    }
}

// ---------------------------------------------------------------------------
// The helper process, inside the own namespace
// ---------------------------------------------------------------------------

fn say(msg: &str) {
    println!("{MARK}{msg}");
    let _ = std::io::stdout().flush();
}

/// The counter of `name` for `application` on `side` of one snapshot.
fn count(total: &Snapshot, name: MetricId, application: &str, side: &str) -> u64 {
    total
        .iter()
        .filter(|(k, _)| {
            k.metric() == name
                && k.labels()
                    .contains(&("application", application.to_owned()))
                && k.labels().contains(&("side", side.to_owned()))
        })
        .map(|(_, v)| match v {
            SeriesValue::Count(n) => *n,
            _ => 0,
        })
        .sum()
}

/// The example plan on `oa` and `ob` with a gateway on each host pool.
fn router_plan() -> Json {
    let text = std::fs::read_to_string(contracts().join("examples/plans/http_dns_smoke.json"))
        .expect("example plan");
    let mut plan: Json = serde_json::from_str(&text).expect("example plan JSON");
    plan["backend"] = "BACKEND_AF_XDP".into();
    plan["ports"] = serde_json::json!([
        { "name": "client0", "interfaceName": "oa" },
        { "name": "server0", "interfaceName": "ob" }
    ]);
    plan["hostPools"][0]["ipv4Prefix"] = "10.0.0.0/24".into();
    plan["hostPools"][0]["gateway"] = "10.0.0.1".into();
    plan["hostPools"][0]["poolCount"] = 4.into();
    plan["hostPools"][0]["count"] = 4.into();
    plan["hostPools"][1]["ipv4Prefix"] = "10.0.1.0/24".into();
    plan["hostPools"][1]["gateway"] = "10.0.1.1".into();
    plan["hostPools"][0]["osProfile"] = "windows11".into();
    plan["hostPools"][1]["osProfile"] = "linux6".into();
    // Two plan workers with a server host each, as in afxdp_veth.rs.
    plan["workers"] = serde_json::json!([
        {
            "core": 0,
            "hosts": [
                { "pool": "clients", "count": 2 },
                { "pool": "servers", "count": 1 }
            ],
            "rates": [
                { "mixEntry": "web_get", "sessionsPerSecond": 10 },
                { "mixEntry": "dns_lookup", "sessionsPerSecond": 5 }
            ]
        },
        {
            "workerId": 1,
            "core": 0,
            "hosts": [
                { "pool": "clients", "offset": 2, "count": 2 },
                { "pool": "servers", "offset": 1, "count": 1 }
            ],
            "rates": [
                { "mixEntry": "web_get", "sessionsPerSecond": 10 },
                { "mixEntry": "dns_lookup", "sessionsPerSecond": 5 }
            ]
        }
    ]);
    plan["load"]["phases"] = serde_json::json!([
        { "name": "steady", "duration": "3s", "shape": "PHASE_SHAPE_HOLD",
          "startFraction": 1, "endFraction": 1 }
    ]);
    plan["warmup"] = "0s".into();
    plan
}

#[test]
#[ignore = "started by the agent router test inside a namespace"]
fn helper_router() {
    if std::env::var("FPS_AGENT_ROUTER_ROLE").is_err() {
        return;
    }
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("fps-agent-router");
    std::fs::create_dir_all(&dir).expect("plan dir");
    let path = dir.join("router_plan.json");
    std::fs::write(&path, serde_json::to_string_pretty(&router_plan()).unwrap()).unwrap();
    let modules = contracts().join("examples/modules");

    let opts = RunOptions::new(path, modules);
    let mut out = Vec::new();
    let mut log = Vec::new();
    let summary = run(&opts, &mut out, &mut log);
    let log = String::from_utf8_lossy(&log).into_owned();
    for line in log.lines() {
        eprintln!("{MARK}log {line}");
    }
    let summary = summary.expect("the run on the datapath completes");
    assert_eq!(summary.outcome, Outcome::Pass, "{log}");
    let (client, server) = (
        summary.client.as_ref().expect("client totals"),
        summary.server.as_ref().expect("server totals"),
    );
    let mut sessions = Vec::new();
    for application in ["http1", "dns"] {
        let attempted = count(client, metric::SESSIONS_ATTEMPTED, application, "client");
        let ok = count(client, metric::SESSIONS_SUCCEEDED, application, "client");
        let served = count(server, metric::SESSIONS_SUCCEEDED, application, "server");
        assert!(attempted > 0, "{application}: no session ran\n{log}");
        assert_eq!(
            ok, attempted,
            "{application}: a client session failed\n{log}"
        );
        assert_eq!(
            served, attempted,
            "{application}: a server session failed\n{log}"
        );
        sessions.push(format!("{application}={ok}"));
    }
    assert_eq!(client.counter_total(metric::SESSIONS_FAILED), 0, "{log}");
    assert_eq!(server.counter_total(metric::SESSIONS_FAILED), 0, "{log}");

    // Each arm asked for its gateway, and the router answered.
    assert_eq!(summary.lanes.len(), 1);
    let lane = &summary.lanes[0];
    assert!(!lane.panicked, "{lane:?}");
    assert!(
        lane.stats.client.tcp > 0 && lane.stats.server.tcp > 0,
        "{lane:?}"
    );
    for (arm, c) in [("client", lane.stats.client), ("server", lane.stats.server)] {
        assert!(c.neighbor_requests >= 1, "{arm} arm never asked: {c:?}");
        assert!(c.neighbor_resolved >= 1, "{arm} arm: no answer: {c:?}");
        assert!(c.neighbor_answers >= 1, "{arm} arm: no answer: {c:?}");
        assert_eq!(
            c.neighbor_failed, 0,
            "{arm} arm: a resolution failed: {c:?}"
        );
        assert_eq!(
            c.neighbor_entries, 1,
            "{arm} arm holds the gateway and nothing else: {c:?}"
        );
        say(&format!(
            "{arm} arm requests={} answers={} resolved={} held={} released={} dropped={} failed={} entries={}",
            c.neighbor_requests,
            c.neighbor_answers,
            c.neighbor_resolved,
            c.neighbor_held,
            c.neighbor_released,
            c.neighbor_dropped,
            c.neighbor_failed,
            c.neighbor_entries
        ));
    }
    say(&format!("run outcome=pass sessions {}", sessions.join(" ")));
}
