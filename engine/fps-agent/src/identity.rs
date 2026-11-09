// SPDX-License-Identifier: GPL-3.0-only
//! The mutual TLS identity of the agent (AGT-08).
//!
//! Since CTL-27 the controller serves `ControllerService` over mutual TLS,
//! and a second HTTPS listener serves `POST /enroll` and `POST /renew` of the
//! `internal/pki` package. This module is the agent side of both.
//!
//! - [`TlsConfig`] holds the flags: the trust anchor (a CA file or a pinned
//!   SHA-256 fingerprint), the source of the enrollment token, the enrollment
//!   address, and the directory with the key and the certificate.
//! - [`Identity::client_tls`] returns the tonic TLS configuration for one
//!   dial. When the directory holds no valid certificate, or a token file
//!   exists, it enrolls first:
//!   it makes an ECDSA P-256 key, sends a CSR with the common name set to the
//!   agent id and the token to `POST /enroll`, writes the files with mode
//!   0600, and forgets the token.
//! - [`Identity::run_renewals`] calls `POST /renew` at `renew_at`. A renewal
//!   uses a separate HTTPS connection, so it never touches the open session,
//!   and the new certificate serves the next dial.
//!
//! The directory holds `agent.key` (0600), `agent.crt`, `identity.json` (the
//! validity of the certificate), and, after an enrollment that pinned a
//! fingerprint, `ca.pem`. Every file is replaced atomically.
//!
//! The CA is the trust anchor of both connections. With a pinned fingerprint
//! and no CA file the agent has no CA certificate before its first
//! enrollment. The controller presents the leaf and the CA on the enrollment
//! listener (CTL-31), so a custom verifier ([`PinnedVerifier`]) finds the
//! certificate with the pinned SHA-256 in the presented chain during the TLS
//! handshake and requires that the leaf chains to it and names the address.
//! The agent sends the token only after that check, so an endpoint of
//! another CA never sees it, and it stores the `ca_pem` of the answer only
//! when that file holds the pinned certificate.

use std::io::Write as _;
use std::os::unix::fs::{DirBuilderExt as _, OpenOptionsExt as _, PermissionsExt as _};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use bytes::Bytes;
use http_body_util::{BodyExt as _, Full, Limited};
use hyper::header::{CONNECTION, CONTENT_TYPE, HOST};
use hyper::Request;
use hyper_util::rt::TokioIo;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::client::WebPkiServerVerifier;
use rustls::crypto::CryptoProvider;
use rustls::{ClientConfig, DigitallySignedStruct, RootCertStore, SignatureScheme};
use rustls_pki_types::pem::PemObject as _;
use rustls_pki_types::{CertificateDer, PrivateKeyDer, ServerName, UnixTime};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;
use tonic::transport::{Certificate, ClientTlsConfig, Identity as TonicIdentity};

/// The file of the private key, mode 0600.
pub const KEY_FILE: &str = "agent.key";
/// The file of the agent certificate.
pub const CERT_FILE: &str = "agent.crt";
/// The file of the CA certificate that an enrollment with a pinned fingerprint
/// stores.
pub const CA_FILE: &str = "ca.pem";
/// The file with the validity of the certificate.
pub const META_FILE: &str = "identity.json";
/// The environment variable that holds the enrollment token when no flag
/// names a source.
pub const TOKEN_ENV: &str = "FPS_ENROLL_TOKEN";
/// The port of the enrollment listener of the controller by default.
pub const DEFAULT_ENROLL_PORT: u16 = 8445;

/// How long a TCP connection to the enrollment endpoint may take.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// How long one enrollment or renewal request may take in all.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
/// The largest answer that the agent reads from the enrollment endpoint.
const MAX_ANSWER: usize = 64 << 10;
/// The first wait after a failed renewal.
const RENEW_RETRY_MIN: Duration = Duration::from_secs(5);
/// The longest wait after a failed renewal.
const RENEW_RETRY_MAX: Duration = Duration::from_secs(60);
/// The longest sleep of the renewal task, so it reads the files again.
const RENEW_POLL_MAX: Duration = Duration::from_secs(3600);

/// Where the enrollment token comes from. It is never a command line value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenSource {
    /// A file that holds the token. The agent deletes it after the exchange.
    File(PathBuf),
    /// An environment variable. The agent removes it after the exchange.
    Env(String),
}

/// The TLS settings of `fps-agent connect` for an `https://` controller.
#[derive(Debug, Clone)]
pub struct TlsConfig {
    /// A PEM file with the CA certificate of the controller.
    pub ca_file: Option<PathBuf>,
    /// The SHA-256 fingerprint of the CA certificate, which the controller
    /// logs as `ca_sha256`.
    pub ca_sha256: Option<[u8; 32]>,
    /// The enrollment token source. `None` means the environment variable
    /// [`TOKEN_ENV`].
    pub token: Option<TokenSource>,
    /// The `host:port` of the HTTPS enrollment listener.
    pub enroll_address: String,
    /// The directory of the key, the certificate, and the CA file.
    pub dir: PathBuf,
}

