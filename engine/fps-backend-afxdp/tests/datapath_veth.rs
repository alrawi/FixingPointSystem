// SPDX-License-Identifier: GPL-3.0-only
//! The user space datapath of TRN-27 on AF_XDP queues over a veth pair.
//!
//! The test makes a network namespace with a veth pair, `dpa` and `dpb`, and
//! runs this test binary again inside it (the hidden test `helper_datapath`).
//! The helper opens an AF_XDP queue on each end of the pair, gives them to
//! `fps_datapath::Datapath` as the client arm and the server arm of one lane,
//! and runs sessions between client hosts and server hosts of two host pools
//! through the own TCP stack: frames leave `dpa` through its AF_XDP socket and
//! arrive at the AF_XDP socket of `dpb`, and back.
//!
//! The helper first opens and holds a number of sessions, reads the sockets
//! that the stack holds, and then runs a number of short sessions (connect,
//! request, response, close) from a fixed number of concurrent tasks while it
//! still holds the first ones. It reports sessions per second and sockets held.
//!
//! The test lives in this crate, and not in `fps-datapath`, so that the
//! `afxdp` job of `trn.yml`, which runs `cargo test -p fps-backend-afxdp` with
//! `FPS_REQUIRE_AFXDP=1`, runs it.
//!
//! AF_XDP needs root. As a normal user the test runs itself again through
//! `sudo -n` and skips with a message when neither works. `CI=true` or
//! `FPS_REQUIRE_AFXDP=1` turns every skip into a failure. The environment
//! sets the size of the run:
//!
//! * `FPS_DP_SESSIONS` short sessions, default 10000.
//! * `FPS_DP_HOLD` sessions held open, default 2000.
//! * `FPS_DP_CONCURRENCY` concurrent short sessions, default 64.
//! * `FPS_DATAPATH_RESULTS`, a directory that receives `datapath-veth.txt`
//!   with the report lines and the host.

use std::io::{BufRead, BufReader, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use fps_backend_afxdp::{open_port, BindMode, BoundMode, OpenedPort, PortConfig, QueueConfig};
use fps_datapath::{split_pools, Datapath, DatapathConfig, DatapathSession, Lane};
use fps_tcp_own::Pools;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const MARK: &str = "fps-dp ";
const REQUEST: usize = 64;
const PORT: u16 = 8080;
const TEST: &str = "datapath_runs_sessions_on_af_xdp_queues_over_a_veth_pair";

fn run(cmd: &str, args: &[&str]) -> Result<String, String> {
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
        "the AF_XDP datapath test cannot run here and CI requires it: {why}"
    );
    eprintln!("skipping the AF_XDP datapath test: {why}");
}

/// Whether the kernel can create an AF_XDP socket at all.
fn kernel_has_af_xdp() -> bool {
    const AF_XDP: libc::c_int = 44;
    // SAFETY: `socket` takes no pointers, and the descriptor that it returns
    // is closed at once.
    let fd = unsafe { libc::socket(AF_XDP, libc::SOCK_RAW, 0) };
    if fd >= 0 {
        // SAFETY: `fd` is the descriptor from above, which nothing else owns.
        unsafe { libc::close(fd) };
    }
    fd >= 0 || std::io::Error::last_os_error().raw_os_error() != Some(libc::EAFNOSUPPORT)
}

fn env_u64(key: &str, default: u64) -> u64 {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

/// A namespace with a veth pair, which `Drop` removes again.
struct Namespace(String);

impl Namespace {
    fn up(name: &str) -> Result<Self, String> {
        run("ip", &["netns", "add", name])?;
        let ns = Self(name.to_owned());
        run(
            "ip",
            &[
                "-n", name, "link", "add", "dpa", "type", "veth", "peer", "name", "dpb",
            ],
        )?;
        for dev in ["dpa", "dpb"] {
            run("ip", &["-n", name, "link", "set", dev, "up"])?;
        }
        Ok(ns)
    }
}

impl Drop for Namespace {
    fn drop(&mut self) {
        let _ = run("ip", &["netns", "del", &self.0]);
    }
}

fn field<'a>(line: &'a str, key: &str) -> &'a str {
    line.split_whitespace()
        .find_map(|w| w.strip_prefix(key).and_then(|r| r.strip_prefix('=')))
        .unwrap_or_else(|| panic!("no field {key} in {line:?}"))
}

