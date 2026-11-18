// SPDX-License-Identifier: GPL-3.0-only
//! A forced AF_XDP run starts the AF_XDP backend through the datapath,
//! reports the bound mode of a real socket (AGT-24), and runs the http1 and
//! dns modules on it (AGT-25).
//!
//! The test makes a network namespace with a veth pair, `dpa` and `dpb`, and
//! runs this test binary again inside it (the hidden test `helper_afxdp`).
//! The helper takes the example plan `http_dns_smoke.json`, names the backend
//! `BACKEND_AF_XDP`, binds the two ports of the plan to the two ends of the
//! pair, gives the pools an OS profile each, and keeps both mix entries on two
//! plan workers, so two server workers listen on port 8080 and two on port 53.
//! Then it:
//! 1. prepares the plan with `prepare_on`, as `fps-agent connect` does after
//!    a forced `--backend af_xdp`, and checks that the run starts AF_XDP,
//! 2. starts the backend and reads the mode of each port back from the bound
//!    sockets, which a veth pair binds natively in copy mode,
//! 3. writes the modes into a `Capabilities` as `xdp_driver_mode`,
//! 4. opens a real TCP session between a client host and a server host of the
//!    plan pools through the datapath, which sends frames out of `dpa`
//!    through its AF_XDP socket into `dpb` and back, and checks every byte,
//! 5. stops the backend and runs the standalone `run` on the same plan. The
//!    workers open their sessions through the datapath, so every http1 and
//!    dns session of the run goes out of `dpa` and in at `dpb` through the
//!    AF_XDP sockets. The run logs the bound mode of each port, passes its
//!    criteria with no failed session, and reports the frames that each lane
//!    carried.
//!
//! The own stack sends no ARP requests, and the station tables of the pools
//! hold the MAC address of every host, so the harness seeds no neighbors. No
//! kernel socket is a peer here, so the transmit checksum offload of the veth
//! pair does not matter (a kernel peer needs it off, see TRN-26).
//!
//! AF_XDP needs root. As a normal user the test runs itself again through
//! `sudo -n` and skips with a message when neither works. `CI=true` or
//! `FPS_REQUIRE_AFXDP=1` turns every skip into a failure.

use std::io::{BufRead, BufReader, Write};
use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use fps_agent::assemble::Sides;
use fps_agent::backend::StartedBackend;
use fps_agent::capabilities::AfXdpSupport;
use fps_agent::plan::Plan;
use fps_agent::proto::{Capabilities, PortInfo, XdpMode};
use fps_agent::run::prepare;
use fps_agent::trust::ModulePolicy;
use fps_agent::{detect, run, Outcome, RunOptions, SystemRoot};
use fps_backend_afxdp::BackendKind;
use fps_hosts::{HostPool, PoolSpec};
use fps_stats::{metric, MetricId, SeriesValue, Snapshot};
use serde_json::Value as Json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const MARK: &str = "fps-agent-afxdp ";
const TEST: &str = "a_forced_af_xdp_run_starts_the_datapath_on_a_veth_pair";
const PORT: u16 = 8080;

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
        "the AF_XDP backend test cannot run here and CI requires it: {why}"
    );
    eprintln!("skipping the AF_XDP backend test: {why}");
}

/// A network namespace with a veth pair, which `Drop` removes again.
struct Namespace(String);

impl Namespace {
    fn up(name: &str) -> Result<Self, String> {
        run_cmd("ip", &["netns", "add", name])?;
        let ns = Self(name.to_owned());
        run_cmd(
            "ip",
            &[
                "-n", name, "link", "add", "dpa", "type", "veth", "peer", "name", "dpb",
            ],
        )?;
        for dev in ["dpa", "dpb"] {
            run_cmd("ip", &["-n", name, "link", "set", dev, "up"])?;
        }
        Ok(ns)
    }
}

impl Drop for Namespace {
    fn drop(&mut self) {
        let _ = run_cmd("ip", &["netns", "del", &self.0]);
    }
}

fn contracts() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../contracts")
}