/// Parses a SHA-256 fingerprint in hex. It accepts upper case, colons between
/// the bytes, and a `sha256:` prefix.
///
/// # Errors
///
/// Fails when the text is not 64 hex digits.
pub fn parse_fingerprint(text: &str) -> Result<[u8; 32], String> {
    let t = text.trim();
    let t = t.strip_prefix("sha256:").unwrap_or(t);
    let digits: Vec<u8> = t.bytes().filter(|b| *b != b':').collect();
    if digits.len() != 64 {
        return Err(format!(
            "{text:?} is not a SHA-256 fingerprint of 64 hex digits"
        ));
    }
    let mut out = [0u8; 32];
    for (i, pair) in digits.chunks(2).enumerate() {
        let hi = hex_value(pair[0]).ok_or_else(|| format!("{text:?} holds a non hex digit"))?;
        let lo = hex_value(pair[1]).ok_or_else(|| format!("{text:?} holds a non hex digit"))?;
        out[i] = hi << 4 | lo;
    }
    Ok(out)
}

fn hex_value(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(String::new(), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

/// Errors of enrollment, renewal, and the TLS setup.
#[derive(Debug, thiserror::Error)]
pub enum IdentityError {
    /// A file could not be read or written.
    #[error("{action} {path}")]
    Io {
        /// What the agent did, for example `read`.
        action: &'static str,
        /// The file.
        path: PathBuf,
        /// The cause.
        #[source]
        source: std::io::Error,
    },
    /// A file holds no valid PEM of the expected kind.
    #[error("{path} holds no valid {what}")]
    Pem {
        /// The file.
        path: PathBuf,
        /// The expected kind, for example `CA certificate`.
        what: &'static str,
    },
    /// The agent has nothing to trust the controller with.
    #[error(
        "no CA to trust: pass --tls-ca FILE with the CA certificate of the controller, or \
         --tls-ca-sha256 with the ca_sha256 that the controller logs"
    )]
    NoTrust,
    /// The CA certificate does not have the pinned fingerprint.
    #[error("the CA certificate does not have the pinned fingerprint: want sha256:{want}, found {found}")]
    PinMismatch {
        /// The pinned fingerprint in hex.
        want: String,
        /// The fingerprints that the agent found, in hex, or `none`.
        found: String,
    },
    /// The key file is readable by other users.
    #[error("{path} has mode {mode:o}, but a private key must be readable by its owner only")]
    KeyMode {
        /// The key file.
        path: PathBuf,
        /// Its permission bits.
        mode: u32,
    },
    /// The agent needs a token and has none.
    #[error("{reason}, and no enrollment token is available: mint one with `fps-controller token -agent-id <id>` and pass it through --enroll-token-file or the environment variable {env}")]
    NoToken {
        /// Why the agent needs a new certificate.
        reason: String,
        /// The environment variable of the default token source.
        env: String,
    },
    /// The token source is configured but gives no token.
    #[error("the enrollment token from {source_name} is missing or empty")]
    EmptyToken {
        /// The file or variable.
        source_name: String,
    },
    /// The TLS handshake with the enrollment endpoint failed.
    #[error("TLS failure toward the enrollment endpoint {address}: {detail}")]
    Handshake {
        /// The `host:port`.
        address: String,
        /// The TLS error.
        detail: String,
    },
    /// The controller refused the token or the certificate.
    #[error(
        "the controller refused the {what} (HTTP 401): it is unknown, used, expired, or revoked"
    )]
    Refused {
        /// `enrollment token` or `certificate`.
        what: &'static str,
    },
    /// The controller rejected the request as malformed.
    #[error("the enrollment endpoint rejected the request with HTTP {status}: {body}")]
    Rejected {
        /// The status code.
        status: u16,
        /// The start of the body.
        body: String,
    },
    /// The enrollment endpoint could not be reached or did not answer.
    #[error("reach the enrollment endpoint {address}: {detail}")]
    Network {
        /// The `host:port`.
        address: String,
        /// The cause.
        detail: String,
    },
    /// The answer of the endpoint is not what the contract of `pki` says.
    #[error("the enrollment answer is invalid: {0}")]
    Answer(String),
    /// The key or the CSR could not be made.
    #[error("make the key or the CSR: {0}")]
    Keygen(String),
}

impl IdentityError {
    /// Returns true when redialing cannot fix the error, so the agent exits.
    pub fn is_permanent(&self) -> bool {
        !matches!(
            self,
            IdentityError::Network { .. } | IdentityError::Answer(_) | IdentityError::Io { .. }
        )
    }
}

fn io_err<'a>(
    action: &'static str,
    path: &'a Path,
) -> impl FnOnce(std::io::Error) -> IdentityError + 'a {
    move |source| IdentityError::Io {
        action,
        path: path.to_owned(),
        source,
    }
}

