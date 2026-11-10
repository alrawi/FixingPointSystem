// SPDX-License-Identifier: GPL-3.0-only
//! The TLS profiles of a run, and the key log that a capture ships (AGT-14).
//!
//! A scenario upgrades a connection with the action `tls_upgrade`, which names
//! a profile. The agent knows two tables of profiles, which extend the
//! reference registry of the http1 module tests:
//!
//! | Role | Names |
//! |---|---|
//! | client | the browser profiles of the TLS module: `chrome-136`, `firefox-139`, `safari-18.5`, `edge-134`, and [`MAIL_CLIENT`], which is `firefox-139` without ALPN |
//! | server | [`SERVER_TLS13`] and [`SERVER_TLS12`], which accept one version, select the ALPN id `http/1.1`, [`SERVER_TLS13_H2`], which accepts TLS 1.3 and selects `h2` for a client that offers it, and [`SERVER_TLS13_NOALPN`], which accepts TLS 1.3 and selects no ALPN id; all present a certificate for [`SERVER_NAME`] |
//!
//! The mail client and the two TLS 1.3 server profiles are the ones that the
//! TLS module registers in `fps_module_tls::mail` (TRN-15), looked up by name
//! in [`client_profile`] and [`server_profile`] (AGT-17). The agent keeps no
//! copy of them. Only `server-tls13` and `server-tls12`, which the module
//! does not register, are defined here. The module's server profiles disable
//! resumption, so that a connector that several sessions share never resumes;
//! the agent builds one connector per session, see below, and sets the
//! resumption of every server profile it builds to stateless tickets.
//!
//! [`scan`] reads the compiled scenarios of the mix and refuses the plan at
//! prepare when an action names a profile that the table of its role lacks.
//! [`build`] then makes the `TlsProfiles` of the run for the names in use.
//!
//! # Trust between agents
//!
//! The two sides of a run are often different agents, and neither can ask the
//! other for a certificate: the only channel between them is the plan. Both
//! therefore derive the authority of the run from the run seed, see
//! [`authority`]. The key of the authority is a function of the seed, so the
//! server side issues a certificate that every client side of the run trusts,
//! and a run needs no exchange. Anyone who knows the seed can sign a
//! certificate that the clients of the run accept. The roots exist only inside
//! the connectors of the engine, never in a system store, and the traffic is
//! synthetic, so the agent accepts that.
//!
//! # One connector per session
//!
//! A connector with a resumption store changes its fingerprint from its second
//! handshake on: the ClientHello then carries `pre_shared_key`, which the JA4
//! of the profile does not count (MOD-04). The agent therefore registers every
//! client profile with [`TlsProfiles::with_client_factory`], so each session
//! builds its own connector with an empty store, never resumes, and sends the
//! ClientHello of its profile whatever the server issues. The server profiles
//! may issue tickets again, which a third party client may resume. [`build`]
//! makes one connector per profile first, so a profile that cannot be built
//! still refuses the plan at prepare and not in the middle of a run.
//!
//! # The key log
//!
//! Every connector and acceptor writes its secrets to a [`KeyTap`]. The tap
//! stores lines only while a capture that asked for them runs, see
//! [`crate::control::Captures`], so a run without such a capture keeps no
//! secret in memory.

use std::collections::BTreeSet;
use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use fps_flow::program::{Op, Program, TlsRole};
use fps_flow::Scenario;
use fps_module_tls::browser::BROWSER_PROFILES;
use fps_module_tls::mail;
use fps_module_tls::{
    ClientProfile, LeafKey, NssKeyLog, RunAuthority, ServerProfile, ServerResumption, TlsAcceptor,
    TlsConnector, TlsError, TlsVersion,
};
use fps_runtime::TlsProfiles;
use num_bigint::BigUint;
use rustls::KeyLog;
use sha2::{Digest, Sha256};

use crate::run::RunError;

/// The server profile that accepts only TLS 1.3.
pub const SERVER_TLS13: &str = "server-tls13";

/// The server profile that accepts only TLS 1.2.
pub const SERVER_TLS12: &str = "server-tls12";

/// The server profile that accepts only TLS 1.3 and selects the ALPN id `h2`
/// for a client that offers it, and `http/1.1` for one that offers that only.
/// The name of the profile of the TLS module.
pub const SERVER_TLS13_H2: &str = mail::SERVER_TLS13_H2;

