// SPDX-License-Identifier: GPL-3.0-only
//! AF_XDP on veth pairs in the QA-01 namespace harness.
//!
//! The test brings up the harness, runs one process in the client namespace
//! and one in the router namespace, and checks that frames cross a veth pair
//! in both directions, that a queue falls back to copy mode where the driver
//! has no zero copy, and that the bind mode comes from the socket. The
//! processes are this test binary again, started with `ip netns exec` and the
//! hidden test `helper_process`, so the test needs no `setns` and no `unsafe`.
//!
//! AF_XDP needs root. As a normal user the test runs itself again through
//! `sudo -n`, which GitHub hosted runners and the owner's hosts allow, and it
//! skips with a message when neither works. `CI=true` or `FPS_REQUIRE_AFXDP=1`
//! turns every skip into a failure, so a runner that cannot run the test does
//! not pass silently. `FPS_AFXDP_FRAMES` overrides the frame count.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

use fps_backend_afxdp::{
    open_port, AfXdpError, BindMode, BoundMode, FrameIo, Idle, OpenedPort, PortConfig, QueueConfig,
    QueueLoop, WorkerOptions, Workers, XdpMode,
};

/// The marker of the lines that helper processes report on.
const MARK: &str = "fps-afxdp ";
/// IEEE 802 local experimental ethertype 1, which nothing else sends.
const ETHERTYPE: [u8; 2] = [0x88, 0xb5];
const HEADER: usize = 14 + 12;
const MAX_LEN: usize = 1514;
const RING: u32 = 256;
const WINDOW: u64 = 96;

fn harness_script() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/harness/netns.sh")
}

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
        "the AF_XDP test cannot run here and CI requires it: {why}"
    );
    eprintln!("skipping the AF_XDP veth test: {why}");
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

fn is_root() -> bool {
    run("id", &["-u"]).is_ok_and(|s| s.trim() == "0")
}

/// One QA-01 harness, which `Drop` takes down again.
struct Harness {
    prefix: String,
    state_dir: PathBuf,
}

impl Harness {
    fn up(prefix: &str) -> Result<Self, String> {
        let state_dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("netns-{prefix}"));
        let out = Command::new(harness_script())
            .arg("up")
            .env("FPS_NETNS", prefix)
            .env("FPS_STATE_DIR", &state_dir)
            .output()
            .map_err(|e| format!("netns.sh: {e}"))?;
        if !out.status.success() {
            return Err(String::from_utf8_lossy(&out.stderr).into_owned());
        }
        Ok(Self {
            prefix: prefix.to_owned(),
            state_dir,
        })
    }

    fn ns(&self, arm: &str) -> String {
        format!("{}-{arm}", self.prefix)
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        let _ = Command::new(harness_script())
            .arg("down")
            .env("FPS_NETNS", &self.prefix)
            .env("FPS_STATE_DIR", &self.state_dir)
            .status();
    }
}

/// A helper process in a namespace, with its report lines.
struct Helper {
    child: Child,
    lines: mpsc::Receiver<String>,
    stdin: Option<ChildStdin>,
    seen: Vec<String>,
}

impl Helper {
    fn start(ns: &str, env: &[(&str, &str)]) -> Self {
        let exe = std::env::current_exe().unwrap();
        let mut cmd = Command::new("ip");
        cmd.args(["netns", "exec", ns]).arg(exe).args([
            "--ignored",
            "--exact",
            "helper_process",
            "--nocapture",
        ]);
        for (k, v) in env {
            cmd.env(k, v);
        }
        let mut child = cmd
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let stdout = child.stdout.take().unwrap();
        let stdin = child.stdin.take();
        let (tx, lines) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if let Some(rest) = line.strip_prefix(MARK) {
                    if tx.send(rest.to_owned()).is_err() {
                        break;
                    }
                }
            }
        });
        Self {
            child,
            lines,
            stdin,
            seen: Vec::new(),
        }
    }

    /// Waits for a report line that starts with `prefix` and returns it.
    fn wait_line(&mut self, prefix: &str, timeout: Duration) -> String {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(l) = self.seen.iter().find(|l| l.starts_with(prefix)) {
                return l.clone();
            }
            let left = deadline.saturating_duration_since(Instant::now());
            match self.lines.recv_timeout(left.max(Duration::from_millis(1))) {
                Ok(l) => self.seen.push(l),
                Err(_) => panic!(
                    "no report line {prefix:?} within {timeout:?}, saw {:?}",
                    self.seen
                ),
            }
        }
    }

    /// Closes stdin, which tells an echo helper to stop, and collects the
    /// remaining lines and the exit status.
    fn finish(mut self) -> (bool, Vec<String>) {
        drop(self.stdin.take());
        let status = self.child.wait().unwrap();
        while let Ok(l) = self.lines.recv_timeout(Duration::from_millis(500)) {
            self.seen.push(l);
        }
        for l in &self.seen {
            eprintln!("{MARK}{l}");
        }
        (status.success(), std::mem::take(&mut self.seen))
    }
}