/// Looks for a rustls error in the chain of `e`, which is how the agent
/// recognizes a failed handshake, a certificate that the peer refuses, and a
/// peer that does not speak TLS. It also looks inside `std::io::Error`, whose
/// `source` skips the error that it wraps.
pub fn tls_failure(e: &(dyn std::error::Error + 'static)) -> Option<String> {
    let mut cur = Some(e);
    while let Some(err) = cur {
        if let Some(r) = err.downcast_ref::<rustls::Error>() {
            return Some(r.to_string());
        }
        if let Some(io) = err.downcast_ref::<std::io::Error>() {
            if let Some(found) = io.get_ref().and_then(|i| tls_failure(i)) {
                return Some(found);
            }
        }
        cur = err.source();
    }
    // h2 keeps the kind and the text of an io error and drops its type, so
    // an alert that arrives on an open HTTP/2 connection reads as text.
    let text = chain(e);
    text.find("received fatal alert")
        .map(|at| text[at..].to_owned())
}

/// Returns true when the first call on a fresh connection failed because the
/// connection closed, which is a broken pipe, a reset, an end of file, or an
/// h2 "connection closed". The caller knows that the TLS handshake had
/// completed and that no response arrived, because it is the first call of a
/// connection that the dial opened. A refused or reached endpoint, a timeout,
/// and a status that the controller sent are not of this kind.
pub fn closed_before_response(status: &tonic::Status) -> bool {
    if !matches!(
        status.code(),
        tonic::Code::Unavailable | tonic::Code::Unknown
    ) {
        return false;
    }
    let mut text = status.message().to_ascii_lowercase();
    let mut cur = std::error::Error::source(status);
    while let Some(c) = cur {
        text.push_str(": ");
        text.push_str(&c.to_string().to_ascii_lowercase());
        cur = c.source();
    }
    [
        "connection closed",
        "broken pipe",
        "connection reset",
        "unexpected eof",
        "unexpected end of file",
    ]
    .iter()
    .any(|needle| text.contains(needle))
}

/// What the agent trusts.
#[derive(Debug, Clone)]
enum Trust {
    /// CA certificates, as PEM and as DER.
    Roots(Vec<u8>, Vec<CertificateDer<'static>>),
    /// Only a fingerprint, until the first enrollment gives the CA.
    Pin([u8; 32]),
}

/// The files of the identity.
#[derive(Debug, Clone)]
struct Stored {
    cert_pem: Vec<u8>,
    key_pem: Vec<u8>,
    not_after: SystemTime,
    renew_at: SystemTime,
}

#[derive(Debug, Serialize, Deserialize)]
struct Meta {
    not_after: i64,
    renew_at: i64,
}

#[derive(Debug, Serialize)]
struct EnrollBody<'a> {
    token: &'a str,
    csr_pem: &'a str,
}

#[derive(Debug, Serialize)]
struct RenewBody<'a> {
    csr_pem: &'a str,
}

/// The answer of `POST /enroll` and `POST /renew`.
#[derive(Debug, Deserialize)]
struct Issued {
    agent_id: String,
    certificate_pem: String,
    #[serde(default)]
    ca_pem: String,
    not_after: String,
    renew_at: String,
}

/// The identity of one agent. Clone the `Arc` to share it.
#[derive(Debug)]
pub struct Identity {
    cfg: TlsConfig,
    agent_id: String,
    /// Wakes the renewal task when an enrollment stored a certificate.
    stored: tokio::sync::Notify,
}

impl Identity {
    /// Returns the identity of `agent_id` for the settings `cfg`.
    pub fn new(cfg: TlsConfig, agent_id: String) -> Self {
        Self {
            cfg,
            agent_id,
            stored: tokio::sync::Notify::new(),
        }
    }

    /// Returns the TLS configuration for one dial of the controller. When
    /// the directory holds no certificate, or the certificate expired, it
    /// enrolls first.
    ///
    /// # Errors
    ///
    /// Fails when the trust anchor is missing, when no token is available for
    /// a needed enrollment, or when the enrollment fails.
    pub async fn client_tls(&self) -> Result<ClientTlsConfig, IdentityError> {
        let mut trust = self.trust()?;
        // A token file that exists means that the operator wants a new
        // enrollment, for example after a revocation. The agent deletes the
        // file after every enrollment, so a leftover file is always new.
        let new_token = matches!(&self.cfg.token, Some(TokenSource::File(p)) if p.exists());
        let stored = match load(&self.cfg.dir)? {
            Some(s) if SystemTime::now() < s.not_after && !new_token => s,
            other => {
                let reason = match &other {
                    Some(s) if SystemTime::now() >= s.not_after => {
                        "the certificate of the agent expired".to_owned()
                    }
                    Some(_) => "a new enrollment token is present".to_owned(),
                    None => format!("the agent has no certificate in {}", self.cfg.dir.display()),
                };
                tracing::info!(reason = %reason, "enrolling");
                let s = self.enroll(&trust, &reason).await?;
                // An enrollment with a pinned fingerprint stored the CA.
                trust = self.trust()?;
                s
            }
        };
        let Trust::Roots(ca_pem, _) = trust else {
            return Err(IdentityError::NoTrust);
        };
        Ok(ClientTlsConfig::new()
            .ca_certificate(Certificate::from_pem(ca_pem))
            .identity(TonicIdentity::from_pem(stored.cert_pem, stored.key_pem)))
    }