/// The server profile that accepts only TLS 1.3 and selects no ALPN id, for
/// the clients of mail and other protocols that send none. The name of the
/// profile of the TLS module.
pub const SERVER_TLS13_NOALPN: &str = mail::SERVER_TLS13_NOALPN;

/// The client profile of a mail client, such as the STARTTLS sessions of smtp
/// and imap: the ClientHello of `firefox-139` without the ALPN extension.
/// Thunderbird shares the TLS stack of Firefox and offers no ALPN on the mail
/// ports. The profile follows the Firefox profile in everything else, and its
/// JA4 differs from the Firefox one only where the ALPN id counts. The name of
/// the profile of the TLS module.
pub const MAIL_CLIENT: &str = mail::MAIL_CLIENT;

/// The name in the certificate of the servers, which the https scenarios send
/// as SNI.
pub const SERVER_NAME: &str = "www.example.test";

/// The name that a client verifies when its action names none, which
/// `fps-runtime` fixes. The certificate carries it too.
const DEFAULT_NAME: &str = "localhost";

/// The common name of the authority of a run.
const AUTHORITY_NAME: &str = "FixingPointSystem run authority";

/// The most key log text that one capture keeps, 32 MiB. A handshake writes
/// about 600 bytes, so the limit holds tens of thousands of sessions.
pub const MAX_KEY_BYTES: usize = 32 << 20;

/// Returns the client profile `name`, or `None` when the agent has none. The
/// mail client is the profile of the TLS module, and the browser profiles are
/// those of its browser registry.
pub fn client_profile(name: &str) -> Option<ClientProfile> {
    if name == MAIL_CLIENT {
        return Some(mail::mail_client());
    }
    BROWSER_PROFILES
        .iter()
        .find(|b| b.name == name)
        .map(|b| b.client_profile())
}

/// Returns the names of the client profiles.
pub fn client_profiles() -> Vec<&'static str> {
    BROWSER_PROFILES
        .iter()
        .map(|b| b.name)
        .chain([MAIL_CLIENT])
        .collect()
}

/// Returns the names of the server profiles.
pub fn server_profiles() -> Vec<&'static str> {
    vec![
        SERVER_TLS13,
        SERVER_TLS12,
        SERVER_TLS13_H2,
        SERVER_TLS13_NOALPN,
    ]
}

/// Returns the server profile `name`, or `None` when the agent has none.
///
/// `server-tls13-h2` and `server-tls13-noalpn` come from the registry of the
/// TLS module. That registry disables resumption, and the agent enables
/// stateless tickets on its copy: every client session of the agent builds its
/// own connector, see the module documentation, so no client resumes, and a
/// third party client of the server may. `server-tls13` and `server-tls12`
/// accept one version each and select `http/1.1`.
pub fn server_profile(name: &str) -> Option<ServerProfile> {
    let one_version = |version| ServerProfile {
        name: name.to_owned(),
        versions: vec![version],
        alpn: vec![b"http/1.1".to_vec()],
        ..ServerProfile::default()
    };
    let profile = match name {
        SERVER_TLS13 => one_version(TlsVersion::Tls13),
        SERVER_TLS12 => one_version(TlsVersion::Tls12),
        _ => mail::server_profile(name)?,
    };
    Some(ServerProfile {
        resumption: ServerResumption::Tickets,
        ..profile
    })
}

/// Returns the ALPN ids that the server profile `name` selects from, in its
/// order, or `None` when the agent has no such server profile. An empty list
/// means that the profile selects no id.
pub fn server_alpn(name: &str) -> Option<Vec<Vec<u8>>> {
    server_profile(name).map(|p| p.alpn)
}

/// The profile names and server names that the scenarios of a plan use.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Needs {
    /// Client profiles, by name.
    pub clients: BTreeSet<String>,
    /// Server profiles, by name.
    pub servers: BTreeSet<String>,
    /// The server names that the client actions send, which the certificate
    /// must carry.
    pub names: BTreeSet<String>,
}

impl Needs {
    /// Returns true when no scenario upgrades a connection.
    pub fn is_empty(&self) -> bool {
        self.clients.is_empty() && self.servers.is_empty()
    }
}