impl Drop for Helper {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn field<'a>(line: &'a str, key: &str) -> &'a str {
    line.split_whitespace()
        .find_map(|w| w.strip_prefix(key).and_then(|r| r.strip_prefix('=')))
        .unwrap_or_else(|| panic!("no field {key} in {line:?}"))
}

fn frames_per_queue() -> u64 {
    std::env::var("FPS_AFXDP_FRAMES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(4000)
}

#[test]
fn veth_pairs_pass_frames_both_ways_with_copy_fallback_and_per_queue_workers() {
    if !is_root() {
        if run("sudo", &["-n", "true"]).is_err() {
            return skip("not root and no passwordless sudo");
        }
        let exe = std::env::current_exe().unwrap();
        let status = Command::new("sudo")
            .args(["-n", "env", "FPS_AFXDP_UNDER_SUDO=1"])
            .arg(exe)
            .args([
                "--exact",
                "veth_pairs_pass_frames_both_ways_with_copy_fallback_and_per_queue_workers",
                "--nocapture",
            ])
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

    let prefix = format!("xdp{}", std::process::id() % 100_000);
    let h = match Harness::up(&prefix) {
        Ok(h) => h,
        Err(e) => return skip(&format!("the namespace harness did not come up: {e}")),
    };
    let (client, router) = (h.ns("client"), h.ns("router"));

    modes_on_a_single_queue_veth(&client);
    one_queue_both_ways(&client, &router);
    two_queues_with_workers(&client, &router);
}

/// Bind modes, setup errors, and the mode that the socket reports, on `eth0`.
fn modes_on_a_single_queue_veth(client: &str) {
    let mut h = Helper::start(
        client,
        &[("FPS_AFXDP_ROLE", "modes"), ("FPS_AFXDP_IFACE", "eth0")],
    );
    let zc = h.wait_line("zerocopy", Duration::from_secs(20));
    assert_eq!(field(&zc, "zerocopy"), "unsupported", "{zc}");
    let (ok, lines) = {
        h.wait_line("done", Duration::from_secs(20));
        h.finish()
    };
    assert!(ok, "the modes helper failed: {lines:?}");
    let auto = lines.iter().find(|l| l.starts_with("auto")).unwrap();
    assert_eq!(
        field(auto, "auto"),
        "copy",
        "a veth pair has no zero copy, so Auto falls back: {auto}"
    );
    assert_eq!(field(auto, "socket"), "copy", "{auto}");
    let busy = lines.iter().find(|l| l.starts_with("busy")).unwrap();
    assert_eq!(field(busy, "busy"), "EBUSY", "{busy}");
    assert!(lines.iter().any(|l| l.starts_with("copy=ok")), "{lines:?}");
    assert!(
        lines.iter().any(|l| l.starts_with("config=rejected")),
        "{lines:?}"
    );
}

/// 4000 frames from the client to the router and back over `eth0` and `cli0`.
fn one_queue_both_ways(client: &str, router: &str) {
    let count = frames_per_queue();
    let mut echo = Helper::start(
        router,
        &[
            ("FPS_AFXDP_ROLE", "echo"),
            ("FPS_AFXDP_IFACE", "cli0"),
            ("FPS_AFXDP_QUEUES", "0"),
        ],
    );
    let ready = echo.wait_line("ready", Duration::from_secs(20));
    assert_eq!(field(&ready, "queues"), "0:copy", "{ready}");
    let count_s = count.to_string();
    let mut send = Helper::start(
        client,
        &[
            ("FPS_AFXDP_ROLE", "send"),
            ("FPS_AFXDP_IFACE", "eth0"),
            ("FPS_AFXDP_QUEUES", "0"),
            ("FPS_AFXDP_FRAMES", &count_s),
        ],
    );
    let done = send.wait_line("sent", Duration::from_secs(60));
    assert_eq!(field(&done, "sent"), count_s, "{done}");
    assert_eq!(
        field(&done, "received"),
        count_s,
        "every frame came back: {done}"
    );
    assert_eq!(field(&done, "bad"), "0", "{done}");
    assert_eq!(field(&done, "queues"), "0:copy", "{done}");
    let (ok, lines) = send.finish();
    assert!(ok, "the sender failed: {lines:?}");
    let (ok, lines) = echo.finish();
    assert!(ok, "the echo helper failed: {lines:?}");
    let q0 = lines
        .iter()
        .find(|l| l.starts_with("echo queue=0"))
        .unwrap();
    assert_eq!(
        field(q0, "echoed"),
        count_s,
        "the router side echoed every frame: {q0}"
    );
    assert_eq!(field(q0, "tx"), count_s, "{q0}");
    assert_eq!(field(q0, "error"), "none", "{q0}");
}

/// Two queues, each with its own worker on both ends, over a veth pair that
/// has two queues. Each frame must come back on the queue that sent it.
fn two_queues_with_workers(client: &str, router: &str) {
    run(
        "ip",
        &[
            "-n",
            client,
            "link",
            "add",
            "xq0",
            "numtxqueues",
            "2",
            "numrxqueues",
            "2",
            "type",
            "veth",
            "peer",
            "name",
            "xq1",
            "netns",
            router,
            "numtxqueues",
            "2",
            "numrxqueues",
            "2",
        ],
    )
    .unwrap();
    for (ns, dev) in [(client, "xq0"), (router, "xq1")] {
        run("ip", &["-n", ns, "link", "set", dev, "up"]).unwrap();
    }
    let count = frames_per_queue();
    let mut echo = Helper::start(
        router,
        &[
            ("FPS_AFXDP_ROLE", "echo"),
            ("FPS_AFXDP_IFACE", "xq1"),
            ("FPS_AFXDP_QUEUES", "0,1"),
        ],
    );
    let ready = echo.wait_line("ready", Duration::from_secs(20));
    assert_eq!(field(&ready, "queues"), "0:copy,1:copy", "{ready}");
    let count_s = count.to_string();
    let mut send = Helper::start(
        client,
        &[
            ("FPS_AFXDP_ROLE", "send"),
            ("FPS_AFXDP_IFACE", "xq0"),
            ("FPS_AFXDP_QUEUES", "0,1"),
            ("FPS_AFXDP_FRAMES", &count_s),
        ],
    );
    let done = send.wait_line("sent", Duration::from_secs(60));
    let total = (count * 2).to_string();
    assert_eq!(field(&done, "sent"), total, "{done}");
    assert_eq!(field(&done, "received"), total, "{done}");
    assert_eq!(field(&done, "bad"), "0", "{done}");
    let (ok, lines) = send.finish();
    assert!(ok, "the sender failed: {lines:?}");
    let (ok, lines) = echo.finish();
    assert!(ok, "the echo helper failed: {lines:?}");
    for q in 0..2 {
        let l = lines
            .iter()
            .find(|l| l.starts_with(&format!("echo queue={q} ")))
            .unwrap();
        assert_eq!(
            field(l, "echoed"),
            count_s,
            "queue {q} of the router saw only its own frames: {l}"
        );
        assert_eq!(field(l, "tx"), count_s, "{l}");
        assert_eq!(field(l, "error"), "none", "{l}");
    }
    run("ip", &["-n", client, "link", "del", "xq0"]).unwrap();
}

// ---------------------------------------------------------------------------
// The helper process
// ---------------------------------------------------------------------------

fn say(msg: &str) {
    println!("{MARK}{msg}");
    let _ = std::io::stdout().flush();
}

fn open(iface: &str, queues: &[u32], mode: BindMode) -> Result<OpenedPort, AfXdpError> {
    open_port(&PortConfig {
        interface: iface.to_owned(),
        queues: queues.to_vec(),
        queue: QueueConfig {
            ring_size: RING,
            mode,
            ..QueueConfig::default()
        },
    })
}

/// Opens a port again after another one closed on the same queue. The kernel
/// releases the queue of a closed socket in a work queue, so the next bind
/// can meet `EBUSY` for a few milliseconds.
fn open_after_close(iface: &str, queues: &[u32], mode: BindMode) -> OpenedPort {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match open(iface, queues, mode) {
            Ok(p) => return p,
            Err(e) if e.raw_os_error() == Some(16) && Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(e) => panic!("open after close failed: {e:?}"),
        }
    }
}

fn mode_name(m: BoundMode) -> &'static str {
    match m {
        BoundMode::ZeroCopy => "zerocopy",
        BoundMode::Copy => "copy",
    }
}

