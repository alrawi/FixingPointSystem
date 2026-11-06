// SPDX-License-Identifier: GPL-3.0-only
//! Client and server profiles, which fix what each side puts on the wire.

use std::str::FromStr;
use std::sync::Arc;

use rustls::crypto::ring::{self, cipher_suite as cs, kx_group};
use rustls::crypto::{CryptoProvider, SupportedKxGroup};
use rustls::{KeyLog, SupportedCipherSuite, SupportedProtocolVersion};

use crate::error::TlsError;

/// A TLS protocol version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TlsVersion {
    /// TLS 1.2, RFC 5246.
    Tls12,
    /// TLS 1.3, RFC 8446.
    Tls13,
}

impl TlsVersion {
    pub(crate) fn rustls(self) -> &'static SupportedProtocolVersion {
        match self {
            TlsVersion::Tls12 => &rustls::version::TLS12,
            TlsVersion::Tls13 => &rustls::version::TLS13,
        }
    }

    /// Returns the name used in reports, `1.2` or `1.3`.
    pub fn as_str(self) -> &'static str {
        match self {
            TlsVersion::Tls12 => "1.2",
            TlsVersion::Tls13 => "1.3",
        }
    }
}

impl FromStr for TlsVersion {
    type Err = TlsError;

    fn from_str(s: &str) -> Result<Self, TlsError> {
        match s {
            "1.2" | "tls1.2" | "TLSv1.2" => Ok(TlsVersion::Tls12),
            "1.3" | "tls1.3" | "TLSv1.3" => Ok(TlsVersion::Tls13),
            other => Err(TlsError::Config(format!(
                "unknown TLS version `{other}`, expected 1.2 or 1.3"
            ))),
        }
    }
}

/// A key exchange group.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Group {
    /// X25519, group 29.
    X25519,
    /// NIST P-256, group 23.
    Secp256r1,
    /// NIST P-384, group 24.
    Secp384r1,
}

impl Group {
    /// The groups of the ring provider, in its preference order.
    pub const ALL: [Group; 3] = [Group::X25519, Group::Secp256r1, Group::Secp384r1];

    fn kx(self) -> &'static dyn SupportedKxGroup {
        match self {
            Group::X25519 => kx_group::X25519,
            Group::Secp256r1 => kx_group::SECP256R1,
            Group::Secp384r1 => kx_group::SECP384R1,
        }
    }

    /// Returns the IANA name, for example `secp256r1`.
    pub fn as_str(self) -> &'static str {
        match self {
            Group::X25519 => "x25519",
            Group::Secp256r1 => "secp256r1",
            Group::Secp384r1 => "secp384r1",
        }
    }

    /// Finds the group of a negotiated key exchange by its name in a handshake.
    pub(crate) fn name_of(group: &dyn SupportedKxGroup) -> String {
        format!("{:?}", group.name()).to_ascii_lowercase()
    }
}

impl FromStr for Group {
    type Err = TlsError;

    fn from_str(s: &str) -> Result<Self, TlsError> {
        Group::ALL
            .into_iter()
            .find(|g| g.as_str().eq_ignore_ascii_case(s))
            .ok_or_else(|| TlsError::Config(format!("unknown key exchange group `{s}`")))
    }
}

/// A cipher suite of the ring provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CipherSuite {
    /// `TLS_AES_128_GCM_SHA256`
    Tls13Aes128GcmSha256,
    /// `TLS_AES_256_GCM_SHA384`
    Tls13Aes256GcmSha384,
    /// `TLS_CHACHA20_POLY1305_SHA256`
    Tls13Chacha20Poly1305Sha256,
    /// `TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256`
    Tls12EcdheEcdsaAes128GcmSha256,
    /// `TLS_ECDHE_ECDSA_WITH_AES_256_GCM_SHA384`
    Tls12EcdheEcdsaAes256GcmSha384,
    /// `TLS_ECDHE_ECDSA_WITH_CHACHA20_POLY1305_SHA256`
    Tls12EcdheEcdsaChacha20Poly1305Sha256,
    /// `TLS_ECDHE_RSA_WITH_AES_128_GCM_SHA256`
    Tls12EcdheRsaAes128GcmSha256,
    /// `TLS_ECDHE_RSA_WITH_AES_256_GCM_SHA384`
    Tls12EcdheRsaAes256GcmSha384,
    /// `TLS_ECDHE_RSA_WITH_CHACHA20_POLY1305_SHA256`
    Tls12EcdheRsaChacha20Poly1305Sha256,
}