fn number(line: &str, key: &str) -> u64 {
    field(line, key)
        .parse()
        .unwrap_or_else(|_| panic!("field {key} of {line:?} is not a number"))
}

#[test]
fn datapath_runs_sessions_on_af_xdp_queues_over_a_veth_pair() {
    let root = run("id", &["-u"]).is_ok_and(|s| s.trim() == "0");
    if !root {
        if run("sudo", &["-n", "true"]).is_err() {
            return skip("not root and no passwordless sudo");
        }
        let exe = std::env::current_exe().unwrap();
        let mut cmd = Command::new("sudo");
        cmd.args(["-n", "env", "FPS_AFXDP_UNDER_SUDO=1"]);
        for key in [
            "CI",
            "FPS_REQUIRE_AFXDP",
            "FPS_DP_SESSIONS",
            "FPS_DP_HOLD",
            "FPS_DP_CONCURRENCY",
            "FPS_DATAPATH_RESULTS",
        ] {
            if let Ok(v) = std::env::var(key) {
                cmd.arg(format!("{key}={v}"));
            }
        }
        let status = cmd
            .arg(exe)
            .args(["--exact", TEST, "--nocapture"])
            .status()
            .unwrap();
        assert!(status.success(), "the test failed under sudo");
        return;
    }
    if let Err(e) = run("ip", &["-V"]) {
        return skip(&e);
    }
    if !kernel_has_af_xdp() {
        return skip("the kernel has no AF_XDP sockets");
    }
    let name = format!("dp{}", std::process::id() % 100_000);
    let ns = match Namespace::up(&name) {
        Ok(ns) => ns,
        Err(e) => return skip(&format!("the namespace did not come up: {e}")),
    };

    let exe = std::env::current_exe().unwrap();
    let mut child = Command::new("ip")
        .args(["netns", "exec", &ns.0])
        .arg(exe)
        .args(["--ignored", "--exact", "helper_datapath", "--nocapture"])
        .env("FPS_DP_ROLE", "run")
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    let stdout = child.stdout.take().unwrap();
    let lines: Vec<String> = BufReader::new(stdout)
        .lines()
        .map_while(Result::ok)
        .filter_map(|l| l.strip_prefix(MARK).map(str::to_owned))
        .collect();
    let status = child.wait().unwrap();
    for l in &lines {
        eprintln!("{MARK}{l}");
    }
    assert!(status.success(), "the helper failed: {lines:?}");

    let result = lines
        .iter()
        .find(|l| l.starts_with("result "))
        .expect("a result line");
    let (sessions, hold) = (
        env_u64("FPS_DP_SESSIONS", 10000),
        env_u64("FPS_DP_HOLD", 2000),
    );
    assert_eq!(number(result, "sessions"), sessions, "{result}");
    assert_eq!(number(result, "failed"), 0, "{result}");
    assert_eq!(number(result, "bad_bytes"), 0, "{result}");
    assert!(number(result, "sessions_per_second") > 0, "{result}");
    assert!(
        number(result, "sockets_held") >= 2 * hold,
        "both ends of every held session are sockets: {result}"
    );
    let frames = lines
        .iter()
        .find(|l| l.starts_with("frames "))
        .expect("a frames line");
    assert!(number(frames, "client_tcp") > 0 && number(frames, "server_tcp") > 0);
    assert_eq!(number(frames, "panicked"), 0, "{frames}");

    if let Ok(dir) = std::env::var("FPS_DATAPATH_RESULTS") {
        let dir = PathBuf::from(dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut text = lines.join("\n");
        text.push('\n');
        std::fs::write(dir.join("datapath-veth.txt"), text).unwrap();
        let host = format!(
            "cpus={}\nkernel={}\n",
            std::thread::available_parallelism().map_or(0, usize::from),
            run("uname", &["-r"]).unwrap_or_default().trim()
        );
        std::fs::write(dir.join("host.txt"), host).unwrap();
    }
}

// ---------------------------------------------------------------------------
// The helper process, inside the namespace
// ---------------------------------------------------------------------------

fn say(msg: &str) {
    println!("{MARK}{msg}");
    let _ = std::io::stdout().flush();
}

fn mode_name(m: BoundMode) -> &'static str {
    match m {
        BoundMode::ZeroCopy => "zerocopy",
        BoundMode::Copy => "copy",
    }
}