fn describe(p: &OpenedPort) -> String {
    let queues: Vec<String> = p
        .info
        .queues
        .iter()
        .map(|q| format!("{}:{}", q.queue_id, mode_name(q.bound)))
        .collect();
    let xdp = match p.info.xdp_mode {
        XdpMode::Native => "native",
        XdpMode::Generic => "generic",
    };
    format!("queues={} xdp={xdp}", queues.join(","))
}

fn env_var(key: &str) -> String {
    std::env::var(key).unwrap_or_default()
}

fn queue_list() -> Vec<u32> {
    env_var("FPS_AFXDP_QUEUES")
        .split(',')
        .filter_map(|q| q.parse().ok())
        .collect()
}

/// SplitMix64, so the payload of a frame follows from its queue and number.
fn mix(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9e37_79b9_7f4a_7c15);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

fn frame_len(queue: u32, seq: u64) -> usize {
    60 + (mix(u64::from(queue) << 40 | seq) % (MAX_LEN as u64 - 59)) as usize
}

/// Builds frame `seq` of `queue` into `buf` and returns its length.
fn build_frame(buf: &mut [u8], queue: u32, seq: u64) -> usize {
    let len = frame_len(queue, seq);
    buf[0..6].copy_from_slice(&[0x02, 0, 0, 0, 0, 0x02]);
    buf[6..12].copy_from_slice(&[0x02, 0, 0, 0, 0, 0x01]);
    buf[12..14].copy_from_slice(&ETHERTYPE);
    buf[14..18].copy_from_slice(&queue.to_be_bytes());
    buf[18..26].copy_from_slice(&seq.to_be_bytes());
    let mut state = mix(u64::from(queue) << 40 | seq ^ 0xa5a5);
    for b in &mut buf[HEADER..len] {
        state = mix(state);
        *b = state as u8;
    }
    len
}

