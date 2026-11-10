// SPDX-License-Identifier: GPL-3.0-only
//! The TLS profiles of a run, which serve the `tls_upgrade` action, and the
//! handshake events that feed the `tls.*` metrics.
//!
//! # One connector per session
//!
//! A [`TlsConnector`] keeps a resumption store. Sessions that share a
//! connector resume, so against a server that issues tickets the second
//! ClientHello carries `pre_shared_key` (TLS 1.3) or a ticket (TLS 1.2) and its
//! JA4 differs from the first. A client profile that must keep one
//! fingerprint therefore registers a factory with
//! [`TlsProfiles::with_client_factory`], and the profiles build a fresh
//! connector for every session. The fresh connector has a fresh store, so no
//! session resumes, and the ClientHello keeps exactly the shape of its
//! profile. The resumption setting of the profile stays as it is, because
//! switching it to `Disabled` would drop the `session_ticket` extension of a
//! TLS 1.2 ClientHello and change the JA4 the other way. A build allocates one
//! rustls `ClientConfig` and a small session store, which is microseconds next
//! to a handshake with its key exchange and signatures.

use std::collections::HashMap;
use std::fmt;
use std::io;
use std::sync::{Arc, Mutex, PoisonError};

use fps_flow::program::TlsRole;
use fps_interp::{BoxedIo, TlsUpgrader, UpgradeError, UpgradeFuture, UpgradeRequest};
use fps_module_tls::{HandshakeReport, TlsAcceptor, TlsConnector, TlsError};
use fps_stats::Side as StatsSide;

/// The server name that a client handshake verifies when the action names none.
pub const DEFAULT_SERVER_NAME: &str = "localhost";

/// Builds the connector of one client session, see
/// [`TlsProfiles::with_client_factory`].
pub type ConnectorFactory = Arc<dyn Fn() -> Result<TlsConnector, TlsError> + Send + Sync>;

/// How a client profile gets its connector.
#[derive(Clone)]
enum ClientEntry {
    /// One connector for every session of the run, so sessions resume.
    Shared(TlsConnector),
    /// A connector that the factory builds for every session.
    PerSession(ConnectorFactory),
}

/// The named TLS profiles that the `tls_upgrade` actions of a flow resolve.
///
/// A client profile is a [`TlsConnector`], which fixes what the ClientHello
/// offers, the roots that verify the server, and the resumption store, and a
/// server profile is a [`TlsAcceptor`], which fixes what the server accepts
/// and the certificate it presents. The action names a profile, and its role
/// picks the table, so one name may exist on both tables.
///
/// A client profile comes in two forms. [`TlsProfiles::with_client`] shares one
/// connector between all sessions, so they resume when the server issues
/// tickets. [`TlsProfiles::with_client_factory`] builds a connector for every
/// session, so each one runs a full handshake with the same ClientHello. Use
/// the factory for a profile that imitates a client, whose fingerprint must not
/// change from one session to the next.
///
/// The key log of a profile receives the secrets of every handshake that
/// runs through it, see [`fps_module_tls::NssKeyLog`]. The capture tap sits
/// below the TLS layer, so a capture holds the encrypted records and the key
/// log decrypts them.
#[derive(Default)]
pub struct TlsProfiles {
    clients: HashMap<String, ClientEntry>,
    servers: HashMap<String, TlsAcceptor>,
    server_name: Option<String>,
}

impl TlsProfiles {
    /// Returns a registry without profiles.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds the client profile `name` with one connector for every session.
    ///
    /// The connector keeps its resumption store, so against a server that
    /// issues tickets the sessions after the first resume, and their
    /// ClientHello differs from the first. See
    /// [`TlsProfiles::with_client_factory`] for a profile that must not.
    #[must_use]
    pub fn with_client(mut self, name: impl Into<String>, connector: TlsConnector) -> Self {
        self.clients
            .insert(name.into(), ClientEntry::Shared(connector));
        self
    }