fn open(interface: &str) -> OpenedPort {
    let config = PortConfig {
        interface: interface.to_owned(),
        queues: vec![0],
        queue: QueueConfig {
            mode: BindMode::Auto,
            ..QueueConfig::default()
        },
    };
    // The kernel releases the queue of a closed socket in a work queue, so a
    // bind right after a close can meet EBUSY for a few milliseconds.
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match open_port(&config) {
            Ok(p) => return p,
            Err(e) if e.raw_os_error() == Some(16) && Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(e) => panic!("opening {interface}: {e:?}"),
        }
    }
}

fn response(request_first: u8, len: usize) -> Vec<u8> {
    (0..len)
        .map(|i| u8::try_from(i % 251).unwrap_or(0) ^ request_first)
        .collect()
}

/// One session of the workload: request, response, and a check of the bytes.
async fn exchange(session: &mut DatapathSession, seed: u8, len: usize) -> bool {
    let request = [seed; REQUEST];
    if session.write_all(&request).await.is_err() {
        return false;
    }
    let mut got = vec![0u8; len];
    if session.read_exact(&mut got).await.is_err() {
        return false;
    }
    got == response(seed, len)
}

/// The addresses of the two pools, so a session number picks a client host
/// and a server host.
struct Book {
    clients: Vec<IpAddr>,
    servers: Vec<IpAddr>,
}

impl Book {
    fn new(pools: &Pools) -> Self {
        let all = |slice: &fps_tcp_own::HostSlice| {
            (0..slice.hosts())
                .filter_map(|i| slice.address(i))
                .map(|a| IpAddr::V4(Ipv4Addr::from(a)))
                .collect::<Vec<_>>()
        };
        Self {
            clients: all(&pools.client),
            servers: all(&pools.server),
        }
    }

    /// Client hosts take turns, and the server host changes after a round.
    fn pick(&self, n: u64) -> (IpAddr, SocketAddr) {
        let (c, s) = (self.clients.len() as u64, self.servers.len() as u64);
        let src = self.clients[usize::try_from(n % c).unwrap_or(0)];
        let dst = self.servers[usize::try_from((n / c) % s).unwrap_or(0)];
        (src, SocketAddr::new(dst, PORT))
    }
}

#[derive(Default)]
struct Tally {
    done: AtomicU64,
    failed: AtomicU64,
    bad: AtomicU64,
    peak: AtomicU64,
}