/// Checks a frame that came back, and returns its queue and number.
fn check_frame(f: &[u8]) -> Option<(u32, u64)> {
    if f.len() < HEADER || f[12..14] != ETHERTYPE {
        return None;
    }
    let queue = u32::from_be_bytes(f[14..18].try_into().ok()?);
    let seq = u64::from_be_bytes(f[18..26].try_into().ok()?);
    let mut want = vec![0u8; MAX_LEN];
    let len = build_frame(&mut want, queue, seq);
    // The echo swaps the two MAC addresses, so the sender's destination is
    // now the source.
    let macs_swapped = f[0..6] == [0x02, 0, 0, 0, 0, 0x01] && f[6..12] == [0x02, 0, 0, 0, 0, 0x02];
    (macs_swapped && f.len() == len && f[12..] == want[12..len]).then_some((queue, seq))
}

#[test]
#[ignore = "started by the veth test inside a namespace"]
fn helper_process() {
    let Ok(role) = std::env::var("FPS_AFXDP_ROLE") else {
        return;
    };
    match role.as_str() {
        "modes" => helper_modes(&env_var("FPS_AFXDP_IFACE")),
        "echo" => helper_echo(&env_var("FPS_AFXDP_IFACE"), &queue_list()),
        "send" => helper_send(&env_var("FPS_AFXDP_IFACE"), &queue_list()),
        other => panic!("unknown role {other}"),
    }
}

