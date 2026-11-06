// SPDX-License-Identifier: GPL-3.0-only
//! Handshake rate benchmark of the TLS module, measured the way spike S2 does.
//!
//! `--mode config` (the default) repeats the method of `spikes/s2-tls`: a client
//! and a server run in memory on one thread, calls of each side are timed
//! separately, the server presents a two certificate chain, the client verifies
//! it, and TLS 1.3 handshakes include the delivery of the session tickets. The
//! only difference is that the rustls configurations come from this crate's
//! profiles, so the run shows what the profile code costs. `--mode session`
//! instead runs the full `TlsConnector` and `TlsAcceptor` over an in memory
//! `SessionIo` pair on a current thread runtime and reports the rate of both
//! sides together, which adds the cost of the async wrapper.
//!
//! ```text
//! cargo run --release -p fps-module-tls --example handshake_bench -- --duration 4 --json out.json
//! ```

use std::fmt::Write as _;
use std::process::Command;
use std::time::{Duration, Instant};

use fps_io::memory::duplex;
use fps_module_tls::{
    ClientProfile, ClientResumption, Group, LeafKey, RunAuthority, ServerIdentity, ServerProfile,
    TlsAcceptor, TlsConnector, TlsVersion,
};
use rustls::pki_types::ServerName;
use rustls::{ClientConnection, HandshakeKind, ServerConnection};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const SERVER_NAME: &str = "bench.example.test";

type Res<T> = Result<T, String>;

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Config,
    Session,
}

/// One profile of the matrix.
#[derive(Clone, Copy)]
struct Case {
    cert: &'static str,
    version: TlsVersion,
    group: Group,
    resumed_mode: bool,
}

struct Row {
    version: TlsVersion,
    cert: &'static str,
    group: Group,
    resumed_mode: bool,
    handshakes: u64,
    resumed_ratio: f64,
    client_us: f64,
    server_us: f64,
    both_us: f64,
}