    /// Renews the certificate at `renew_at`, again and again. The task never
    /// returns, so the caller aborts it. A renewal does not touch the open
    /// session, which keeps the certificate that it was opened with, and the
    /// new certificate serves the next dial. The agent therefore never
    /// redials just to renew, and a renewal during a run is harmless.
    pub async fn run_renewals(&self) {
        let mut retry = RENEW_RETRY_MIN;
        loop {
            let wait = match load(&self.cfg.dir) {
                Ok(Some(s)) => {
                    let now = SystemTime::now();
                    if now >= s.not_after {
                        // Too late to renew. The next dial enrolls again.
                        RENEW_POLL_MAX.min(Duration::from_secs(30))
                    } else {
                        s.renew_at.duration_since(now).unwrap_or(Duration::ZERO)
                    }
                }
                // No certificate yet. The first dial enrolls.
                Ok(None) => Duration::from_secs(30),
                Err(e) => {
                    tracing::warn!(error = %e, "read the identity for renewal");
                    Duration::from_secs(30)
                }
            };
            if !wait.is_zero() {
                tokio::select! {
                    () = tokio::time::sleep(wait.min(RENEW_POLL_MAX)) => {}
                    () = self.stored.notified() => {}
                }
                continue;
            }
            match self.renew().await {
                Ok(not_after) => {
                    retry = RENEW_RETRY_MIN;
                    let secs = not_after
                        .duration_since(UNIX_EPOCH)
                        .map_or(0, |d| d.as_secs());
                    tracing::info!(not_after_unix = secs, "certificate renewed");
                }
                Err(e) => {
                    let level_error = e.is_permanent();
                    if level_error {
                        tracing::error!(error = %chain(&e), "certificate renewal refused, the next dial enrolls again if a token exists");
                    } else {
                        tracing::warn!(error = %chain(&e), retry_ms = retry.as_millis() as u64, "certificate renewal failed");
                    }
                    tokio::time::sleep(if level_error { RENEW_RETRY_MAX } else { retry }).await;
                    retry = (retry * 2).min(RENEW_RETRY_MAX);
                }
            }
        }
    }

    /// Resolves the trust anchor from the CA file, the stored CA, or the
    /// pinned fingerprint, in this order.
    fn trust(&self) -> Result<Trust, IdentityError> {
        let pin = self.cfg.ca_sha256;
        let source = match &self.cfg.ca_file {
            Some(p) => Some(p.clone()),
            None => {
                let stored = self.cfg.dir.join(CA_FILE);
                stored.exists().then_some(stored)
            }
        };
        let Some(path) = source else {
            return pin.map(Trust::Pin).ok_or(IdentityError::NoTrust);
        };
        let pem = std::fs::read(&path).map_err(io_err("read", &path))?;
        let ders: Vec<CertificateDer<'static>> = CertificateDer::pem_slice_iter(&pem)
            .collect::<Result<_, _>>()
            .map_err(|_| IdentityError::Pem {
                path: path.clone(),
                what: "CA certificate",
            })?;
        if ders.is_empty() {
            return Err(IdentityError::Pem {
                path,
                what: "CA certificate",
            });
        }
        if let Some(want) = pin {
            if !ders.iter().any(|d| fingerprint(d) == want) {
                let found: Vec<String> = ders.iter().map(|d| hex(&fingerprint(d))).collect();
                return Err(IdentityError::PinMismatch {
                    want: hex(&want),
                    found: found.join(","),
                });
            }
        }
        Ok(Trust::Roots(pem, ders))
    }

    fn token(&self) -> Result<Option<(String, TokenSource)>, IdentityError> {
        let source = self
            .cfg
            .token
            .clone()
            .unwrap_or_else(|| TokenSource::Env(TOKEN_ENV.to_owned()));
        let text = match &source {
            TokenSource::File(p) => match std::fs::read_to_string(p) {
                Ok(t) => t,
                // The file is gone: an earlier enrollment used and deleted
                // it, or the operator has not written it yet.
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
                Err(e) => return Err(io_err("read", p)(e)),
            },
            TokenSource::Env(name) => match std::env::var(name) {
                Ok(t) => t,
                Err(_) => return Ok(None),
            },
        };
        let text = text.trim().to_owned();
        if text.is_empty() {
            return Err(IdentityError::EmptyToken {
                source_name: describe(&source),
            });
        }
        Ok(Some((text, source)))
    }

    async fn enroll(&self, trust: &Trust, reason: &str) -> Result<Stored, IdentityError> {
        let Some((token, source)) = self.token()? else {
            return Err(IdentityError::NoToken {
                reason: reason.to_owned(),
                env: TOKEN_ENV.to_owned(),
            });
        };
        let (key, csr) = new_key_and_csr(&self.agent_id)?;
        let body = serde_json::to_string(&EnrollBody {
            token: &token,
            csr_pem: &csr,
        })
        .map_err(|e| IdentityError::Answer(e.to_string()))?;
        let config = client_config(trust, None)?;
        let answer = post(&self.cfg.enroll_address, config, "/enroll", body).await?;
        let issued = self.accept(answer, "enrollment token")?;
        if let Trust::Pin(pin) = trust {
            check_pinned_ca(pin, &issued.ca_pem)?;
        }
        let stored = self.store(&issued, Some(&key.serialize_pem()))?;
        if matches!(trust, Trust::Pin(_)) {
            write_atomic(&self.cfg.dir.join(CA_FILE), issued.ca_pem.as_bytes(), 0o644)?;
        }
        forget(&source);
        self.stored.notify_one();
        Ok(stored)
    }