#[test]
fn a_forced_af_xdp_run_starts_the_datapath_on_a_veth_pair() {
    let root = run_cmd("id", &["-u"]).is_ok_and(|s| s.trim() == "0");
    if !root {
        if run_cmd("sudo", &["-n", "true"]).is_err() {
            return skip("not root and no passwordless sudo");
        }
        let exe = std::env::current_exe().expect("the test binary");
        let mut cmd = Command::new("sudo");
        cmd.args(["-n", "env", "FPS_AFXDP_UNDER_SUDO=1"]);
        for key in ["CI", "FPS_REQUIRE_AFXDP"] {
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
    let name = format!("ag{}", std::process::id() % 100_000);
    let ns = match Namespace::up(&name) {
        Ok(ns) => ns,
        Err(e) => return skip(&format!("the namespace did not come up: {e}")),
    };

    let exe = std::env::current_exe().expect("the test binary");
    let mut child = Command::new("ip")
        .args(["netns", "exec", &ns.0])
        .arg(exe)
        .args(["--ignored", "--exact", "helper_afxdp", "--nocapture"])
        .env("FPS_AGENT_AFXDP_ROLE", "run")
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
    assert!(status.success(), "the helper failed: {lines:?}");
    for want in [
        "selected backend=af_xdp",
        "port client0 interface dpa",
        "port server0 interface dpb",
        "session bytes=4096 ok=true",
        "run outcome=pass sessions http1=",
    ] {
        assert!(
            lines.iter().any(|l| l.contains(want)),
            "no line with {want:?} in {lines:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// The helper process, inside the namespace
// ---------------------------------------------------------------------------

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

fn say(msg: &str) {
    println!("{MARK}{msg}");
    let _ = std::io::stdout().flush();
}

/// The example plan on the two ends of the veth pair with the AF_XDP backend.
fn af_xdp_plan() -> Json {
    let text = std::fs::read_to_string(contracts().join("examples/plans/http_dns_smoke.json"))
        .expect("example plan");
    let mut plan: Json = serde_json::from_str(&text).expect("example plan JSON");
    plan["backend"] = "BACKEND_AF_XDP".into();
    plan["ports"] = serde_json::json!([
        { "name": "client0", "interfaceName": "dpa" },
        { "name": "server0", "interfaceName": "dpb" }
    ]);
    plan["hostPools"][0]["ipv4Prefix"] = "10.1.0.0/24".into();
    plan["hostPools"][0]["poolCount"] = 4.into();
    plan["hostPools"][0]["count"] = 4.into();
    plan["hostPools"][1]["ipv4Prefix"] = "10.2.0.0/24".into();
    // One OS profile for each side (TRN-28): the own stack writes them into
    // the SYN and the SYN-ACK.
    plan["hostPools"][0]["osProfile"] = "windows11".into();
    plan["hostPools"][1]["osProfile"] = "linux6".into();
    // Two plan workers with a server host each: both listen on 8080 and 53.
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

/// The address of host `i` of a pool of the plan.
fn host_address(plan: &Plan, pool: &str, i: u32) -> IpAddr {
    let slice = plan
        .host_pools
        .iter()
        .find(|p| p.name == pool)
        .expect("the pool");
    let parsed = HostPool::parse(&PoolSpec {
        name: &slice.name,
        ipv4: Some(&slice.ipv4_prefix),
        count: slice.pool_count,
        ..PoolSpec::default()
    })
    .expect("the pool parses");
    IpAddr::V4(
        parsed
            .ipv4_addr(i)
            .expect("an address")
            .expect("an IPv4 pool"),
    )
}

#[test]
#[ignore = "started by the AF_XDP backend test inside a namespace"]
fn helper_afxdp() {
    if std::env::var("FPS_AGENT_AFXDP_ROLE").is_err() {
        return;
    }
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("fps-agent-afxdp");
    std::fs::create_dir_all(&dir).expect("plan dir");
    let path = dir.join("af_xdp_plan.json");
    std::fs::write(&path, serde_json::to_string_pretty(&af_xdp_plan()).unwrap()).unwrap();
    let modules = contracts().join("examples/modules");

    // 1. The run follows the selection. The agent selected AF_XDP, and the
    //    plan agrees.
    let plan = Plan::load(&path).expect("the plan loads");
    let prepared = fps_agent::run::prepare_on(
        plan.clone(),
        &modules,
        Sides::Both,
        &ModulePolicy::default(),
        &fps_agent::tls::KeyTap::new(),
        Some(BackendKind::AfXdp),
    )
    .expect("the plan prepares on AF_XDP");
    assert_eq!(prepared.backend(), BackendKind::AfXdp);
    say("selected backend=af_xdp");
    // A plan for the kernel backend on an agent that selected AF_XDP is refused.
    let mut kernel_plan = plan.clone();
    kernel_plan.backend = fps_agent::plan::Backend::Kernel;
    let refused = fps_agent::run::prepare_on(
        kernel_plan,
        &modules,
        Sides::Both,
        &ModulePolicy::default(),
        &fps_agent::tls::KeyTap::new(),
        Some(BackendKind::AfXdp),
    )
    .expect_err("a kernel plan on an AF_XDP agent");
    assert!(
        refused.to_string().contains("asks for backend kernel"),
        "{refused}"
    );

    // 2. The backend starts and reports the mode of its real sockets.
    let started = prepared.start_backend().expect("the datapath starts");
    let StartedBackend::AfXdp(backend) = started else {
        panic!("AF_XDP was selected, and the kernel backend started");
    };
    let mut caps = Capabilities {
        ports: ["dpa", "dpb", "eth0"]
            .iter()
            .map(|n| PortInfo {
                name: (*n).to_owned(),
                ..PortInfo::default()
            })
            .collect(),
        ..Capabilities::default()
    };
    for port in backend.ports() {
        say(&format!("{port}"));
        assert_ne!(port.xdp_mode(), XdpMode::Unspecified, "{port}");
        assert_eq!(port.queues.len(), 1, "{port}");
    }
    let started = StartedBackend::AfXdp(backend);
    // 3. The mode reaches PortInfo.xdp_driver_mode of the ports that were bound.
    started.report(&mut caps);
    for (info, bound) in caps.ports.iter().take(2).zip(started.ports()) {
        assert_eq!(info.xdp_driver_mode, bound.xdp_mode(), "{}", info.name);
        say(&format!(
            "report {} xdp_driver_mode={}",
            info.name,
            info.xdp_driver_mode.proto_name()
        ));
    }
    assert_eq!(
        caps.ports[2].xdp_driver_mode,
        XdpMode::Unspecified,
        "a port that no queue bound stays unreported"
    );

    // 4. A real session through the datapath.
    let StartedBackend::AfXdp(backend) = started else {
        unreachable!("matched above");
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .expect("a runtime");
    {
        let _guard = runtime.enter();
        let mut listener = backend.datapath().listen(PORT).expect("listen");
        let connector = backend.datapath().connector();
        let src = host_address(&plan, "clients", 0);
        let dst = SocketAddr::new(host_address(&plan, "servers", 0), PORT);
        let (sent, ok) = runtime.block_on(async move {
            let server = tokio::spawn(async move {
                let (mut session, _peer) = listener.accept().await.expect("accept");
                let mut buf = vec![0u8; 4096];
                session.read_exact(&mut buf).await.expect("read");
                // Echo the bytes back inverted, so a reflected frame is not
                // mistaken for an answer.
                for b in &mut buf {
                    *b = !*b;
                }
                session.write_all(&buf).await.expect("write");
                let mut rest = Vec::new();
                let _ = session.read_to_end(&mut rest).await;
            });
            let request: Vec<u8> = (0..4096u32)
                .map(|i| u8::try_from(i % 251).unwrap_or(0))
                .collect();
            let mut session =
                tokio::time::timeout(Duration::from_secs(10), connector.connect(src, dst))
                    .await
                    .expect("no timeout")
                    .expect("connect");
            session.write_all(&request).await.expect("write");
            let mut got = vec![0u8; 4096];
            session.read_exact(&mut got).await.expect("read");
            let ok = got.iter().zip(&request).all(|(g, r)| *g == !*r);
            drop(session);
            let _ = tokio::time::timeout(Duration::from_secs(5), server).await;
            (request.len(), ok)
        });
        say(&format!("session bytes={sent} ok={ok}"));
        assert!(ok, "a byte came back wrong");
    }
    let reports = backend.stop();
    assert!(reports.iter().all(|r| !r.panicked), "{reports:?}");
    let frames: u64 = reports
        .iter()
        .map(|r| r.stats.client.tcp + r.stats.server.tcp)
        .sum();
    assert!(frames > 0, "the session sent frames through the queues");
    say(&format!("frames tcp={frames}"));

    // 5. The standalone run starts the backend, and the http1 and dns workers
    //    run their sessions on it (AGT-25).
    let opts = RunOptions::new(path, modules);
    let mut out = Vec::new();
    let mut log = Vec::new();
    let summary = run(&opts, &mut out, &mut log);
    let log = String::from_utf8_lossy(&log).into_owned();
    for line in log.lines() {
        eprintln!("{MARK}log {line}");
    }
    let summary = summary.expect("the run on the datapath completes");
    for line in log
        .lines()
        .filter(|l| l.contains("backend af_xdp bound") || l.contains("datapath lane"))
    {
        say(line);
    }
    assert!(
        log.contains("backend af_xdp bound port client0 interface dpa"),
        "{log}"
    );
    assert!(
        log.contains("backend af_xdp bound port server0 interface dpb"),
        "{log}"
    );
    assert!(log.contains("xdp_driver_mode XDP_MODE_"), "{log}");
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
    // The frames of the sessions went through the AF_XDP queues of the lane.
    assert_eq!(summary.lanes.len(), 1);
    let lane = &summary.lanes[0];
    assert!(!lane.panicked, "{lane:?}");
    assert!(
        lane.stats.client.tcp > 0 && lane.stats.server.tcp > 0,
        "{lane:?}"
    );
    assert!(lane.stats.udp_client.tx_datagrams > 0, "{lane:?}");
    assert!(lane.stats.udp_server.rx_accepted > 0, "{lane:?}");
    assert_eq!(lane.stats.udp_client.rx_no_listener, 0, "{lane:?}");
    say(&format!("run outcome=pass sessions {}", sessions.join(" ")));

    // The lease of a queue ends with the run, so a second start works.
    let plan = Plan::load(&opts.plan).expect("the plan loads");
    let again = prepare(plan, &opts.modules, Sides::Both, &ModulePolicy::default())
        .expect("prepare")
        .start_backend()
        .expect("the queues were released");
    assert_eq!(again.kind(), BackendKind::AfXdp);
}
