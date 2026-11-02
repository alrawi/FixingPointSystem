//! Phase 0 spike S2: TLS handshake cost per core.
//!
//! A client and a server run in memory on one thread, so the measurement
//! excludes the network and isolates cryptography and state machine cost. We
//! time client calls and server calls separately, because an agent emulates
//! the client side only in client simulation and both sides in application
//! simulation.

use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context, Result};
use clap::Parser;
use rcgen::{
    BasicConstraints, CertificateParams, DnType, IsCa, KeyPair, PKCS_ECDSA_P256_SHA256,
    PKCS_RSA_SHA256,
};
use rustls::client::Resumption;
use rustls::crypto::{ring as provider, CryptoProvider, SupportedKxGroup};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName};
use rustls::{
    ClientConfig, ClientConnection, HandshakeKind, RootCertStore, ServerConfig, ServerConnection,
    SupportedProtocolVersion,
};
use serde::Serialize;

const SERVER_NAME: &str = "bench.example.test";

#[derive(Parser)]
#[command(
    version,
    about = "FixingPointSystem Phase 0 spike S2: TLS handshake cost per core"
)]
struct Cli {
    /// Seconds to measure each profile.
    #[arg(long, default_value_t = 3.0)]
    duration: f64,
    /// Write results as JSON to this file.
    #[arg(long)]
    json: Option<std::path::PathBuf>,
    /// Skip RSA profiles (they need the openssl command to generate a key).
    #[arg(long)]
    no_rsa: bool,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
enum CertKind {
    #[serde(rename = "ecdsa_p256")]
    EcdsaP256,
    #[serde(rename = "rsa_2048")]
    Rsa2048,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum Mode {
    Full,
    Resumed,
}

#[derive(Clone, Copy, Debug, Serialize)]
enum Group {
    #[serde(rename = "x25519")]
    X25519,
    #[serde(rename = "secp256r1")]
    Secp256r1,
}

#[derive(Debug, Serialize)]
struct Row {
    version: &'static str,
    cert: CertKind,
    group: Group,
    mode: Mode,
    handshakes: u64,
    resumed_ratio: f64,
    client_us_per_handshake: f64,
    server_us_per_handshake: f64,
    client_handshakes_per_core_s: f64,
    server_handshakes_per_core_s: f64,
    both_sides_handshakes_per_core_s: f64,
}

struct Identity {
    chain: Vec<CertificateDer<'static>>,
    key: PrivatePkcs8KeyDer<'static>,
    roots: RootCertStore,
}

fn rsa_key_pem() -> Result<String> {
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
        .context("running openssl to generate an RSA key")?;
    if !out.status.success() {
        bail!("openssl failed: {}", String::from_utf8_lossy(&out.stderr));
    }
    Ok(String::from_utf8(out.stdout)?)
}

fn identity(kind: CertKind) -> Result<Identity> {
    let ca_key = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256)?;
    let mut ca_params = CertificateParams::new(Vec::<String>::new())?;
    ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    ca_params
        .distinguished_name
        .push(DnType::CommonName, "FixingPointSystem bench CA");
    let ca = ca_params.self_signed(&ca_key)?;

    let leaf_key = match kind {
        CertKind::EcdsaP256 => KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256)?,
        CertKind::Rsa2048 => KeyPair::from_pem_and_sign_algo(&rsa_key_pem()?, &PKCS_RSA_SHA256)?,
    };
    let mut leaf_params = CertificateParams::new(vec![SERVER_NAME.to_string()])?;
    leaf_params
        .distinguished_name
        .push(DnType::CommonName, SERVER_NAME);
    let leaf = leaf_params.signed_by(&leaf_key, &ca, &ca_key)?;

    let mut roots = RootCertStore::empty();
    roots.add(ca.der().clone())?;
    Ok(Identity {
        chain: vec![leaf.der().clone(), ca.der().clone()],
        key: PrivatePkcs8KeyDer::from(leaf_key.serialize_der()),
        roots,
    })
}

fn crypto(group: Group) -> Arc<CryptoProvider> {
    let mut p = provider::default_provider();
    let g: &'static dyn SupportedKxGroup = match group {
        Group::X25519 => provider::kx_group::X25519,
        Group::Secp256r1 => provider::kx_group::SECP256R1,
    };
    p.kx_groups = vec![g];
    Arc::new(p)
}

fn configs(
    id: &Identity,
    version: &'static SupportedProtocolVersion,
    group: Group,
    mode: Mode,
) -> Result<(Arc<ClientConfig>, Arc<ServerConfig>)> {
    let prov = crypto(group);
    let mut server = ServerConfig::builder_with_provider(prov.clone())
        .with_protocol_versions(&[version])?
        .with_no_client_auth()
        .with_single_cert(id.chain.clone(), PrivateKeyDer::Pkcs8(id.key.clone_key()))?;
    server.ticketer = provider::Ticketer::new()?;
    let mut client = ClientConfig::builder_with_provider(prov)
        .with_protocol_versions(&[version])?
        .with_root_certificates(id.roots.clone())
        .with_no_client_auth();
    client.resumption = match mode {
        Mode::Full => Resumption::disabled(),
        Mode::Resumed => Resumption::in_memory_sessions(256),
    };
    Ok((Arc::new(client), Arc::new(server)))
}

#[derive(Default, Clone, Copy)]
struct Cost {
    client: Duration,
    server: Duration,
}