    async fn renew(&self) -> Result<SystemTime, IdentityError> {
        let trust = self.trust()?;
        let Some(stored) = load(&self.cfg.dir)? else {
            return Err(IdentityError::Answer("no certificate to renew".to_owned()));
        };
        // The same key signs the new request. The files change one at a
        // time, so a new key would leave a window with a mismatched pair.
        let key = rcgen::KeyPair::from_pem(&String::from_utf8_lossy(&stored.key_pem))
            .map_err(|e| IdentityError::Keygen(e.to_string()))?;
        let csr = csr_for(&self.agent_id, &key)?;
        let body = serde_json::to_string(&RenewBody { csr_pem: &csr })
            .map_err(|e| IdentityError::Answer(e.to_string()))?;
        let chain = parse_certs(&stored.cert_pem, &self.cfg.dir.join(CERT_FILE))?;
        let key_der = parse_key(&stored.key_pem, &self.cfg.dir.join(KEY_FILE))?;
        let config = client_config(&trust, Some((chain, key_der)))?;
        let answer = post(&self.cfg.enroll_address, config, "/renew", body).await?;
        let issued = self.accept(answer, "certificate")?;
        let renewed = self.store(&issued, None)?;
        Ok(renewed.not_after)
    }

    /// Turns the HTTP answer into the issued certificate.
    fn accept(&self, answer: Answer, what: &'static str) -> Result<Issued, IdentityError> {
        match answer.status {
            200 => {}
            401 => return Err(IdentityError::Refused { what }),
            status => {
                let mut body = String::from_utf8_lossy(&answer.body).into_owned();
                body.truncate(200);
                return Err(IdentityError::Rejected { status, body });
            }
        }
        let issued: Issued = serde_json::from_slice(&answer.body)
            .map_err(|e| IdentityError::Answer(format!("not the documented JSON: {e}")))?;
        if issued.agent_id != self.agent_id {
            return Err(IdentityError::Answer(format!(
                "the certificate is for {:?}, not for this agent {:?}",
                issued.agent_id, self.agent_id
            )));
        }
        Ok(issued)
    }

    /// Writes the certificate, and the key when there is a new one.
    fn store(&self, issued: &Issued, key_pem: Option<&str>) -> Result<Stored, IdentityError> {
        let dir = &self.cfg.dir;
        let not_after = parse_time(&issued.not_after)?;
        let renew_at = parse_time(&issued.renew_at)?;
        let cert_path = dir.join(CERT_FILE);
        parse_certs(issued.certificate_pem.as_bytes(), &cert_path)?;
        create_dir(dir)?;
        let key_pem = match key_pem {
            Some(k) => {
                write_atomic(&dir.join(KEY_FILE), k.as_bytes(), 0o600)?;
                k.as_bytes().to_vec()
            }
            None => {
                std::fs::read(dir.join(KEY_FILE)).map_err(io_err("read", &dir.join(KEY_FILE)))?
            }
        };
        let meta = Meta {
            not_after: unix(not_after),
            renew_at: unix(renew_at),
        };
        let meta_json =
            serde_json::to_vec(&meta).map_err(|e| IdentityError::Answer(e.to_string()))?;
        write_atomic(&dir.join(META_FILE), &meta_json, 0o644)?;
        write_atomic(&cert_path, issued.certificate_pem.as_bytes(), 0o644)?;
        Ok(Stored {
            cert_pem: issued.certificate_pem.clone().into_bytes(),
            key_pem,
            not_after,
            renew_at,
        })
    }
}

fn describe(s: &TokenSource) -> String {
    match s {
        TokenSource::File(p) => p.display().to_string(),
        TokenSource::Env(n) => format!("the environment variable {n}"),
    }
}

/// Deletes or unsets the token, so the agent does not keep a spent secret.
fn forget(source: &TokenSource) {
    match source {
        TokenSource::File(p) => {
            if let Err(e) = std::fs::remove_file(p) {
                if e.kind() != std::io::ErrorKind::NotFound {
                    tracing::warn!(path = %p.display(), error = %e, "delete the spent enrollment token file");
                }
            }
        }
        TokenSource::Env(n) => std::env::remove_var(n),
    }
}

fn chain(e: &dyn std::error::Error) -> String {
    let mut s = e.to_string();
    let mut cur = e.source();
    while let Some(c) = cur {
        s.push_str(": ");
        s.push_str(&c.to_string());
        cur = c.source();
    }
    s
}

fn fingerprint(der: &CertificateDer<'_>) -> [u8; 32] {
    Sha256::digest(der.as_ref()).into()
}

fn unix(t: SystemTime) -> i64 {
    match t.duration_since(UNIX_EPOCH) {
        Ok(d) => d.as_secs() as i64,
        Err(e) => -(e.duration().as_secs() as i64),
    }
}