fn rsa_key_pem() -> Res<String> {
    let out = Command::new("openssl")
        .args([
            "genpkey",
            "-algorithm",
            "RSA",
            "-pkeyopt",
            "rsa_keygen_bits:2048",
            "-quiet",
        ])
        .output()
        .map_err(|e| format!("running openssl to generate an RSA key: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "openssl failed: {}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    String::from_utf8(out.stdout).map_err(|e| e.to_string())
}

struct Rig {
    connector: TlsConnector,
    acceptor: TlsAcceptor,
}

fn rig(
    ca: &RunAuthority,
    identity: &ServerIdentity,
    version: TlsVersion,
    group: Group,
    resumed: bool,
) -> Res<Rig> {
    let client = ClientProfile {
        name: "bench".into(),
        versions: vec![version],
        groups: vec![group],
        resumption: if resumed {
            ClientResumption::InMemory { sessions: 256 }
        } else {
            ClientResumption::Disabled
        },
        ..ClientProfile::default()
    };
    let server = ServerProfile {
        name: "bench".into(),
        versions: vec![version],
        groups: vec![group],
        ..ServerProfile::default()
    };
    Ok(Rig {
        connector: TlsConnector::new(&client, ca.root_store().map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?,
        acceptor: TlsAcceptor::new(&server, identity).map_err(|e| e.to_string())?,
    })
}

#[derive(Default, Clone, Copy)]
struct Cost {
    client: Duration,
    server: Duration,
}

/// One in memory handshake, the loop of spike S2 including ticket delivery.
fn handshake(rig: &Rig, buf: &mut Vec<u8>) -> Res<(Cost, bool)> {
    let mut cost = Cost::default();
    let name = ServerName::try_from(SERVER_NAME).map_err(|e| e.to_string())?;
    let t = Instant::now();
    let mut client =
        ClientConnection::new(rig.connector.config().clone(), name).map_err(|e| e.to_string())?;
    cost.client += t.elapsed();
    let t = Instant::now();
    let mut server =
        ServerConnection::new(rig.acceptor.config().clone()).map_err(|e| e.to_string())?;
    cost.server += t.elapsed();

    for _ in 0..64 {
        buf.clear();
        let t = Instant::now();
        while client.wants_write() {
            client.write_tls(buf).map_err(|e| e.to_string())?;
        }
        cost.client += t.elapsed();
        if !buf.is_empty() {
            let t = Instant::now();
            let mut rd = &buf[..];
            while !rd.is_empty() {
                server.read_tls(&mut rd).map_err(|e| e.to_string())?;
                server
                    .process_new_packets()
                    .map_err(|e| format!("server: {e}"))?;
            }
            cost.server += t.elapsed();
        }
        buf.clear();
        let t = Instant::now();
        while server.wants_write() {
            server.write_tls(buf).map_err(|e| e.to_string())?;
        }
        cost.server += t.elapsed();
        if !buf.is_empty() {
            let t = Instant::now();
            let mut rd = &buf[..];
            while !rd.is_empty() {
                client.read_tls(&mut rd).map_err(|e| e.to_string())?;
                client
                    .process_new_packets()
                    .map_err(|e| format!("client: {e}"))?;
            }
            cost.client += t.elapsed();
        }
        if !client.is_handshaking()
            && !server.is_handshaking()
            && !client.wants_write()
            && !server.wants_write()
        {
            return Ok((
                cost,
                client.handshake_kind() == Some(HandshakeKind::Resumed),
            ));
        }
    }
    Err("handshake did not finish".into())
}

/// One handshake through the wrapper, followed by one byte from the server so
/// that the client reads its TLS 1.3 tickets. Returns the wall time.
async fn session_handshake(rig: &Rig) -> Res<(Duration, bool)> {
    let t = Instant::now();
    let (c, s) = duplex(64 * 1024);
    let (c, s) = tokio::join!(
        rig.connector.connect(c, SERVER_NAME),
        rig.acceptor.accept(s)
    );
    let (mut c, mut s) = (c.map_err(|e| e.to_string())?, s.map_err(|e| e.to_string())?);
    s.write_all(b"x").await.map_err(|e| e.to_string())?;
    let mut one = [0u8; 1];
    c.read_exact(&mut one).await.map_err(|e| e.to_string())?;
    Ok((t.elapsed(), c.report().resumed))
}

fn measure(
    ca: &RunAuthority,
    identity: &ServerIdentity,
    case: Case,
    secs: f64,
    mode: Mode,
) -> Res<Row> {
    let Case {
        cert,
        version,
        group,
        resumed_mode,
    } = case;
    let rig = rig(ca, identity, version, group, resumed_mode)?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .map_err(|e| e.to_string())?;
    let mut buf = Vec::with_capacity(16 * 1024);
    let (mut total, mut both) = (Cost::default(), Duration::ZERO);
    let (mut n, mut resumed) = (0u64, 0u64);
    let mut one = |measure: bool| -> Res<()> {
        let r = match mode {
            Mode::Config => {
                let (c, r) = handshake(&rig, &mut buf)?;
                if measure {
                    total.client += c.client;
                    total.server += c.server;
                }
                r
            }
            Mode::Session => {
                let (d, r) = runtime.block_on(session_handshake(&rig))?;
                if measure {
                    both += d;
                }
                r
            }
        };
        if measure {
            n += 1;
            resumed += u64::from(r);
        }
        Ok(())
    };
    // Warm up caches and prime the resumption store.
    for _ in 0..20 {
        one(false)?;
    }
    let start = Instant::now();
    while start.elapsed().as_secs_f64() < secs {
        one(true)?;
    }
    let (client_us, server_us, both_us) = match mode {
        Mode::Config => {
            let c = total.client.as_secs_f64() * 1e6 / n as f64;
            let s = total.server.as_secs_f64() * 1e6 / n as f64;
            (c, s, c + s)
        }
        Mode::Session => (f64::NAN, f64::NAN, both.as_secs_f64() * 1e6 / n as f64),
    };
    Ok(Row {
        version,
        cert,
        group,
        resumed_mode,
        handshakes: n,
        resumed_ratio: resumed as f64 / n as f64,
        client_us,
        server_us,
        both_us,
    })
}

fn main() -> Res<()> {
    let (mut secs, mut json, mut rsa, mut mode) = (3.0, None, true, Mode::Config);
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--duration" => {
                secs = args
                    .next()
                    .and_then(|v| v.parse().ok())
                    .ok_or("--duration needs seconds")?
            }
            "--json" => json = Some(args.next().ok_or("--json needs a path")?),
            "--no-rsa" => rsa = false,
            "--mode" => {
                mode = match args.next().as_deref() {
                    Some("config") => Mode::Config,
                    Some("session") => Mode::Session,
                    _ => return Err("--mode takes config or session".into()),
                }
            }
            other => return Err(format!("unknown argument `{other}`")),
        }
    }
    let ca = RunAuthority::generate("FixingPointSystem bench CA").map_err(|e| e.to_string())?;
    let mut certs = vec![("ecdsa_p256", LeafKey::EcdsaP256)];
    if rsa {
        certs.push(("rsa_2048", LeafKey::RsaPem(rsa_key_pem()?)));
    }
    println!(
        "{:<4} {:<11} {:<10} {:<8} {:>9} {:>8} {:>10} {:>10} {:>12} {:>12} {:>12}",
        "tls",
        "cert",
        "group",
        "mode",
        "handshake",
        "resumed",
        "client_us",
        "server_us",
        "client_hs/s",
        "server_hs/s",
        "both_hs/s"
    );
    let mut rows = Vec::new();
    for (cert, key) in certs {
        let identity = ca.issue(&[SERVER_NAME], key).map_err(|e| e.to_string())?;
        let mut cases = vec![];
        for version in [TlsVersion::Tls13, TlsVersion::Tls12] {
            for resumed in [false, true] {
                cases.push((version, Group::X25519, resumed));
            }
        }
        cases.push((TlsVersion::Tls13, Group::Secp256r1, false));
        for (version, group, resumed) in cases {
            let r = measure(
                &ca,
                &identity,
                Case {
                    cert,
                    version,
                    group,
                    resumed_mode: resumed,
                },
                secs,
                mode,
            )?;
            println!(
                "{:<4} {:<11} {:<10} {:<8} {:>9} {:>8.2} {:>10.1} {:>10.1} {:>12.0} {:>12.0} {:>12.0}",
                r.version.as_str(),
                r.cert,
                r.group.as_str(),
                if r.resumed_mode { "resumed" } else { "full" },
                r.handshakes,
                r.resumed_ratio,
                r.client_us,
                r.server_us,
                1e6 / r.client_us,
                1e6 / r.server_us,
                1e6 / r.both_us
            );
            rows.push(r);
        }
    }
    if let Some(path) = json {
        let mut out = String::from("[\n");
        for (i, r) in rows.iter().enumerate() {
            let num = |v: f64| {
                if v.is_finite() {
                    format!("{v}")
                } else {
                    "null".to_owned()
                }
            };
            let _ = writeln!(
                out,
                "  {{\"version\": \"{}\", \"cert\": \"{}\", \"group\": \"{}\", \"mode\": \"{}\", \"handshakes\": {}, \"resumed_ratio\": {}, \"client_us_per_handshake\": {}, \"server_us_per_handshake\": {}, \"client_handshakes_per_core_s\": {}, \"server_handshakes_per_core_s\": {}, \"both_sides_handshakes_per_core_s\": {}}}{}",
                r.version.as_str(),
                r.cert,
                r.group.as_str(),
                if r.resumed_mode { "resumed" } else { "full" },
                r.handshakes,
                r.resumed_ratio,
                num(r.client_us),
                num(r.server_us),
                num(1e6 / r.client_us),
                num(1e6 / r.server_us),
                num(1e6 / r.both_us),
                if i + 1 < rows.len() { "," } else { "" }
            );
        }
        out.push_str("]\n");
        std::fs::write(&path, out).map_err(|e| e.to_string())?;
    }
    Ok(())
}