impl CipherSuite {
    /// Every suite of the ring provider, in its preference order.
    pub const ALL: [CipherSuite; 9] = [
        CipherSuite::Tls13Aes256GcmSha384,
        CipherSuite::Tls13Aes128GcmSha256,
        CipherSuite::Tls13Chacha20Poly1305Sha256,
        CipherSuite::Tls12EcdheEcdsaAes256GcmSha384,
        CipherSuite::Tls12EcdheEcdsaAes128GcmSha256,
        CipherSuite::Tls12EcdheEcdsaChacha20Poly1305Sha256,
        CipherSuite::Tls12EcdheRsaAes256GcmSha384,
        CipherSuite::Tls12EcdheRsaAes128GcmSha256,
        CipherSuite::Tls12EcdheRsaChacha20Poly1305Sha256,
    ];

    pub(crate) fn supported(self) -> SupportedCipherSuite {
        match self {
            CipherSuite::Tls13Aes128GcmSha256 => cs::TLS13_AES_128_GCM_SHA256,
            CipherSuite::Tls13Aes256GcmSha384 => cs::TLS13_AES_256_GCM_SHA384,
            CipherSuite::Tls13Chacha20Poly1305Sha256 => cs::TLS13_CHACHA20_POLY1305_SHA256,
            CipherSuite::Tls12EcdheEcdsaAes128GcmSha256 => {
                cs::TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256
            }
            CipherSuite::Tls12EcdheEcdsaAes256GcmSha384 => {
                cs::TLS_ECDHE_ECDSA_WITH_AES_256_GCM_SHA384
            }
            CipherSuite::Tls12EcdheEcdsaChacha20Poly1305Sha256 => {
                cs::TLS_ECDHE_ECDSA_WITH_CHACHA20_POLY1305_SHA256
            }
            CipherSuite::Tls12EcdheRsaAes128GcmSha256 => cs::TLS_ECDHE_RSA_WITH_AES_128_GCM_SHA256,
            CipherSuite::Tls12EcdheRsaAes256GcmSha384 => cs::TLS_ECDHE_RSA_WITH_AES_256_GCM_SHA384,
            CipherSuite::Tls12EcdheRsaChacha20Poly1305Sha256 => {
                cs::TLS_ECDHE_RSA_WITH_CHACHA20_POLY1305_SHA256
            }
        }
    }

    /// Returns the IANA name of the suite.
    pub fn as_str(self) -> &'static str {
        match self {
            CipherSuite::Tls13Aes128GcmSha256 => "TLS_AES_128_GCM_SHA256",
            CipherSuite::Tls13Aes256GcmSha384 => "TLS_AES_256_GCM_SHA384",
            CipherSuite::Tls13Chacha20Poly1305Sha256 => "TLS_CHACHA20_POLY1305_SHA256",
            CipherSuite::Tls12EcdheEcdsaAes128GcmSha256 => {
                "TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256"
            }
            CipherSuite::Tls12EcdheEcdsaAes256GcmSha384 => {
                "TLS_ECDHE_ECDSA_WITH_AES_256_GCM_SHA384"
            }
            CipherSuite::Tls12EcdheEcdsaChacha20Poly1305Sha256 => {
                "TLS_ECDHE_ECDSA_WITH_CHACHA20_POLY1305_SHA256"
            }
            CipherSuite::Tls12EcdheRsaAes128GcmSha256 => "TLS_ECDHE_RSA_WITH_AES_128_GCM_SHA256",
            CipherSuite::Tls12EcdheRsaAes256GcmSha384 => "TLS_ECDHE_RSA_WITH_AES_256_GCM_SHA384",
            CipherSuite::Tls12EcdheRsaChacha20Poly1305Sha256 => {
                "TLS_ECDHE_RSA_WITH_CHACHA20_POLY1305_SHA256"
            }
        }
    }
}

impl FromStr for CipherSuite {
    type Err = TlsError;

    fn from_str(s: &str) -> Result<Self, TlsError> {
        CipherSuite::ALL
            .into_iter()
            .find(|c| c.as_str().eq_ignore_ascii_case(s))
            .ok_or_else(|| TlsError::Config(format!("unknown or unsupported cipher suite `{s}`")))
    }
}

/// How a client keeps sessions for resumption.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientResumption {
    /// Every handshake is a full handshake.
    Disabled,
    /// The connector keeps up to `sessions` server names with their tickets or session ids.
    InMemory {
        /// Number of server names that the store keeps.
        sessions: usize,
    },
}

/// How a server enables resumption.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServerResumption {
    /// The server issues no tickets and keeps no sessions.
    Disabled,
    /// The server issues stateless tickets, which resume on TLS 1.2 and 1.3.
    Tickets,
    /// The server keeps up to `sessions` sessions in memory and issues ids and TLS 1.3 tickets for them.
    SessionCache {
        /// Capacity of the cache.
        sessions: usize,
    },
}