fn walk(program: &Program, default: TlsRole, scenario: &str, needs: &mut Needs) -> Vec<String> {
    let mut errors = Vec::new();
    for op in &program.ops {
        let Op::UpgradeTls(u) = op else { continue };
        let role = u.role.unwrap_or(default);
        let (table, known): (&mut BTreeSet<String>, Vec<&str>) = match role {
            TlsRole::Client => (&mut needs.clients, client_profiles()),
            TlsRole::Server => (&mut needs.servers, server_profiles()),
        };
        if known.contains(&u.profile.as_str()) {
            table.insert(u.profile.clone());
        } else {
            errors.push(format!(
                "scenario {scenario}: tls_upgrade names the {} TLS profile '{}', which this agent does not provide; its {} profiles are {}",
                match role {
                    TlsRole::Client => "client",
                    TlsRole::Server => "server",
                },
                u.profile,
                match role {
                    TlsRole::Client => "client",
                    TlsRole::Server => "server",
                },
                known.join(", ")
            ));
        }
        if let Some(name) = &u.server_name {
            needs.names.insert(name.clone());
        }
    }
    errors
}

/// Reads the `tls_upgrade` actions of `scenarios`, given as scenario name and
/// compiled scenario, and returns the profiles that they need. An action that
/// names a profile that the table of its role lacks ends the plan with a rule
/// error that lists every such action.
pub fn scan<'s>(
    scenarios: impl IntoIterator<Item = (&'s str, &'s Scenario)>,
) -> Result<Needs, RunError> {
    let mut needs = Needs::default();
    let mut errors = Vec::new();
    for (name, scenario) in scenarios {
        for flow in &scenario.flows {
            errors.extend(walk(&flow.client, TlsRole::Client, name, &mut needs));
            if let Some(server) = &flow.server {
                errors.extend(walk(server, TlsRole::Server, name, &mut needs));
            }
        }
    }
    if errors.is_empty() {
        Ok(needs)
    } else {
        Err(RunError::rule(errors.join("; ")))
    }
}

// The curve P-256, FIPS 186-4 D.1.2.3.
const P: &str = "ffffffff00000001000000000000000000000000ffffffffffffffffffffffff";
const N: &str = "ffffffff00000000ffffffffffffffffbce6faada7179e84f3b9cac2fc632551";
const GX: &str = "6b17d1f2e12c4247f8bce6e563a440f277037d812deb33a0f4a13945d898c296";
const GY: &str = "4fe342e2fe1a7f9b8ee7eb4a7c0f9e162bce33576b315ececbb6406837bf51f5";

fn big(hex: &str) -> BigUint {
    BigUint::parse_bytes(hex.as_bytes(), 16).unwrap_or_default()
}

type Point = Option<(BigUint, BigUint)>;

struct Curve {
    p: BigUint,
}

impl Curve {
    fn inv(&self, x: &BigUint) -> BigUint {
        x.modpow(&(&self.p - 2u32), &self.p)
    }

    fn sub(&self, a: &BigUint, b: &BigUint) -> BigUint {
        ((a + &self.p) - (b % &self.p)) % &self.p
    }

    fn double(&self, a: &Point) -> Point {
        let (x, y) = a.as_ref()?;
        if *y == BigUint::default() {
            return None;
        }
        // The slope is (3 x^2 + a) / 2 y with a = -3.
        let num = (BigUint::from(3u32) * x * x + (&self.p - 3u32)) % &self.p;
        let l = num * self.inv(&((y * 2u32) % &self.p)) % &self.p;
        let x3 = self.sub(&(&l * &l % &self.p), &((x * 2u32) % &self.p));
        let y3 = self.sub(&(&l * self.sub(x, &x3) % &self.p), y);
        Some((x3, y3))
    }

    fn add(&self, a: &Point, b: &Point) -> Point {
        let (Some((x1, y1)), Some((x2, y2))) = (a, b) else {
            return a.clone().or_else(|| b.clone());
        };
        if x1 == x2 {
            return if y1 == y2 { self.double(a) } else { None };
        }
        let l = self.sub(y2, y1) * self.inv(&self.sub(x2, x1)) % &self.p;
        let x3 = self.sub(&self.sub(&(&l * &l % &self.p), x1), x2);
        let y3 = self.sub(&(&l * self.sub(x1, &x3) % &self.p), y1);
        Some((x3, y3))
    }

    /// Returns `d` times the base point. The loop is not constant time, which
    /// is fine for a key that the seed gives away anyway.
    fn mul_base(&self, d: &BigUint) -> Point {
        let mut acc: Point = None;
        let mut addend: Point = Some((big(GX), big(GY)));
        for i in 0..d.bits() {
            if d.bit(i) {
                acc = self.add(&acc, &addend);
            }
            addend = self.double(&addend);
        }
        acc
    }
}