fn from_unix(secs: i64) -> SystemTime {
    if secs >= 0 {
        UNIX_EPOCH + Duration::from_secs(secs as u64)
    } else {
        UNIX_EPOCH - Duration::from_secs(secs.unsigned_abs())
    }
}

fn parse_time(text: &str) -> Result<SystemTime, IdentityError> {
    let t = time::OffsetDateTime::parse(text, &time::format_description::well_known::Rfc3339)
        .map_err(|e| IdentityError::Answer(format!("time {text:?}: {e}")))?;
    Ok(from_unix(t.unix_timestamp()))
}

fn parse_certs(pem: &[u8], path: &Path) -> Result<Vec<CertificateDer<'static>>, IdentityError> {
    let certs: Vec<CertificateDer<'static>> = CertificateDer::pem_slice_iter(pem)
        .collect::<Result<_, _>>()
        .map_err(|_| IdentityError::Pem {
            path: path.to_owned(),
            what: "certificate",
        })?;
    if certs.is_empty() {
        return Err(IdentityError::Pem {
            path: path.to_owned(),
            what: "certificate",
        });
    }
    Ok(certs)
}

fn parse_key(pem: &[u8], path: &Path) -> Result<PrivateKeyDer<'static>, IdentityError> {
    PrivateKeyDer::from_pem_slice(pem).map_err(|_| IdentityError::Pem {
        path: path.to_owned(),
        what: "private key",
    })
}

/// Reads the identity files. A missing file or a damaged `identity.json`
/// means that the agent has no identity.
fn load(dir: &Path) -> Result<Option<Stored>, IdentityError> {
    let key_path = dir.join(KEY_FILE);
    let cert_path = dir.join(CERT_FILE);
    let read = |p: &Path| match std::fs::read(p) {
        Ok(b) => Ok(Some(b)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(io_err("read", p)(e)),
    };
    let (Some(key_pem), Some(cert_pem)) = (read(&key_path)?, read(&cert_path)?) else {
        return Ok(None);
    };
    let mode = std::fs::metadata(&key_path)
        .map_err(io_err("stat", &key_path))?
        .permissions()
        .mode()
        & 0o777;
    if mode & 0o077 != 0 {
        return Err(IdentityError::KeyMode {
            path: key_path,
            mode,
        });
    }
    let Some(meta) = read(&dir.join(META_FILE))? else {
        return Ok(None);
    };
    let Ok(meta) = serde_json::from_slice::<Meta>(&meta) else {
        tracing::warn!(dir = %dir.display(), "the identity.json file is damaged, so the agent enrolls again");
        return Ok(None);
    };
    Ok(Some(Stored {
        cert_pem,
        key_pem,
        not_after: from_unix(meta.not_after),
        renew_at: from_unix(meta.renew_at),
    }))
}

fn create_dir(dir: &Path) -> Result<(), IdentityError> {
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
        .map_err(io_err("create", dir))
}

/// Writes `bytes` to `path` through a temporary file in the same directory
/// and a rename, so a reader sees the old file or the new one.
fn write_atomic(path: &Path, bytes: &[u8], mode: u32) -> Result<(), IdentityError> {
    let tmp = path.with_extension("tmp");
    let _ = std::fs::remove_file(&tmp);
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(mode)
        .open(&tmp)
        .map_err(io_err("create", &tmp))?;
    file.write_all(bytes)
        .and_then(|()| file.sync_all())
        .map_err(io_err("write", &tmp))?;
    drop(file);
    std::fs::rename(&tmp, path).map_err(io_err("rename to", path))
}

fn new_key_and_csr(agent_id: &str) -> Result<(rcgen::KeyPair, String), IdentityError> {
    let key = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256)
        .map_err(|e| IdentityError::Keygen(e.to_string()))?;
    let csr = csr_for(agent_id, &key)?;
    Ok((key, csr))
}

/// Builds a CSR with the common name `agent_id`, which the controller needs.
fn csr_for(agent_id: &str, key: &rcgen::KeyPair) -> Result<String, IdentityError> {
    let keygen = |e: rcgen::Error| IdentityError::Keygen(e.to_string());
    let mut params = rcgen::CertificateParams::new(Vec::<String>::new()).map_err(keygen)?;
    let mut name = rcgen::DistinguishedName::new();
    name.push(rcgen::DnType::CommonName, agent_id);
    params.distinguished_name = name;
    params
        .serialize_request(key)
        .map_err(keygen)?
        .pem()
        .map_err(keygen)
}

/// A verifier for the first enrollment with a pinned fingerprint and no CA
/// file. It runs inside the TLS handshake, so a refusal ends the connection
/// before the agent sends the token.
///
/// Since CTL-31 the endpoint presents the leaf and the CA. The verifier looks
/// for the certificate with the pinned SHA-256 among the certificates that
/// follow the leaf, makes a root store of only that certificate, and hands
/// the rest of the check (the chain, the validity, the usage, and the name of
/// the address that the agent dialed) to the webpki verifier of rustls. The
/// handshake signatures use the algorithms of the provider as usual.
#[derive(Debug)]
struct PinnedVerifier {
    pin: [u8; 32],
    provider: Arc<CryptoProvider>,
}