/// What a client puts on the wire.
///
/// The ClientHello lists the cipher suites in the order of `cipher_suites`
/// (suites of a version that is not enabled drop out) and the groups in the
/// order of `groups`. The first group also supplies the key share, so the
/// handshake needs no retry when the server accepts it. An empty list selects
/// the default of the ring provider.
#[derive(Debug, Clone)]
pub struct ClientProfile {
    /// Name of the profile, for reports and logs.
    pub name: String,
    /// Versions to offer, in any order. The default is TLS 1.3 and 1.2.
    pub versions: Vec<TlsVersion>,
    /// Whether the ClientHello carries the server name indication. Rustls never
    /// sends it for an IP address, whatever this flag says.
    pub send_sni: bool,
    /// ALPN protocol ids in preference order, such as `h2` and `http/1.1`.
    pub alpn: Vec<Vec<u8>>,
    /// Key exchange groups in preference order.
    pub groups: Vec<Group>,
    /// Cipher suites in preference order.
    pub cipher_suites: Vec<CipherSuite>,
    /// Session resumption of the connector.
    pub resumption: ClientResumption,
    /// Receives the secrets of every handshake, see [`crate::NssKeyLog`].
    pub key_log: Option<Arc<dyn KeyLog>>,
}

impl Default for ClientProfile {
    fn default() -> Self {
        Self {
            name: "default".into(),
            versions: vec![TlsVersion::Tls13, TlsVersion::Tls12],
            send_sni: true,
            alpn: Vec::new(),
            groups: Vec::new(),
            cipher_suites: Vec::new(),
            resumption: ClientResumption::InMemory { sessions: 256 },
            key_log: None,
        }
    }
}

/// What a server offers and accepts.
#[derive(Debug, Clone)]
pub struct ServerProfile {
    /// Name of the profile, for reports and logs.
    pub name: String,
    /// Versions to accept. The default is TLS 1.3 and 1.2.
    pub versions: Vec<TlsVersion>,
    /// ALPN protocol ids in preference order. A client that offers ids and
    /// shares none with a non empty list fails with `no_application_protocol`.
    pub alpn: Vec<Vec<u8>>,
    /// Key exchange groups the server accepts, in preference order.
    pub groups: Vec<Group>,
    /// Cipher suites the server accepts, in preference order.
    pub cipher_suites: Vec<CipherSuite>,
    /// Whether the server picks the suite by its own order and not the client's.
    pub use_server_cipher_order: bool,
    /// Ticket and session policy.
    pub resumption: ServerResumption,
    /// Receives the secrets of every handshake, see [`crate::NssKeyLog`].
    pub key_log: Option<Arc<dyn KeyLog>>,
}

impl Default for ServerProfile {
    fn default() -> Self {
        Self {
            name: "default".into(),
            versions: vec![TlsVersion::Tls13, TlsVersion::Tls12],
            alpn: Vec::new(),
            groups: Vec::new(),
            cipher_suites: Vec::new(),
            use_server_cipher_order: false,
            resumption: ServerResumption::Tickets,
            key_log: None,
        }
    }
}

/// Builds the crypto provider with the suite and group order of a profile.
pub(crate) fn provider(groups: &[Group], suites: &[CipherSuite]) -> Arc<CryptoProvider> {
    let mut p = ring::default_provider();
    if !suites.is_empty() {
        p.cipher_suites = suites.iter().map(|s| s.supported()).collect();
    }
    if !groups.is_empty() {
        p.kx_groups = groups.iter().map(|g| g.kx()).collect();
    }
    Arc::new(p)
}

/// Returns the rustls versions of a profile, or an error when the list is empty.
pub(crate) fn versions(
    list: &[TlsVersion],
) -> Result<Vec<&'static SupportedProtocolVersion>, TlsError> {
    if list.is_empty() {
        return Err(TlsError::Config(
            "a profile needs at least one TLS version".into(),
        ));
    }
    Ok(list.iter().map(|v| v.rustls()).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_round_trip_through_from_str() {
        for c in CipherSuite::ALL {
            assert_eq!(c.as_str().parse::<CipherSuite>().unwrap(), c);
            assert_eq!(
                format!("{:?}", c.supported().suite()),
                c.as_str()
                    .replace("TLS_AES", "TLS13_AES")
                    .replace("TLS_CHACHA", "TLS13_CHACHA")
            );
        }
        for g in Group::ALL {
            assert_eq!(g.as_str().parse::<Group>().unwrap(), g);
            assert_eq!(Group::name_of(g.kx()), g.as_str());
        }
        assert_eq!("1.3".parse::<TlsVersion>().unwrap(), TlsVersion::Tls13);
        assert!("1.1".parse::<TlsVersion>().is_err());
        assert!("TLS_RC4_128_MD5".parse::<CipherSuite>().is_err());
    }

    #[test]
    fn all_lists_follow_the_provider_order() {
        let provider = ring::default_provider();
        let ours: Vec<_> = CipherSuite::ALL
            .iter()
            .map(|c| c.supported().suite())
            .collect();
        let theirs: Vec<_> = provider.cipher_suites.iter().map(|c| c.suite()).collect();
        assert_eq!(ours, theirs);
    }
}