fn helper_modes(iface: &str) {
    // Zero copy only: a veth pair has no zero copy driver.
    match open(iface, &[0], BindMode::ZeroCopy) {
        Ok(p) => {
            say(&format!("zerocopy=ok {}", describe(&p)));
            panic!("a veth pair bound in zero copy mode");
        }
        Err(e) if e.is_zero_copy_unsupported() => {
            say(&format!(
                "zerocopy=unsupported errno={}",
                e.raw_os_error().unwrap_or(0)
            ));
        }
        Err(e) => panic!("zero copy failed for another reason: {e:?}"),
    }
    // A refused setting never reaches the kernel.
    let bad = open_port(&PortConfig {
        queue: QueueConfig {
            ring_size: 100,
            ..QueueConfig::default()
        },
        ..PortConfig::new(iface)
    });
    assert!(matches!(bad, Err(AfXdpError::Config(_))), "{bad:?}");
    say("config=rejected");
    // Copy only.
    let p = open_after_close(iface, &[0], BindMode::Copy);
    assert_eq!(p.info.queues[0].bound, BoundMode::Copy);
    say(&format!("copy=ok {}", describe(&p)));
    drop(p);
    // Automatic mode falls back, and the socket says so.
    let p = open_after_close(iface, &[0], BindMode::Auto);
    let from_socket = p.queues[0].bind_mode().unwrap();
    say(&format!(
        "auto={} socket={} {}",
        mode_name(p.info.queues[0].bound),
        mode_name(from_socket),
        describe(&p)
    ));
    // A second port on the same device meets the first program.
    match open(iface, &[0], BindMode::Auto) {
        Ok(_) => panic!("two XDP programs on one interface"),
        Err(e) => {
            let name = if e.raw_os_error() == Some(16) {
                "EBUSY"
            } else {
                "other"
            };
            say(&format!("busy={name} error={e}"));
        }
    }
    drop(p);
    // Dropping the port detaches the program, so the next open works.
    drop(open_after_close(iface, &[0], BindMode::Auto));
    say("done");
}

/// Echoes every frame back with the two MAC addresses swapped.
struct Echo {
    buf: Vec<u8>,
    lens: Vec<usize>,
    echoed: Arc<AtomicU64>,
}

impl QueueLoop for Echo {
    fn turn(&mut self, io: &mut dyn FrameIo) -> std::io::Result<bool> {
        self.lens.clear();
        let (buf, lens) = (&mut self.buf, &mut self.lens);
        let mut off = 0;
        let n = io.recv_batch(32, &mut |f| {
            if f.len() < HEADER || f[12..14] != ETHERTYPE {
                return;
            }
            let end = off + f.len();
            buf[off..end].copy_from_slice(f);
            buf[off..off + 6].copy_from_slice(&f[6..12]);
            buf[off + 6..off + 12].copy_from_slice(&f[0..6]);
            lens.push(f.len());
            off = end;
        })?;
        self.echoed
            .fetch_add(self.lens.len() as u64, Ordering::Relaxed);
        let mut sent = 0;
        let deadline = Instant::now() + Duration::from_secs(5);
        while sent < self.lens.len() {
            let start: usize = self.lens[..sent].iter().sum();
            let (lens, buf) = (&self.lens, &self.buf);
            let mut at = start;
            let queued = io.send_batch_with(lens.len() - sent, &mut |i, out| {
                let len = lens[sent + i];
                out[..len].copy_from_slice(&buf[at..at + len]);
                at += len;
                len
            })?;
            sent += queued;
            if queued == 0 {
                io.flush()?;
                assert!(Instant::now() < deadline, "the transmit ring stayed full");
            }
        }
        Ok(n > 0)
    }
}