impl ServerCertVerifier for PinnedVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        let Some(at) = intermediates
            .iter()
            .position(|c| fingerprint(c) == self.pin)
        else {
            let found: Vec<String> = intermediates.iter().map(|c| hex(&fingerprint(c))).collect();
            return Err(rustls::Error::General(format!(
                "the endpoint presented no certificate with the pinned fingerprint sha256:{} \
                 after its own (it presented {})",
                hex(&self.pin),
                if found.is_empty() {
                    "none".to_owned()
                } else {
                    found.join(",")
                }
            )));
        };
        let mut roots = RootCertStore::empty();
        roots
            .add(intermediates[at].clone().into_owned())
            .map_err(|e| rustls::Error::General(format!("the pinned CA is not usable: {e}")))?;
        let rest: Vec<CertificateDer<'_>> = intermediates
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != at)
            .map(|(_, c)| c.clone())
            .collect();
        WebPkiServerVerifier::builder_with_provider(Arc::new(roots), Arc::clone(&self.provider))
            .build()
            .map_err(|e| rustls::Error::General(e.to_string()))?
            .verify_server_cert(end_entity, &rest, server_name, ocsp, now)
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

/// Checks that the `ca_pem` of the answer holds the certificate with the
/// pinned fingerprint, which is the file that the agent stores as its CA.
/// The handshake already checked that the endpoint presented that CA and that
/// its certificate chains to it.
fn check_pinned_ca(pin: &[u8; 32], ca_pem: &str) -> Result<(), IdentityError> {
    let cas: Vec<CertificateDer<'static>> = CertificateDer::pem_slice_iter(ca_pem.as_bytes())
        .collect::<Result<_, _>>()
        .map_err(|_| IdentityError::Answer("ca_pem holds no valid certificate".to_owned()))?;
    if cas.iter().any(|c| fingerprint(c) == *pin) {
        return Ok(());
    }
    let found: Vec<String> = cas.iter().map(|c| hex(&fingerprint(c))).collect();
    Err(IdentityError::PinMismatch {
        want: hex(pin),
        found: if found.is_empty() {
            "none".to_owned()
        } else {
            found.join(",")
        },
    })
}

fn provider() -> Arc<CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

/// Builds the rustls client configuration for the HTTPS requests.
fn client_config(
    trust: &Trust,
    identity: Option<(Vec<CertificateDer<'static>>, PrivateKeyDer<'static>)>,
) -> Result<Arc<ClientConfig>, IdentityError> {
    let provider = provider();
    let setup = |e: rustls::Error| IdentityError::Handshake {
        address: String::new(),
        detail: e.to_string(),
    };
    let builder = ClientConfig::builder_with_provider(Arc::clone(&provider))
        .with_safe_default_protocol_versions()
        .map_err(setup)?;
    let builder = match trust {
        Trust::Roots(_, ders) => {
            let mut roots = RootCertStore::empty();
            for der in ders {
                roots.add(der.clone()).map_err(setup)?;
            }
            builder.with_root_certificates(roots)
        }
        Trust::Pin(pin) => builder
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(PinnedVerifier {
                pin: *pin,
                provider: Arc::clone(&provider),
            })),
    };
    let mut config = match identity {
        Some((chain, key)) => builder.with_client_auth_cert(chain, key).map_err(setup)?,
        None => builder.with_no_client_auth(),
    };
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    Ok(Arc::new(config))
}

fn split_address(address: &str) -> Result<&str, IdentityError> {
    let invalid = || IdentityError::Network {
        address: address.to_owned(),
        detail: "the enrollment address must be HOST:PORT".to_owned(),
    };
    let (host, port) = address.rsplit_once(':').ok_or_else(invalid)?;
    if port.parse::<u16>().is_err() || host.is_empty() {
        return Err(invalid());
    }
    Ok(host.trim_start_matches('[').trim_end_matches(']'))
}

fn server_name(address: &str) -> Result<ServerName<'static>, IdentityError> {
    let host = split_address(address)?;
    ServerName::try_from(host.to_owned()).map_err(|_| IdentityError::Network {
        address: address.to_owned(),
        detail: format!("{host:?} is not a valid host name or address"),
    })
}

/// The HTTP answer.
#[derive(Debug)]
struct Answer {
    status: u16,
    body: Vec<u8>,
}