    /// Adds the client profile `name` and builds its connector with `factory`
    /// for every session, so no session resumes and every ClientHello has the
    /// shape of the profile.
    ///
    /// The factory runs on the worker thread when a session reaches its
    /// `tls_upgrade`, and an error fails that session with reason `internal`.
    /// It must build the same profile every time, and the connectors may share
    /// the key log of the profile. For example:
    ///
    /// ```ignore
    /// let profile = browser.client_profile();
    /// let roots = authority.root_store()?;
    /// let profiles = TlsProfiles::new().with_client_factory("chrome-136", move || {
    ///     TlsConnector::new(&profile, roots.clone())
    /// });
    /// ```
    #[must_use]
    pub fn with_client_factory(
        mut self,
        name: impl Into<String>,
        factory: impl Fn() -> Result<TlsConnector, TlsError> + Send + Sync + 'static,
    ) -> Self {
        self.clients
            .insert(name.into(), ClientEntry::PerSession(Arc::new(factory)));
        self
    }

    /// Adds the server profile `name`.
    #[must_use]
    pub fn with_server(mut self, name: impl Into<String>, acceptor: TlsAcceptor) -> Self {
        self.servers.insert(name.into(), acceptor);
        self
    }

    /// Sets the server name of client handshakes whose action sets none. The
    /// default is [`DEFAULT_SERVER_NAME`].
    #[must_use]
    pub fn with_server_name(mut self, name: impl Into<String>) -> Self {
        self.server_name = Some(name.into());
        self
    }
}

impl fmt::Debug for TlsProfiles {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut clients: Vec<_> = self
            .clients
            .iter()
            .map(|(k, v)| match v {
                ClientEntry::Shared(_) => format!("{k} (shared)"),
                ClientEntry::PerSession(_) => format!("{k} (per session)"),
            })
            .collect();
        let mut servers: Vec<_> = self.servers.keys().collect();
        clients.sort();
        servers.sort();
        f.debug_struct("TlsProfiles")
            .field("clients", &clients)
            .field("servers", &servers)
            .field("server_name", &self.server_name)
            .finish()
    }
}

/// What one TLS handshake of a session came to, for the `tls.*` metrics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TlsEvent {
    /// The handshake completed.
    Completed {
        side: StatsSide,
        /// Time from the start of the handshake to its end.
        elapsed_us: u64,
        resumed: bool,
    },
    /// The handshake failed, timed out, or was dropped before it ended.
    Failed {
        side: StatsSide,
        /// The alert that the peer sent, when the failure carries one.
        alert_rx: Option<String>,
        /// The alert that this side sent, when it is known.
        alert_tx: Option<String>,
    },
}

/// The handshake events of one session or one scenario instance, which the
/// upgrader fills while the session runs and the worker drains at its end.
#[derive(Debug, Clone, Default)]
pub(crate) struct TlsSink(Arc<Mutex<Vec<TlsEvent>>>);

impl TlsSink {
    fn push(&self, event: TlsEvent) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(event);
    }

    fn take(&self) -> Vec<TlsEvent> {
        std::mem::take(&mut *self.0.lock().unwrap_or_else(PoisonError::into_inner))
    }
}

/// Records a failed handshake when the handshake future is dropped before it
/// ended, which is how the open timeout of the interpreter cancels it.
struct Pending {
    sink: Option<TlsSink>,
    side: StatsSide,
}

impl Pending {
    /// Records `event`, if any, and disarms the guard.
    fn finish(mut self, event: Option<TlsEvent>) {
        if let (Some(sink), Some(event)) = (self.sink.take(), event) {
            sink.push(event);
        }
    }
}

impl Drop for Pending {
    fn drop(&mut self) {
        if let Some(sink) = self.sink.take() {
            sink.push(TlsEvent::Failed {
                side: self.side,
                alert_rx: None,
                alert_tx: None,
            });
        }
    }
}

/// The upgrader of one session or scenario instance: the profiles of the run,
/// and the sink that collects the handshake events for the metrics.
#[derive(Clone)]
pub(crate) struct SessionTls {
    profiles: Arc<TlsProfiles>,
    sink: TlsSink,
}

impl SessionTls {
    pub(crate) fn new(profiles: Arc<TlsProfiles>) -> Self {
        Self {
            profiles,
            sink: TlsSink::default(),
        }
    }