#[test]
#[ignore = "started by the datapath veth test inside a namespace"]
fn helper_datapath() {
    if std::env::var("FPS_DP_ROLE").is_err() {
        return;
    }
    let sessions = env_u64("FPS_DP_SESSIONS", 10000);
    let hold = env_u64("FPS_DP_HOLD", 2000);
    let concurrency = env_u64("FPS_DP_CONCURRENCY", 64).max(1);

    let (mut a, mut b) = (open("dpa"), open("dpb"));
    let modes = format!(
        "client={} server={}",
        mode_name(a.info.queues[0].bound),
        mode_name(b.info.queues[0].bound)
    );
    let pools = Pools::counts(64, 64, None).unwrap();
    let lane = Lane {
        client: Box::new(a.queues.remove(0)),
        server: Box::new(b.queues.remove(0)),
        pools: split_pools(&pools, 1).unwrap().remove(0),
    };
    let dp = Datapath::start(vec![lane], DatapathConfig::default()).unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .unwrap();
    let tally = Arc::new(Tally::default());
    let started_at;
    let (held_sockets, elapsed);
    {
        let _guard = runtime.enter();
        let mut listener = dp.listen(PORT).unwrap();
        runtime.spawn(async move {
            while let Ok(mut session) = listener.accept().await {
                tokio::spawn(async move {
                    let mut request = [0u8; REQUEST];
                    if session.read_exact(&mut request).await.is_err() {
                        return;
                    }
                    if session
                        .write_all(&response(request[0], 1024))
                        .await
                        .is_err()
                    {
                        return;
                    }
                    // Wait for the client to close, so a held session stays
                    // open on both ends.
                    let mut rest = Vec::new();
                    let _ = session.read_to_end(&mut rest).await;
                });
            }
        });
        let book = Arc::new(Book::new(&pools));

        // Phase 1: open the sessions to hold.
        let connector = dp.connector();
        let held = runtime.block_on(async {
            let next = Arc::new(AtomicU64::new(0));
            let mut workers = Vec::new();
            for _ in 0..concurrency.min(hold.max(1)) {
                let (connector, next, tally, book) = (
                    connector.clone(),
                    Arc::clone(&next),
                    Arc::clone(&tally),
                    Arc::clone(&book),
                );
                workers.push(tokio::spawn(async move {
                    let mut mine = Vec::new();
                    loop {
                        let n = next.fetch_add(1, Ordering::Relaxed);
                        if n >= hold {
                            return mine;
                        }
                        let (src, dst) = book.pick(n);
                        match connector.connect(src, dst).await {
                            Ok(mut s) => {
                                if exchange(&mut s, 1, 1024).await {
                                    mine.push(s);
                                } else {
                                    tally.failed.fetch_add(1, Ordering::Relaxed);
                                }
                            }
                            Err(e) => {
                                eprintln!("hold connect {n}: {e}");
                                tally.failed.fetch_add(1, Ordering::Relaxed);
                            }
                        }
                    }
                }));
            }
            let mut held = Vec::new();
            for w in workers {
                held.extend(w.await.unwrap());
            }
            held
        });
        held_sockets = dp.sockets_open();
        say(&format!(
            "held sessions={} sockets_held={held_sockets} failed={}",
            held.len(),
            tally.failed.load(Ordering::Relaxed)
        ));
        assert_eq!(held.len() as u64, hold, "every held session came up");

        // Phase 2: short sessions while the first ones stay open.
        started_at = Instant::now();
        runtime.block_on(async {
            let next = Arc::new(AtomicU64::new(0));
            let mut workers = Vec::new();
            for _ in 0..concurrency {
                let (connector, next, tally, book) = (
                    connector.clone(),
                    Arc::clone(&next),
                    Arc::clone(&tally),
                    Arc::clone(&book),
                );
                let monitor = dp.monitor();
                workers.push(tokio::spawn(async move {
                    loop {
                        let n = next.fetch_add(1, Ordering::Relaxed);
                        if n >= sessions {
                            return;
                        }
                        let (src, dst) = book.pick(n + hold);
                        let ok = match connector.connect(src, dst).await {
                            Ok(mut s) => {
                                let seed = u8::try_from(n % 200).unwrap_or(0) + 2;
                                let ok = exchange(&mut s, seed, 1024).await;
                                if !ok {
                                    tally.bad.fetch_add(1, Ordering::Relaxed);
                                }
                                ok
                            }
                            Err(e) => {
                                eprintln!("connect {n}: {e}");
                                false
                            }
                        };
                        if !ok {
                            tally.failed.fetch_add(1, Ordering::Relaxed);
                        }
                        let done = tally.done.fetch_add(1, Ordering::Relaxed) + 1;
                        if done % 128 == 0 {
                            tally
                                .peak
                                .fetch_max(monitor.sockets_open(), Ordering::Relaxed);
                        }
                    }
                }));
            }
            for w in workers {
                w.await.unwrap();
            }
        });
        elapsed = started_at.elapsed();
        drop(held);
    }
    let done = tally.done.load(Ordering::Relaxed);
    let rate = (done as f64 / elapsed.as_secs_f64()).round() as u64;
    let peak = tally.peak.load(Ordering::Relaxed).max(held_sockets);
    say(&format!(
        "result sessions={done} failed={} bad_bytes={} concurrency={concurrency} hold={hold} \
         elapsed_ms={} sessions_per_second={rate} sockets_held={held_sockets} \
         sockets_peak={peak} {modes}",
        tally.failed.load(Ordering::Relaxed),
        tally.bad.load(Ordering::Relaxed),
        elapsed.as_millis(),
    ));
    let reports = dp.stop();
    let lane = &reports[0];
    say(&format!(
        "frames stack_tx={} client_tcp={} server_tcp={} client_arp={} client_other={} \
         client_nd_unsupported={} client_udp_unsupported={} loop_passes={} panicked={}",
        lane.stats.stack.frames,
        lane.stats.client.tcp,
        lane.stats.server.tcp,
        lane.stats.client.arp_replied + lane.stats.client.arp_ignored,
        lane.stats.client.other_dropped + lane.stats.server.other_dropped,
        lane.stats.client.nd_unsupported + lane.stats.server.nd_unsupported,
        lane.stats.client.udp_unsupported + lane.stats.server.udp_unsupported,
        lane.stats.loop_passes,
        u8::from(lane.panicked),
    ));
}