/// Run one handshake in memory, including delivery of TLS 1.3 tickets.
fn handshake(
    ccfg: &Arc<ClientConfig>,
    scfg: &Arc<ServerConfig>,
    buf: &mut Vec<u8>,
) -> Result<(Cost, bool)> {
    let mut cost = Cost::default();
    let name = ServerName::try_from(SERVER_NAME)?;
    let t = Instant::now();
    let mut client = ClientConnection::new(ccfg.clone(), name)?;
    cost.client += t.elapsed();
    let t = Instant::now();
    let mut server = ServerConnection::new(scfg.clone())?;
    cost.server += t.elapsed();

    for _ in 0..64 {
        buf.clear();
        let t = Instant::now();
        while client.wants_write() {
            client.write_tls(buf)?;
        }
        cost.client += t.elapsed();
        if !buf.is_empty() {
            let t = Instant::now();
            let mut rd = &buf[..];
            while !rd.is_empty() {
                server.read_tls(&mut rd)?;
                server
                    .process_new_packets()
                    .map_err(|e| anyhow!("server: {e}"))?;
            }
            cost.server += t.elapsed();
        }
        buf.clear();
        let t = Instant::now();
        while server.wants_write() {
            server.write_tls(buf)?;
        }
        cost.server += t.elapsed();
        if !buf.is_empty() {
            let t = Instant::now();
            let mut rd = &buf[..];
            while !rd.is_empty() {
                client.read_tls(&mut rd)?;
                client
                    .process_new_packets()
                    .map_err(|e| anyhow!("client: {e}"))?;
            }
            cost.client += t.elapsed();
        }
        if !client.is_handshaking()
            && !server.is_handshaking()
            && !client.wants_write()
            && !server.wants_write()
        {
            let resumed = client.handshake_kind() == Some(HandshakeKind::Resumed);
            return Ok((cost, resumed));
        }
    }
    bail!("handshake did not finish")
}

fn measure(
    id: &Identity,
    cert: CertKind,
    version: &'static SupportedProtocolVersion,
    group: Group,
    mode: Mode,
    secs: f64,
) -> Result<Row> {
    let (ccfg, scfg) = configs(id, version, group, mode)?;
    let mut buf = Vec::with_capacity(16 * 1024);
    // Warm up caches, and prime the resumption store.
    for _ in 0..20 {
        handshake(&ccfg, &scfg, &mut buf)?;
    }
    let mut total = Cost::default();
    let (mut n, mut resumed) = (0u64, 0u64);
    let start = Instant::now();
    while start.elapsed().as_secs_f64() < secs {
        let (c, r) = handshake(&ccfg, &scfg, &mut buf)?;
        total.client += c.client;
        total.server += c.server;
        n += 1;
        resumed += r as u64;
    }
    let cus = total.client.as_secs_f64() * 1e6 / n as f64;
    let sus = total.server.as_secs_f64() * 1e6 / n as f64;
    Ok(Row {
        version: if version.version == rustls::ProtocolVersion::TLSv1_3 {
            "1.3"
        } else {
            "1.2"
        },
        cert,
        group,
        mode,
        handshakes: n,
        resumed_ratio: resumed as f64 / n as f64,
        client_us_per_handshake: cus,
        server_us_per_handshake: sus,
        client_handshakes_per_core_s: 1e6 / cus,
        server_handshakes_per_core_s: 1e6 / sus,
        both_sides_handshakes_per_core_s: 1e6 / (cus + sus),
    })
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let mut certs = vec![CertKind::EcdsaP256];
    if !cli.no_rsa {
        certs.push(CertKind::Rsa2048);
    }
    let mut rows = Vec::new();
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
    for cert in certs {
        let id = identity(cert)?;
        let mut cases = vec![];
        for version in [&rustls::version::TLS13, &rustls::version::TLS12] {
            for mode in [Mode::Full, Mode::Resumed] {
                cases.push((version, Group::X25519, mode));
            }
        }
        cases.push((&rustls::version::TLS13, Group::Secp256r1, Mode::Full));
        for (version, group, mode) in cases {
            let r = measure(&id, cert, version, group, mode, cli.duration)?;
            println!("{:<4} {:<11} {:<10} {:<8} {:>9} {:>8.2} {:>10.1} {:>10.1} {:>12.0} {:>12.0} {:>12.0}",
                     r.version, format!("{:?}", r.cert), format!("{:?}", r.group), format!("{:?}", r.mode),
                     r.handshakes, r.resumed_ratio, r.client_us_per_handshake, r.server_us_per_handshake,
                     r.client_handshakes_per_core_s, r.server_handshakes_per_core_s, r.both_sides_handshakes_per_core_s);
            rows.push(r);
        }
    }
    if let Some(path) = cli.json {
        std::fs::write(&path, serde_json::to_string_pretty(&rows)?)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check(cert: CertKind) {
        let id = identity(cert).unwrap();
        for version in [&rustls::version::TLS13, &rustls::version::TLS12] {
            let (c, s) = configs(&id, version, Group::X25519, Mode::Full).unwrap();
            let mut buf = Vec::new();
            let (_, resumed) = handshake(&c, &s, &mut buf).unwrap();
            assert!(!resumed);
            let (c, s) = configs(&id, version, Group::X25519, Mode::Resumed).unwrap();
            handshake(&c, &s, &mut buf).unwrap();
            let (_, resumed) = handshake(&c, &s, &mut buf).unwrap();
            assert!(resumed, "second handshake should resume");
        }
    }

    #[test]
    fn ecdsa_full_and_resumed() {
        check(CertKind::EcdsaP256);
    }

    #[test]
    fn rsa_full_and_resumed() {
        if Command::new("openssl").arg("version").output().is_err() {
            eprintln!("openssl not found; skipping RSA test");
            return;
        }
        check(CertKind::Rsa2048);
    }
}
