// SPDX-License-Identifier: GPL-3.0-only
//! The TLS profiles of a run, which serve the `tls_upgrade` action.

use std::collections::HashMap;
use std::fmt;
use std::io;

use fps_flow::program::TlsRole;
use fps_interp::{BoxedIo, TlsUpgrader, UpgradeError, UpgradeFuture, UpgradeRequest};
use fps_module_tls::{TlsAcceptor, TlsConnector, TlsError};

/// The server name that a client handshake verifies when the action names none.
pub const DEFAULT_SERVER_NAME: &str = "localhost";

/// The named TLS profiles that the `tls_upgrade` actions of a flow resolve.
///
/// A client profile is a [`TlsConnector`], which fixes what the ClientHello
/// offers, the roots that verify the server, and the resumption store, and a
/// server profile is a [`TlsAcceptor`], which fixes what the server accepts
/// and the certificate it presents. The action names a profile, and its role
/// picks the table, so one name may exist on both tables.
///
/// The key log of a profile receives the secrets of every handshake that
/// runs through it, see [`fps_module_tls::NssKeyLog`]. The capture tap sits
/// below the TLS layer, so a capture holds the encrypted records and the key
/// log decrypts them.
#[derive(Default)]
pub struct TlsProfiles {
    clients: HashMap<String, TlsConnector>,
    servers: HashMap<String, TlsAcceptor>,
    server_name: Option<String>,
}

impl TlsProfiles {
    /// Returns a registry without profiles.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds the client profile `name`.
    #[must_use]
    pub fn with_client(mut self, name: impl Into<String>, connector: TlsConnector) -> Self {
        self.clients.insert(name.into(), connector);
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
        let mut clients: Vec<_> = self.clients.keys().collect();
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

impl TlsUpgrader for TlsProfiles {
    fn upgrade<'io>(&self, io: BoxedIo<'io>, request: UpgradeRequest<'_>) -> UpgradeFuture<'io> {
        match request.role {
            TlsRole::Client => {
                let Some(connector) = self.clients.get(request.profile).cloned() else {
                    return unavailable(format!(
                        "no client TLS profile '{}' in this run",
                        request.profile
                    ));
                };
                let name = request
                    .server_name
                    .or(self.server_name.as_deref())
                    .unwrap_or(DEFAULT_SERVER_NAME)
                    .to_owned();
                Box::pin(async move {
                    let secured = connector
                        .connect(io, &name)
                        .await
                        .map_err(handshake_failure)?;
                    Ok(Box::new(secured) as BoxedIo<'io>)
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
                    let secured = acceptor.accept(io).await.map_err(handshake_failure)?;
                    Ok(Box::new(secured) as BoxedIo<'io>)
                })
            }
        }
    }
}