fn helper_echo(iface: &str, queues: &[u32]) {
    let port = open_after_close(iface, queues, BindMode::Auto);
    say(&format!("ready {}", describe(&port)));
    let counters: HashMap<u32, Arc<AtomicU64>> = queues
        .iter()
        .map(|q| (*q, Arc::new(AtomicU64::new(0))))
        .collect();
    let for_loops = counters.clone();
    let workers = Workers::spawn(
        port.queues,
        &WorkerOptions {
            idle: Idle::Wait(Duration::from_millis(2)),
            ..WorkerOptions::default()
        },
        move |q| Echo {
            buf: vec![0; 32 * 2048],
            lens: Vec::new(),
            echoed: Arc::clone(&for_loops[&q]),
        },
    )
    .unwrap();
    // The parent closes our stdin when the test is over.
    let mut sink = Vec::new();
    let _ = std::io::stdin().read_to_end(&mut sink);
    let mut failed = false;
    for r in workers.stop() {
        let error = r
            .error
            .as_ref()
            .map_or("none".to_owned(), |e| e.to_string().replace(' ', "_"));
        failed |= r.error.is_some();
        say(&format!(
            "echo queue={} echoed={} rx={} tx={} busy={} error={error}",
            r.queue_id,
            counters[&r.queue_id].load(Ordering::Relaxed),
            r.stats.rx_frames,
            r.stats.tx_frames,
            r.busy_turns
        ));
    }
    assert!(!failed, "a worker ended with an error");
}

fn helper_send(iface: &str, queues: &[u32]) {
    let count = frames_per_queue();
    let mut port = open_after_close(iface, queues, BindMode::Auto);
    let modes = describe(&port);
    let mut next = vec![0u64; queues.len()];
    let mut got = vec![0u64; queues.len()];
    let mut seen = vec![vec![false; count as usize]; queues.len()];
    let mut bad = 0u64;
    let deadline = Instant::now() + Duration::from_secs(45);
    let total = count * queues.len() as u64;
    while got.iter().sum::<u64>() < total && Instant::now() < deadline {
        for (i, q) in port.queues.iter_mut().enumerate() {
            let qid = q.queue_id();
            let outstanding = next[i] - got[i];
            let room = WINDOW
                .saturating_sub(outstanding)
                .min(count - next[i])
                .min(16);
            if room > 0 {
                let base = next[i];
                // Odd queues copy their frames through `send_batch`, and even
                // queues build them in place with `send_batch_with`, so both
                // calls cross the wire.
                let n = if i % 2 == 1 {
                    let frames: Vec<Vec<u8>> = (0..room)
                        .map(|k| {
                            let mut f = vec![0u8; MAX_LEN];
                            let len = build_frame(&mut f, qid, base + k);
                            f.truncate(len);
                            f
                        })
                        .collect();
                    let refs: Vec<&[u8]> = frames.iter().map(Vec::as_slice).collect();
                    q.send_batch(&refs).unwrap()
                } else {
                    q.send_batch_with(room as usize, &mut |k, buf| {
                        build_frame(buf, qid, base + k as u64)
                    })
                    .unwrap()
                };
                next[i] += n as u64;
            }
            q.flush().unwrap();
            let (mut ok, mut wrong) = (Vec::new(), 0u64);
            q.recv_batch(32, &mut |f| match check_frame(f) {
                Some((fq, seq)) if fq == qid && seq < count => ok.push(seq),
                _ => wrong += 1,
            })
            .unwrap();
            bad += wrong;
            for seq in ok {
                if std::mem::replace(&mut seen[i][seq as usize], true) {
                    bad += 1;
                } else {
                    got[i] += 1;
                }
            }
        }
        if got.iter().sum::<u64>() < total {
            for q in &mut port.queues {
                let _ = q.wait_readable(Duration::from_millis(1));
            }
        }
    }
    say(&format!(
        "sent={} received={} bad={bad} {modes}",
        next.iter().sum::<u64>(),
        got.iter().sum::<u64>()
    ));
    for q in &port.queues {
        let k = q.kernel_stats().unwrap();
        say(&format!(
            "kernel queue={} rx_dropped={} invalid={}",
            q.queue_id(),
            k.rx_dropped,
            k.rx_invalid_descs
        ));
    }
    assert_eq!(got.iter().sum::<u64>(), total, "frames were lost");
}