/// Sends one `POST` with a JSON body over TLS and reads the answer.
async fn post(
    address: &str,
    config: Arc<ClientConfig>,
    path: &str,
    body: String,
) -> Result<Answer, IdentityError> {
    let network = |detail: String| IdentityError::Network {
        address: address.to_owned(),
        detail,
    };
    let tls_or_network = |e: &(dyn std::error::Error + 'static)| match tls_failure(e) {
        Some(detail) => IdentityError::Handshake {
            address: address.to_owned(),
            detail,
        },
        None => IdentityError::Network {
            address: address.to_owned(),
            detail: chain(e),
        },
    };
    let name = server_name(address)?;
    let host = split_address(address)?.to_owned();
    let exchange = async {
        let tcp = tokio::time::timeout(CONNECT_TIMEOUT, TcpStream::connect(address))
            .await
            .map_err(|_| network("the connection timed out".to_owned()))?
            .map_err(|e| network(e.to_string()))?;
        let _ = tcp.set_nodelay(true);
        let tls = TlsConnector::from(config)
            .connect(name, tcp)
            .await
            .map_err(|e| tls_or_network(&e))?;
        let (mut sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(tls))
            .await
            .map_err(|e| tls_or_network(&e))?;
        let driver = tokio::spawn(connection);
        let request = Request::builder()
            .method("POST")
            .uri(path)
            .header(HOST, host.as_str())
            .header(CONTENT_TYPE, "application/json")
            .header(CONNECTION, "close")
            .body(Full::new(Bytes::from(body)))
            .map_err(|e| network(e.to_string()))?;
        let response = sender
            .send_request(request)
            .await
            .map_err(|e| tls_or_network(&e))?;
        let status = response.status().as_u16();
        let collected = Limited::new(response.into_body(), MAX_ANSWER)
            .collect()
            .await
            .map_err(|e| network(format!("read the answer: {e}")))?;
        driver.abort();
        Ok(Answer {
            status,
            body: collected.to_bytes().to_vec(),
        })
    };
    tokio::time::timeout(REQUEST_TIMEOUT, exchange)
        .await
        .map_err(|_| network("the request timed out".to_owned()))?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprints_parse_in_the_usual_spellings() {
        let hexed = "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff";
        let want = parse_fingerprint(hexed).expect("plain hex");
        assert_eq!(want[0], 0x00);
        assert_eq!(want[31], 0xff);
        assert_eq!(
            parse_fingerprint(&hexed.to_uppercase()).expect("upper"),
            want
        );
        assert_eq!(
            parse_fingerprint(&format!("sha256:{hexed}")).expect("prefix"),
            want
        );
        let colons: Vec<String> = hexed
            .as_bytes()
            .chunks(2)
            .map(|c| String::from_utf8_lossy(c).into_owned())
            .collect();
        assert_eq!(parse_fingerprint(&colons.join(":")).expect("colons"), want);
        assert!(parse_fingerprint("abcd").is_err());
        assert!(parse_fingerprint(&"zz".repeat(32)).is_err());
    }

    #[test]
    fn a_csr_carries_the_agent_id_as_its_common_name() {
        let (key, csr) = new_key_and_csr("edge-1").expect("csr");
        assert!(csr.starts_with("-----BEGIN CERTIFICATE REQUEST-----"));
        let der = rustls_pki_types::CertificateSigningRequestDer::from_pem_slice(csr.as_bytes())
            .expect("pem");
        assert!(der.windows(6).any(|w| w == b"edge-1"));
        assert!(key.serialize_pem().contains("PRIVATE KEY"));
    }

    #[test]
    fn addresses_need_a_host_and_a_port() {
        assert_eq!(split_address("10.0.0.1:8445").expect("ip"), "10.0.0.1");
        assert_eq!(split_address("[::1]:8445").expect("v6"), "::1");
        assert!(split_address("10.0.0.1").is_err());
        assert!(split_address(":8445").is_err());
        assert!(server_name("localhost:8445").is_ok());
    }

    #[test]
    fn atomic_writes_set_the_mode_and_leave_no_temporary_file() {
        let dir = std::env::temp_dir().join(format!("fps-identity-{}", std::process::id()));
        create_dir(&dir).expect("dir");
        let path = dir.join(KEY_FILE);
        write_atomic(&path, b"one", 0o600).expect("first");
        write_atomic(&path, b"two", 0o600).expect("second");
        assert_eq!(std::fs::read(&path).expect("read"), b"two");
        let mode = std::fs::metadata(&path).expect("stat").permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        assert!(!dir.join("agent.tmp").exists());
        std::fs::remove_dir_all(&dir).expect("cleanup");
    }

    #[test]
    fn a_key_that_others_can_read_is_refused() {
        let dir = std::env::temp_dir().join(format!("fps-identity-mode-{}", std::process::id()));
        create_dir(&dir).expect("dir");
        write_atomic(&dir.join(KEY_FILE), b"k", 0o644).expect("key");
        write_atomic(&dir.join(CERT_FILE), b"c", 0o644).expect("cert");
        assert!(matches!(load(&dir), Err(IdentityError::KeyMode { .. })));
        std::fs::remove_dir_all(&dir).expect("cleanup");
    }

    #[test]
    fn a_rustls_error_inside_an_io_error_is_found() {
        let inner = rustls::Error::AlertReceived(rustls::AlertDescription::BadCertificate);
        let io = std::io::Error::new(std::io::ErrorKind::InvalidData, inner);
        let found = tls_failure(&io).expect("found");
        assert!(found.contains("BadCertificate"), "{found}");
        let plain = std::io::Error::new(std::io::ErrorKind::ConnectionRefused, "refused");
        assert!(tls_failure(&plain).is_none());
    }

    #[test]
    fn enrollment_times_parse_with_nanoseconds() {
        let t = parse_time("2026-11-09T05:22:20.123456789Z").expect("time");
        assert_eq!(unix(t), 1_794_201_740);
        assert!(parse_time("yesterday").is_err());
    }
}