    /// Takes the handshake events recorded so far.
    pub(crate) fn take_events(&self) -> Vec<TlsEvent> {
        self.sink.take()
    }
}

impl TlsUpgrader for SessionTls {
    fn upgrade<'io>(&self, io: BoxedIo<'io>, request: UpgradeRequest<'_>) -> UpgradeFuture<'io> {
        self.profiles
            .upgrade_with(io, request, Some(self.sink.clone()))
    }
}

fn unavailable<'io>(detail: String) -> UpgradeFuture<'io> {
    Box::pin(std::future::ready(Err(UpgradeError::Unavailable(detail))))
}

/// Maps a failed handshake to an error that the interpreter reports as
/// `tls_failure`, whatever broke the transport underneath.
fn handshake_failure(e: TlsError) -> UpgradeError {
    match e {
        TlsError::Handshake { .. } => {
            UpgradeError::Handshake(io::Error::new(io::ErrorKind::InvalidData, e))
        }
        other => UpgradeError::Unavailable(other.to_string()),
    }
}

/// The event of a failed handshake, or `None` for an error that is not a
/// handshake failure, such as a profile that does not build.
fn failure_event(e: &TlsError, side: StatsSide) -> Option<TlsEvent> {
    match e {
        TlsError::Handshake {
            alert_received,
            alert_sent,
            ..
        } => Some(TlsEvent::Failed {
            side,
            alert_rx: alert_received.clone(),
            alert_tx: alert_sent.clone(),
        }),
        _ => None,
    }
}

fn completed(report: &HandshakeReport, side: StatsSide) -> TlsEvent {
    TlsEvent::Completed {
        side,
        elapsed_us: u64::try_from(report.elapsed.as_micros()).unwrap_or(u64::MAX),
        resumed: report.resumed,
    }
}

impl TlsProfiles {
    /// Runs the upgrade of `request` and reports its handshake to `sink`.
    fn upgrade_with<'io>(
        &self,
        io: BoxedIo<'io>,
        request: UpgradeRequest<'_>,
        sink: Option<TlsSink>,
    ) -> UpgradeFuture<'io> {
        match request.role {
            TlsRole::Client => {
                let connector = match self.clients.get(request.profile) {
                    None => {
                        return unavailable(format!(
                            "no client TLS profile '{}' in this run",
                            request.profile
                        ))
                    }
                    Some(ClientEntry::Shared(c)) => c.clone(),
                    Some(ClientEntry::PerSession(build)) => match build() {
                        Ok(c) => c,
                        Err(e) => {
                            return unavailable(format!(
                                "client TLS profile '{}': {e}",
                                request.profile
                            ))
                        }
                    },
                };
                let name = request
                    .server_name
                    .or(self.server_name.as_deref())
                    .unwrap_or(DEFAULT_SERVER_NAME)
                    .to_owned();
                Box::pin(async move {
                    let pending = Pending {
                        sink,
                        side: StatsSide::Client,
                    };
                    match connector.connect(io, &name).await {
                        Ok(secured) => {
                            pending.finish(Some(completed(secured.report(), StatsSide::Client)));
                            Ok(Box::new(secured) as BoxedIo<'io>)
                        }
                        Err(e) => {
                            pending.finish(failure_event(&e, StatsSide::Client));
                            Err(handshake_failure(e))
                        }
                    }
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
                    let pending = Pending {
                        sink,
                        side: StatsSide::Server,
                    };
                    match acceptor.accept(io).await {
                        Ok(secured) => {
                            pending.finish(Some(completed(secured.report(), StatsSide::Server)));
                            Ok(Box::new(secured) as BoxedIo<'io>)
                        }
                        Err(e) => {
                            pending.finish(failure_event(&e, StatsSide::Server));
                            Err(handshake_failure(e))
                        }
                    }
                })
            }
        }
    }
}

impl TlsUpgrader for TlsProfiles {
    fn upgrade<'io>(&self, io: BoxedIo<'io>, request: UpgradeRequest<'_>) -> UpgradeFuture<'io> {
        self.upgrade_with(io, request, None)
    }
}