/// Derives the PKCS#8 form of the P-256 key of the authority of a run from its
/// seed.
fn authority_key(seed: u64) -> Result<Vec<u8>, RunError> {
    let mut h = Sha256::new();
    h.update(b"fps-agent tls authority v1");
    h.update(seed.to_be_bytes());
    // The scalar is in 1..=n-1.
    let n = big(N);
    let d = BigUint::from_bytes_be(&h.finalize()) % (&n - 1u32) + 1u32;
    let curve = Curve { p: big(P) };
    let Some((x, y)) = curve.mul_base(&d) else {
        return Err(RunError::rule(
            "the authority key is the point at infinity".into(),
        ));
    };
    let pad = |v: &BigUint| {
        let bytes = v.to_bytes_be();
        let mut out = vec![0u8; 32usize.saturating_sub(bytes.len())];
        out.extend_from_slice(&bytes);
        out
    };
    // The PKCS#8 version 2 document that ring writes for P-256: the algorithm
    // identifiers, then the ECPrivateKey with its public key.
    let mut der = vec![
        0x30, 0x81, 0x87, 0x02, 0x01, 0x00, 0x30, 0x13, 0x06, 0x07, 0x2a, 0x86, 0x48, 0xce, 0x3d,
        0x02, 0x01, 0x06, 0x08, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07, 0x04, 0x6d, 0x30,
        0x6b, 0x02, 0x01, 0x01, 0x04, 0x20,
    ];
    der.extend_from_slice(&pad(&d));
    der.extend_from_slice(&[0xa1, 0x44, 0x03, 0x42, 0x00, 0x04]);
    der.extend_from_slice(&pad(&x));
    der.extend_from_slice(&pad(&y));
    Ok(der)
}

/// Returns the authority of the run `seed`. Two calls with one seed, in one
/// process or in two agents, give authorities that trust each other's
/// certificates, see the module documentation.
pub fn authority(seed: u64) -> Result<RunAuthority, RunError> {
    RunAuthority::from_pkcs8_der(AUTHORITY_NAME, &authority_key(seed)?).map_err(tls_error)
}

fn tls_error(e: TlsError) -> RunError {
    RunError::rule(format!("TLS profiles: {e}"))
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The key log lines of one capture, and whether the limit cut them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct KeyLines {
    /// NSS key log lines, one per secret, each ending in a newline.
    pub text: String,
    /// True when the lines hit [`MAX_KEY_BYTES`] and later handshakes are
    /// missing.
    pub truncated: bool,
}

#[derive(Default)]
struct Sink {
    active: bool,
    lines: KeyLines,
}

/// Receives the secrets of every TLS handshake of the agent and keeps them
/// while a capture that exports keys runs. Clones share one sink.
#[derive(Clone, Default)]
pub struct KeyTap {
    sink: Arc<Mutex<Sink>>,
}

impl fmt::Debug for KeyTap {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // The secrets stay out of debug output.
        f.debug_struct("KeyTap").finish_non_exhaustive()
    }
}

struct TapLog(Arc<Mutex<Sink>>);

impl fmt::Debug for TapLog {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TapLog").finish_non_exhaustive()
    }
}

impl KeyLog for TapLog {
    fn log(&self, label: &str, client_random: &[u8], secret: &[u8]) {
        let mut sink = lock(&self.0);
        if !sink.active || sink.lines.truncated {
            return;
        }
        let line = NssKeyLog::format_line(label, client_random, secret);
        if sink.lines.text.len() + line.len() > MAX_KEY_BYTES {
            sink.lines.truncated = true;
        } else {
            sink.lines.text.push_str(&line);
        }
    }

    fn will_log(&self, _label: &str) -> bool {
        lock(&self.0).active
    }
}

impl KeyTap {
    /// Returns a tap that keeps nothing until [`KeyTap::begin`].
    pub fn new() -> Self {
        Self::default()
    }

    /// Starts keeping lines, and drops those of an earlier capture.
    pub fn begin(&self) {
        let mut sink = lock(&self.sink);
        sink.active = true;
        sink.lines = KeyLines::default();
    }

    /// Stops keeping lines and returns what the tap kept.
    pub fn end(&self) -> KeyLines {
        let mut sink = lock(&self.sink);
        sink.active = false;
        std::mem::take(&mut sink.lines)
    }

    fn log(&self) -> Arc<dyn KeyLog> {
        Arc::new(TapLog(Arc::clone(&self.sink)))
    }
}

