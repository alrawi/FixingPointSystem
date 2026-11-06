// SPDX-License-Identifier: GPL-3.0-only
//! Errors of the TLS module.

use std::fmt;
use std::io;

/// Side of a TLS session, with the label values of the metric catalog.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Side {
    /// The side that sent the ClientHello.
    Client,
    /// The side that answered it.
    Server,
}

impl Side {
    /// Returns the value of the `side` metric label.
    pub fn as_str(self) -> &'static str {
        match self {
            Side::Client => "client",
            Side::Server => "server",
        }
    }
}

impl fmt::Display for Side {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Errors that the TLS module reports.
#[derive(Debug, thiserror::Error)]
pub enum TlsError {
    /// A profile or a name did not describe a usable configuration.
    #[error("tls configuration: {0}")]
    Config(String),
    /// The certificate authority could not generate or sign a certificate.
    #[error("tls authority: {step}")]
    Authority {
        /// The step that failed.
        step: &'static str,
        /// The error of the certificate library.
        #[source]
        source: rcgen::Error,
    },
    /// The handshake failed or the transport broke during it.
    #[error("tls {side} handshake with {peer} failed: {source}")]
    Handshake {
        /// The side that ran the handshake.
        side: Side,
        /// The server name of a client, or `client` for a server.
        peer: String,
        /// The transport or protocol error. A protocol error carries a
        /// [`rustls::Error`] that `get_ref` and `downcast_ref` reach.
        #[source]
        source: io::Error,
        /// The alert that the peer sent, as a lower snake case name such as
        /// `handshake_failure`, the value of the `alert` label of `tls.alerts.rx`.
        alert_received: Option<String>,
        /// The alert that this side sent, when it is known. Alerts that go out
        /// encrypted are known only when the error implies them.
        alert_sent: Option<String>,
    },
}

impl TlsError {
    /// Returns the failure reason of the metric catalog for a failed session.
    pub fn failure_reason(&self) -> &'static str {
        "tls_failure"
    }
}

impl From<TlsError> for io::Error {
    fn from(e: TlsError) -> Self {
        let kind = match &e {
            TlsError::Handshake { source, .. } => source.kind(),
            _ => io::ErrorKind::InvalidInput,
        };
        io::Error::new(kind, e)
    }
}

/// Converts a CamelCase name to lower snake case, the form of the `alert` label.
pub(crate) fn snake_case(name: &str) -> String {
    let chars: Vec<char> = name.chars().collect();
    let mut out = String::with_capacity(name.len() + 4);
    for (i, &c) in chars.iter().enumerate() {
        if c.is_ascii_uppercase() {
            // A word starts after a lowercase letter, or before the lowercase
            // tail of an acronym, so UnknownCA becomes unknown_ca.
            let prev = i.checked_sub(1).and_then(|p| chars.get(p));
            let next = chars.get(i + 1);
            let starts_word = match prev {
                None => false,
                Some(p) if p.is_ascii_lowercase() || p.is_ascii_digit() => true,
                Some(_) => next.is_some_and(|n| n.is_ascii_lowercase()),
            };
            if starts_word {
                out.push('_');
            }
            out.push(c.to_ascii_lowercase());
        } else if c.is_ascii_alphanumeric() {
            out.push(c);
        } else {
            break;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::snake_case;

    #[test]
    fn alert_names_become_label_values() {
        assert_eq!(snake_case("HandshakeFailure"), "handshake_failure");
        assert_eq!(snake_case("BadCertificate"), "bad_certificate");
        assert_eq!(snake_case("UnknownCA"), "unknown_ca");
        assert_eq!(snake_case("Unknown(200)"), "unknown");
    }
}
