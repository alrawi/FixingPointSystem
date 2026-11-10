// SPDX-License-Identifier: GPL-3.0-only
//! The TLS profiles that the https scenarios of the http2 module name.
//!
//! The scenarios open a plain connection and upgrade it at once with the
//! `tls_upgrade` action, so the interpreter needs a [`TlsUpgrader`]. The agent
//! builds one from the plan (AGT-14), and until then this registry serves the
//! module tests and the fixture peer. It follows `fps_runtime::TlsProfiles`,
//! which the test crate cannot use without the WebAssembly runtime behind it.
//!
//! The client table holds the four browser profiles of the TLS module under
//! their own names, and the server table holds the acceptor `server-tls13-h2`.
//! A client handshake builds its connector from the profile, so every session
//! starts with an empty resumption store, see [`Profiles::with_clients`].
//! The names that the scenarios use are the constants of this file. Every
//! acceptor presents a certificate for [`SERVER_NAME`] from a run authority,
//! the test PKI of the TLS module, and offers only `h2`, so a browser that
//! offers `h2` and `http/1.1` ends up on `h2`. The agent has no such profile
//! yet: its server profiles select `http/1.1` (AGT-14), so the agent needs
//! `server-tls13-h2` before it can run these scenarios.
#![allow(dead_code)] // The tests and the fixture peer use different subsets.

use std::collections::HashMap;
use std::io;
use std::sync::{Arc, Mutex, MutexGuard};

use fps_flow::program::TlsRole;
use fps_interp::{BoxedIo, TlsUpgrader, UpgradeError, UpgradeFuture, UpgradeRequest};
use fps_module_tls::browser::BROWSER_PROFILES;
use fps_module_tls::{
    ClientProfile, HandshakeReport, LeafKey, NssKeyLog, RunAuthority, ServerIdentity,
    ServerProfile, TlsAcceptor, TlsConnector, TlsError, TlsVersion,
};
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::CertificateDer;
use rustls::RootCertStore;

/// The name in the certificate of the server and in the SNI of the clients.
pub const SERVER_NAME: &str = "www.example.test";

/// The server profile that accepts only TLS 1.3 and selects `h2`.
pub const SERVER_TLS13_H2: &str = "server-tls13-h2";

/// The ALPN id that the server selects.
pub const ALPN_H2: &[u8] = b"h2";

/// The named TLS profiles of one run and the reports of the handshakes that
/// went through them.
pub struct Profiles {
    clients: HashMap<String, ClientProfile>,
    roots: RootCertStore,
    servers: HashMap<String, TlsAcceptor>,
    reports: Arc<Mutex<Vec<HandshakeReport>>>,
}

impl Default for Profiles {
    fn default() -> Self {
        Self {
            clients: HashMap::new(),
            roots: RootCertStore::empty(),
            servers: HashMap::new(),
            reports: Arc::default(),
        }
    }
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

/// Generates the authority of a run, which stands in for the test PKI.
pub fn authority() -> Result<RunAuthority, TlsError> {
    RunAuthority::generate("FixingPointSystem test CA")
}

/// Issues the server certificate for [`SERVER_NAME`].
pub fn identity(authority: &RunAuthority) -> Result<ServerIdentity, TlsError> {
    authority.issue(&[SERVER_NAME], LeafKey::EcdsaP256)
}

/// Reads the certificate of an authority from its PEM form, as a client in
/// another process does after the server wrote it.
pub fn roots_from_pem(pem: &[u8]) -> Result<RootCertStore, String> {
    let mut roots = RootCertStore::empty();
    let cert = CertificateDer::from_pem_slice(pem).map_err(|e| format!("read the CA PEM: {e}"))?;
    roots
        .add(cert)
        .map_err(|e| format!("add the CA to the root store: {e}"))?;
    Ok(roots)
}

impl Profiles {
    /// Builds the client table: every browser profile of the TLS module, with
    /// the server verified against `roots` and the secrets of every handshake
    /// written to `key_log`.
    ///
    /// Each client handshake builds a fresh connector, so no session resumes
    /// an earlier one. This matters for JA4. A connector with a resumption
    /// store sends the `session_ticket` extension, which the expected JA4 of
    /// every browser profile counts, and a connector without one leaves it out.
    /// A connector that resumed a TLS 1.3 session adds `pre_shared_key` to the
    /// ClientHello. Only the first handshake of a connector therefore has the
    /// JA4 that the profile documents, and a shared connector would break the
    /// fingerprint from the second session on.
    pub fn with_clients(
        mut self,
        roots: &RootCertStore,
        key_log: &Arc<NssKeyLog>,
    ) -> Result<Self, TlsError> {
        for browser in &BROWSER_PROFILES {
            let mut profile = browser.client_profile();
            profile.key_log = Some(key_log.clone());
            self.clients.insert(browser.name.to_owned(), profile);
        }
        self.roots = roots.clone();
        Ok(self)
    }

    /// Builds the server table: [`SERVER_TLS13_H2`] with `identity`, TLS 1.3
    /// only, and the ALPN id `h2`.
    pub fn with_servers(mut self, identity: &ServerIdentity) -> Result<Self, TlsError> {
        let profile = ServerProfile {
            name: SERVER_TLS13_H2.into(),
            versions: vec![TlsVersion::Tls13],
            alpn: vec![ALPN_H2.to_vec()],
            ..ServerProfile::default()
        };
        self.servers.insert(
            SERVER_TLS13_H2.to_owned(),
            TlsAcceptor::new(&profile, identity)?,
        );
        Ok(self)
    }

    /// Returns the reports of the handshakes so far, in completion order.
    pub fn reports(&self) -> Vec<HandshakeReport> {
        lock(&self.reports).clone()
    }
}

fn unavailable<'io>(detail: String) -> UpgradeFuture<'io> {
    Box::pin(std::future::ready(Err(UpgradeError::Unavailable(detail))))
}

fn failure(e: TlsError) -> UpgradeError {
    match e {
        TlsError::Handshake { .. } => {
            UpgradeError::Handshake(io::Error::new(io::ErrorKind::InvalidData, e))
        }
        other => UpgradeError::Unavailable(other.to_string()),
    }
}

impl TlsUpgrader for Profiles {
    fn upgrade<'io>(&self, io: BoxedIo<'io>, request: UpgradeRequest<'_>) -> UpgradeFuture<'io> {
        let reports = Arc::clone(&self.reports);
        match request.role {
            TlsRole::Client => {
                let Some(profile) = self.clients.get(request.profile) else {
                    return unavailable(format!(
                        "no client TLS profile '{}' in this run",
                        request.profile
                    ));
                };
                let connector = match TlsConnector::new(profile, self.roots.clone()) {
                    Ok(connector) => connector,
                    Err(e) => return unavailable(e.to_string()),
                };
                let name = request.server_name.unwrap_or(SERVER_NAME).to_owned();
                Box::pin(async move {
                    let session = connector.connect(io, &name).await.map_err(failure)?;
                    lock(&reports).push(session.report().clone());
                    Ok(Box::new(session) as BoxedIo<'io>)
                })
            }
            TlsRole::Server => {
                let Some(acceptor) = self.servers.get(request.profile).cloned() else {
                    return unavailable(format!(
                        "no server TLS profile '{}' in this run",
                        request.profile
                    ));
                };
                Box::pin(async move {
                    let session = acceptor.accept(io).await.map_err(failure)?;
                    lock(&reports).push(session.report().clone());
                    Ok(Box::new(session) as BoxedIo<'io>)
                })
            }
        }
    }
}