/// Builds the profiles that `needs` names, with the authority of the run
/// `seed` and the secrets going to `tap`. Returns `None` when no action
/// upgrades a connection, so a run without TLS builds nothing.
pub fn build(seed: u64, needs: &Needs, tap: &KeyTap) -> Result<Option<Arc<TlsProfiles>>, RunError> {
    if needs.is_empty() {
        return Ok(None);
    }
    let ca = authority(seed)?;
    let mut profiles = TlsProfiles::new();
    if !needs.clients.is_empty() {
        let roots = ca.root_store().map_err(tls_error)?;
        for name in client_profiles() {
            if !needs.clients.contains(name) {
                continue;
            }
            let Some(mut profile) = client_profile(name) else {
                return Err(RunError::rule(format!(
                    "TLS profiles: the client profile {name} cannot be built"
                )));
            };
            profile.key_log = Some(tap.log());
            // One connector now, so that a profile that cannot be built
            // refuses the plan here. Every session builds its own.
            TlsConnector::new(&profile, roots.clone()).map_err(tls_error)?;
            let roots = roots.clone();
            profiles = profiles
                .with_client_factory(name, move || TlsConnector::new(&profile, roots.clone()));
        }
    }
    if !needs.servers.is_empty() {
        let mut names: BTreeSet<&str> = needs.names.iter().map(String::as_str).collect();
        names.insert(SERVER_NAME);
        names.insert(DEFAULT_NAME);
        let names: Vec<&str> = names.into_iter().collect();
        let identity = ca.issue(&names, LeafKey::EcdsaP256).map_err(tls_error)?;
        for name in server_profiles() {
            if !needs.servers.contains(name) {
                continue;
            }
            let Some(mut profile) = server_profile(name) else {
                return Err(RunError::rule(format!(
                    "TLS profiles: the server profile {name} cannot be built"
                )));
            };
            // The profile issues stateless tickets. Every client session of
            // the agent has its own connector, so none resumes.
            profile.key_log = Some(tap.log());
            let acceptor = TlsAcceptor::new(&profile, &identity).map_err(tls_error)?;
            profiles = profiles.with_server(name, acceptor);
        }
    }
    Ok(Some(Arc::new(profiles)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(v: &BigUint) -> String {
        format!("{:0>64}", v.to_str_radix(16))
    }

    #[test]
    fn the_curve_arithmetic_matches_the_known_multiples_of_the_base_point() {
        let c = Curve { p: big(P) };
        let g = c.mul_base(&BigUint::from(1u32));
        assert_eq!(g, Some((big(GX), big(GY))));
        let two = c.mul_base(&BigUint::from(2u32));
        let (x, y) = two.expect("2G");
        assert_eq!(
            hex(&x),
            "7cf27b188d034f7e8a52380304b51ac3c08969e277f21b35a60b48fc47669978"
        );
        assert_eq!(
            hex(&y),
            "07775510db8ed040293d9ac69f7430dbba7dade63ce982299e04b79d227873d1"
        );
        let three = c.mul_base(&BigUint::from(3u32)).expect("3G");
        assert_eq!(
            hex(&three.0),
            "5ecbe4d1a6330a44c8f7ef951d4bf165e6c6b721efada985fb41661bc6e7fd6c"
        );
        // n * G is the point at infinity.
        assert_eq!(c.mul_base(&big(N)), None);
    }

    #[test]
    fn the_derived_key_is_a_valid_p256_key_that_depends_on_the_seed() {
        use ring::signature::{EcdsaKeyPair, ECDSA_P256_SHA256_ASN1_SIGNING};
        let a = authority_key(7).unwrap();
        assert_eq!(a, authority_key(7).unwrap());
        assert_ne!(a, authority_key(8).unwrap());
        // ring checks that the public key belongs to the private key.
        let rng = ring::rand::SystemRandom::new();
        EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, &a, &rng)
            .expect("ring accepts the derived key");
    }

    #[test]
    fn the_tap_keeps_lines_only_between_begin_and_end() {
        let tap = KeyTap::new();
        let log = tap.log();
        assert!(!log.will_log("CLIENT_RANDOM"));
        log.log("CLIENT_RANDOM", &[1, 2], &[3]);
        assert_eq!(tap.end(), KeyLines::default());
        tap.begin();
        assert!(log.will_log("CLIENT_RANDOM"));
        log.log("CLIENT_RANDOM", &[1, 2], &[3]);
        let lines = tap.end();
        assert_eq!(lines.text, "CLIENT_RANDOM 0102 03\n");
        assert!(!lines.truncated);
        log.log("CLIENT_RANDOM", &[1, 2], &[3]);
        assert_eq!(tap.end(), KeyLines::default());
    }

    fn scenario(client_profile: &str, server_profile: &str) -> Scenario {
        Scenario::parse(&format!(
            r#"
scenario: t
flows:
  - id: f
    module: m
    client:
      - connect: {{ port: 25 }}
      - tls_upgrade: {{ profile: {client_profile}, server_name: mx.example.test }}
      - close: fin
    server:
      - accept: {{ port: 25 }}
      - tls_upgrade: {{ profile: {server_profile} }}
      - close: fin
"#
        ))
        .unwrap()
    }

    #[test]
    fn scan_collects_the_names_and_refuses_unknown_ones() {
        let ok = scenario("firefox-139", SERVER_TLS12);
        let needs = scan([("m/t", &ok)]).unwrap();
        assert_eq!(needs.clients, BTreeSet::from(["firefox-139".to_owned()]));
        assert_eq!(needs.servers, BTreeSet::from([SERVER_TLS12.to_owned()]));
        assert_eq!(needs.names, BTreeSet::from(["mx.example.test".to_owned()]));

        let bad = scenario("lynx", "server-tls13");
        let e = scan([("m/t", &bad)]).unwrap_err().to_string();
        assert!(e.contains("lynx") && e.contains("chrome-136"), "{e}");
        // A server profile name does not serve a client action.
        let swapped = scenario(SERVER_TLS13, SERVER_TLS13);
        assert!(scan([("m/t", &swapped)]).is_err());
        // A scenario without the action needs nothing.
        let plain = Scenario::parse(
            "scenario: p\nflows:\n  - id: f\n    module: m\n    client:\n      - connect: { port: 1 }\n      - close: fin\n",
        )
        .unwrap();
        assert!(scan([("m/p", &plain)]).unwrap().is_empty());
        assert!(build(1, &Needs::default(), &KeyTap::new())
            .unwrap()
            .is_none());
    }

    /// AGT-17. The mail client and the two TLS 1.3 server profiles of the
    /// agent are the ones in the registry of the TLS module, not copies: the
    /// names are the module's constants, and every field the module sets
    /// reaches the agent. The one deliberate difference is the resumption of
    /// the server profiles, which the agent enables, see [`server_profile`].
    #[test]
    fn the_agent_profiles_are_the_profiles_of_the_tls_module() {
        assert_eq!(MAIL_CLIENT, mail::MAIL_CLIENT);
        assert_eq!(SERVER_TLS13_H2, mail::SERVER_TLS13_H2);
        assert_eq!(SERVER_TLS13_NOALPN, mail::SERVER_TLS13_NOALPN);

        let (agent, module) = (client_profile(MAIL_CLIENT).unwrap(), mail::mail_client());
        assert_eq!(agent.name, module.name);
        assert_eq!(agent.versions, module.versions);
        assert_eq!(agent.send_sni, module.send_sni);
        assert_eq!(agent.alpn, module.alpn);
        assert_eq!(agent.groups, module.groups);
        assert_eq!(agent.cipher_suites, module.cipher_suites);
        assert_eq!(agent.sig_schemes, module.sig_schemes);
        assert_eq!(agent.resumption, module.resumption);

        for (name, module) in [
            (SERVER_TLS13_H2, mail::server_tls13_h2()),
            (SERVER_TLS13_NOALPN, mail::server_tls13_noalpn()),
        ] {
            let agent = server_profile(name).unwrap();
            assert_eq!(agent.name, module.name, "{name}");
            assert_eq!(agent.versions, module.versions, "{name}");
            assert_eq!(agent.alpn, module.alpn, "{name}");
            assert_eq!(agent.groups, module.groups, "{name}");
            assert_eq!(agent.cipher_suites, module.cipher_suites, "{name}");
            assert_eq!(
                agent.use_server_cipher_order, module.use_server_cipher_order,
                "{name}"
            );
            // The module disables tickets, and the agent issues them.
            assert_eq!(module.resumption, ServerResumption::Disabled, "{name}");
            assert_eq!(agent.resumption, ServerResumption::Tickets, "{name}");
        }
        // Every server profile of the agent issues tickets, as before.
        for name in server_profiles() {
            let p = server_profile(name).unwrap();
            assert_eq!(p.resumption, ServerResumption::Tickets, "{name}");
        }
        assert!(server_profile("server-tls99").is_none());
        assert!(client_profile("lynx").is_none());
    }
}
