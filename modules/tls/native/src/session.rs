// SPDX-License-Identifier: GPL-3.0-only
//! The TLS session wrapper, connector, and acceptor.

use std::future::poll_fn;
use std::io::{self, Read, Write};
use std::pin::Pin;
use std::sync::Arc;
use std::task::{ready, Context, Poll};
use std::time::Duration;

use fps_io::SessionIo;
use rustls::client::Resumption;
use rustls::pki_types::ServerName;
use rustls::server::{NoServerSessionStorage, ServerSessionMemoryCache};
use rustls::{
    AlertDescription, ClientConfig, ClientConnection, Connection, HandshakeKind, RootCertStore,
    ServerConfig, ServerConnection,
};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::time::Instant;

use crate::authority::ServerIdentity;
use crate::error::{snake_case, Side, TlsError};
use crate::profile::{
    provider, versions, ClientProfile, ClientResumption, Group, ServerProfile, ServerResumption,
    TlsVersion,
};

/// Size of the ciphertext read buffer, which holds one maximum size TLS record.
const READ_CHUNK: usize = 16 * 1024 + 512;

/// What a completed handshake negotiated. The fields feed the `tls.*` metrics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HandshakeReport {
    /// The side that this report describes.
    pub side: Side,
    /// The negotiated protocol version.
    pub version: TlsVersion,
    /// The IANA name of the negotiated cipher suite.
    pub cipher_suite: String,
    /// The negotiated key exchange group, in lower case such as `x25519`, when
    /// rustls reports it. A resumed TLS 1.2 session has none.
    pub group: Option<String>,
    /// The negotiated ALPN protocol id.
    pub alpn: Option<Vec<u8>>,
    /// The server name that the client sent, or that the server received. It is
    /// `None` when no SNI crossed the wire.
    pub server_name: Option<String>,
    /// Whether the handshake resumed an earlier session, see `tls.resumptions`.
    pub resumed: bool,
    /// Time from the start of the handshake to its end on the Tokio clock, the
    /// value of `tls.handshake_time`. A client counts from the ClientHello, and
    /// a server from the start of `accept`, so a server includes the wait for
    /// the ClientHello.
    pub elapsed: Duration,
}

/// A session that runs over TLS.
///
/// The session is a [`SessionIo`], so a flow program runs over it as over a
/// plain session. Dropping it closes nothing on the wire. A caller that wants
/// a clean close shuts the write side down first, which sends `close_notify`
/// before the transport shutdown. A transport that ends without `close_notify`
/// reads as end of stream, because many servers close that way.
pub struct TlsSession<IO> {
    io: IO,
    conn: Connection,
    inbound: Box<[u8]>,
    outbound: Vec<u8>,
    out_pos: usize,
    close_sent: bool,
    report: HandshakeReport,
}

impl<IO> std::fmt::Debug for TlsSession<IO> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TlsSession")
            .field("report", &self.report)
            .finish_non_exhaustive()
    }
}

impl<IO: SessionIo> TlsSession<IO> {
    fn new(io: IO, conn: Connection, side: Side) -> Self {
        let placeholder = HandshakeReport {
            side,
            version: TlsVersion::Tls13,
            cipher_suite: String::new(),
            group: None,
            alpn: None,
            server_name: None,
            resumed: false,
            elapsed: Duration::ZERO,
        };
        Self {
            io,
            conn,
            inbound: vec![0u8; READ_CHUNK].into_boxed_slice(),
            outbound: Vec::with_capacity(READ_CHUNK),
            out_pos: 0,
            close_sent: false,
            report: placeholder,
        }
    }

    /// Returns what the handshake negotiated.
    pub fn report(&self) -> &HandshakeReport {
        &self.report
    }

    /// Returns the underlying session.
    pub fn get_ref(&self) -> &IO {
        &self.io
    }

    /// Writes pending TLS records to the transport.
    fn poll_flush_tls(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        loop {
            while self.out_pos < self.outbound.len() {
                let n =
                    ready!(Pin::new(&mut self.io).poll_write(cx, &self.outbound[self.out_pos..]))?;
                if n == 0 {
                    return Poll::Ready(Err(io::ErrorKind::WriteZero.into()));
                }
                self.out_pos += n;
            }
            self.outbound.clear();
            self.out_pos = 0;
            if !self.conn.wants_write() {
                return Poll::Ready(Ok(()));
            }
            self.conn.write_tls(&mut self.outbound)?;
        }
    }

    /// Reads one chunk of ciphertext and feeds it to rustls. Returns the chunk
    /// size, and zero at the end of the transport.
    fn poll_fill(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<usize>> {
        let n = {
            let mut buf = ReadBuf::new(&mut self.inbound);
            ready!(Pin::new(&mut self.io).poll_read(cx, &mut buf))?;
            buf.filled().len()
        };
        if n == 0 {
            // Tell rustls about the end of the transport, so a read reports it.
            let mut empty: &[u8] = &[];
            self.conn.read_tls(&mut empty)?;
            return Poll::Ready(Ok(0));
        }
        let mut rest = &self.inbound[..n];
        while !rest.is_empty() {
            let before = rest.len();
            self.conn.read_tls(&mut rest)?;
            self.conn.process_new_packets().map_err(protocol_error)?;
            if rest.len() == before {
                break;
            }
        }
        Poll::Ready(Ok(n))
    }

    async fn drive_handshake(&mut self) -> io::Result<()> {
        loop {
            if !self.conn.is_handshaking() {
                // The last flight must go out in full, because the peer waits
                // for it. The exception is the pair of session tickets that a
                // TLS 1.3 server sends after the client Finished: the client
                // is done and reads them only with its next read, so waiting
                // here would deadlock two peers over a transport that buffers
                // less than the tickets. The next read, write, flush, or
                // shutdown sends what is left.
                let tickets_only = matches!(self.conn, Connection::Server(_))
                    && self.conn.protocol_version() == Some(rustls::ProtocolVersion::TLSv1_3);
                return poll_fn(|cx| match self.poll_flush_tls(cx) {
                    Poll::Pending if tickets_only => Poll::Ready(Ok(())),
                    ready => ready,
                })
                .await;
            }
            poll_fn(|cx| self.poll_flush_tls(cx)).await?;
            if poll_fn(|cx| self.poll_fill(cx)).await? == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "the transport ended during the handshake",
                ));
            }
        }
    }

    /// Runs the handshake and fills the report, or builds the error of a failure.
    async fn handshake(
        mut self,
        side: Side,
        peer: String,
        started: Instant,
    ) -> Result<Self, TlsError> {
        match self.drive_handshake().await {
            Ok(()) => {
                self.report = self.make_report(side, started);
                Ok(self)
            }
            Err(source) => {
                // The failure queued an alert for the peer, so make a best
                // effort to send it. The transport may be gone, which is fine.
                let alert_sent = self.pending_plaintext_alert();
                let _ = poll_fn(|cx| self.poll_flush_tls(cx)).await;
                Err(handshake_error(side, peer, source, alert_sent))
            }
        }
    }

    /// Returns the description of a plaintext alert record that waits in the
    /// output, which is how rustls queues an alert it raised before the
    /// handshake keys exist.
    fn pending_plaintext_alert(&mut self) -> Option<AlertDescription> {
        if self.conn.wants_write() && self.out_pos >= self.outbound.len() {
            self.outbound.clear();
            self.out_pos = 0;
            let _ = self.conn.write_tls(&mut self.outbound);
        }
        match self.outbound.get(self.out_pos..)? {
            [21, _, _, 0, 2, _level, description, ..] => Some(AlertDescription::from(*description)),
            _ => None,
        }
    }

    fn make_report(&self, side: Side, started: Instant) -> HandshakeReport {
        let version = match self.conn.protocol_version() {
            Some(rustls::ProtocolVersion::TLSv1_2) => TlsVersion::Tls12,
            _ => TlsVersion::Tls13,
        };
        let server_name = match &self.conn {
            Connection::Server(s) => s.server_name().map(str::to_owned),
            Connection::Client(_) => self.report.server_name.clone(),
        };
        HandshakeReport {
            side,
            version,
            cipher_suite: self
                .conn
                .negotiated_cipher_suite()
                .map(|s| iana_name(s.suite()))
                .unwrap_or_default(),
            group: self
                .conn
                .negotiated_key_exchange_group()
                .map(Group::name_of),
            alpn: self.conn.alpn_protocol().map(<[u8]>::to_vec),
            server_name,
            resumed: self.conn.handshake_kind() == Some(HandshakeKind::Resumed),
            elapsed: started.elapsed(),
        }
    }
}

/// Returns the IANA name of a suite, such as `TLS_AES_128_GCM_SHA256`.
fn iana_name(suite: rustls::CipherSuite) -> String {
    // Rustls names its TLS 1.3 suites TLS13_*, while IANA writes TLS_*.
    format!("{suite:?}").replacen("TLS13_", "TLS_", 1)
}

fn protocol_error(e: rustls::Error) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, e)
}

/// Builds a handshake error and extracts the alerts from its cause.
fn handshake_error(
    side: Side,
    peer: String,
    source: io::Error,
    sent: Option<AlertDescription>,
) -> TlsError {
    let tls = source
        .get_ref()
        .and_then(|e| e.downcast_ref::<rustls::Error>());
    let alert_received = match tls {
        Some(rustls::Error::AlertReceived(a)) => Some(snake_case(&format!("{a:?}"))),
        _ => None,
    };
    let alert_sent = sent
        .map(|a| snake_case(&format!("{a:?}")))
        .or_else(|| match tls {
            Some(rustls::Error::InvalidCertificate(rustls::CertificateError::UnknownIssuer)) => {
                Some("unknown_ca".to_owned())
            }
            Some(rustls::Error::InvalidCertificate(rustls::CertificateError::Expired)) => {
                Some("certificate_expired".to_owned())
            }
            Some(rustls::Error::InvalidCertificate(_)) => Some("bad_certificate".to_owned()),
            Some(rustls::Error::PeerIncompatible(_)) => Some("handshake_failure".to_owned()),
            Some(rustls::Error::NoApplicationProtocol) => {
                Some("no_application_protocol".to_owned())
            }
            Some(rustls::Error::DecryptError) => Some("bad_record_mac".to_owned()),
            _ => None,
        });
    TlsError::Handshake {
        side,
        peer,
        source,
        alert_received,
        alert_sent,
    }
}

impl<IO: SessionIo> AsyncRead for TlsSession<IO> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        if buf.remaining() == 0 {
            return Poll::Ready(Ok(()));
        }
        loop {
            match this.conn.reader().read(buf.initialize_unfilled()) {
                Ok(n) => {
                    buf.advance(n);
                    return Poll::Ready(Ok(()));
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {}
                // The transport ended without close_notify, which reads as end of stream.
                Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Poll::Ready(Ok(())),
                Err(e) => return Poll::Ready(Err(e)),
            }
            // Send what rustls queued, such as a key update answer, while we wait.
            if let Poll::Ready(Err(e)) = this.poll_flush_tls(cx) {
                return Poll::Ready(Err(e));
            }
            ready!(this.poll_fill(cx))?;
        }
    }
}

impl<IO: SessionIo> AsyncWrite for TlsSession<IO> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        if buf.is_empty() {
            return Poll::Ready(Ok(0));
        }
        loop {
            let n = this.conn.writer().write(buf)?;
            if n > 0 {
                if let Poll::Ready(Err(e)) = this.poll_flush_tls(cx) {
                    return Poll::Ready(Err(e));
                }
                return Poll::Ready(Ok(n));
            }
            ready!(this.poll_flush_tls(cx))?;
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        this.conn.writer().flush()?;
        ready!(this.poll_flush_tls(cx))?;
        Pin::new(&mut this.io).poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        if !this.close_sent {
            this.conn.send_close_notify();
            this.close_sent = true;
        }
        ready!(this.poll_flush_tls(cx))?;
        Pin::new(&mut this.io).poll_shutdown(cx)
    }
}

impl<IO: SessionIo> SessionIo for TlsSession<IO> {
    fn abort(&mut self) {
        self.io.abort();
    }
}

/// Opens client sessions over TLS with one client profile.
///
/// A connector owns its resumption store. Sessions that one connector opens to
/// the same server name resume, so a run shares one connector per profile.
#[derive(Clone)]
pub struct TlsConnector {
    config: Arc<ClientConfig>,
    profile: Arc<str>,
}

impl std::fmt::Debug for TlsConnector {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TlsConnector")
            .field("profile", &self.profile)
            .finish_non_exhaustive()
    }
}

impl TlsConnector {
    /// Builds a connector that verifies servers against `roots`.
    pub fn new(profile: &ClientProfile, roots: RootCertStore) -> Result<Self, TlsError> {
        let mut config =
            ClientConfig::builder_with_provider(provider(&profile.groups, &profile.cipher_suites))
                .with_protocol_versions(&versions(&profile.versions)?)
                .map_err(|e| TlsError::Config(format!("client profile {}: {e}", profile.name)))?
                .with_root_certificates(roots)
                .with_no_client_auth();
        config.alpn_protocols = profile.alpn.clone();
        config.enable_sni = profile.send_sni;
        config.resumption = match profile.resumption {
            ClientResumption::Disabled => Resumption::disabled(),
            ClientResumption::InMemory { sessions } => Resumption::in_memory_sessions(sessions),
        };
        if let Some(log) = &profile.key_log {
            config.key_log = Arc::clone(log);
        }
        Ok(Self {
            config: Arc::new(config),
            profile: profile.name.as_str().into(),
        })
    }

    /// Returns the rustls configuration, for callers that drive a connection themselves.
    pub fn config(&self) -> &Arc<ClientConfig> {
        &self.config
    }

    /// Runs the client handshake over `io` and returns the secured session.
    ///
    /// `server_name` is the name to verify and, when `send_sni` is set, to send.
    pub async fn connect<IO: SessionIo>(
        &self,
        io: IO,
        server_name: &str,
    ) -> Result<TlsSession<IO>, TlsError> {
        let started = Instant::now();
        let name = ServerName::try_from(server_name.to_owned())
            .map_err(|e| TlsError::Config(format!("server name `{server_name}`: {e}")))?;
        let conn = ClientConnection::new(Arc::clone(&self.config), name)
            .map_err(|e| TlsError::Config(format!("client profile {}: {e}", self.profile)))?;
        let mut session = TlsSession::new(io, Connection::Client(conn), Side::Client);
        let sni = self.config.enable_sni
            && !matches!(
                ServerName::try_from(server_name),
                Ok(ServerName::IpAddress(_))
            );
        session.report.server_name = sni.then(|| server_name.to_owned());
        session
            .handshake(Side::Client, server_name.to_owned(), started)
            .await
    }
}

/// Accepts server sessions over TLS with one server profile and one certificate.
#[derive(Clone)]
pub struct TlsAcceptor {
    config: Arc<ServerConfig>,
    profile: Arc<str>,
}

impl std::fmt::Debug for TlsAcceptor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TlsAcceptor")
            .field("profile", &self.profile)
            .finish_non_exhaustive()
    }
}

impl TlsAcceptor {
    /// Builds an acceptor that presents `identity`.
    pub fn new(profile: &ServerProfile, identity: &ServerIdentity) -> Result<Self, TlsError> {
        let mut config =
            ServerConfig::builder_with_provider(provider(&profile.groups, &profile.cipher_suites))
                .with_protocol_versions(&versions(&profile.versions)?)
                .map_err(|e| TlsError::Config(format!("server profile {}: {e}", profile.name)))?
                .with_no_client_auth()
                .with_single_cert(
                    identity.chain().to_vec(),
                    rustls::pki_types::PrivateKeyDer::Pkcs8(identity.key()),
                )
                .map_err(|e| {
                    TlsError::Config(format!("server profile {}: certificate: {e}", profile.name))
                })?;
        config.alpn_protocols = profile.alpn.clone();
        config.ignore_client_order = profile.use_server_cipher_order;
        match profile.resumption {
            ServerResumption::Disabled => {
                config.session_storage = Arc::new(NoServerSessionStorage {});
                config.send_tls13_tickets = 0;
            }
            ServerResumption::Tickets => {
                config.ticketer = rustls::crypto::ring::Ticketer::new().map_err(|e| {
                    TlsError::Config(format!("server profile {}: ticketer: {e}", profile.name))
                })?;
            }
            ServerResumption::SessionCache { sessions } => {
                config.session_storage = ServerSessionMemoryCache::new(sessions);
            }
        }
        if let Some(log) = &profile.key_log {
            config.key_log = Arc::clone(log);
        }
        Ok(Self {
            config: Arc::new(config),
            profile: profile.name.as_str().into(),
        })
    }

    /// Returns the rustls configuration, for callers that drive a connection themselves.
    pub fn config(&self) -> &Arc<ServerConfig> {
        &self.config
    }

    /// Runs the server handshake over `io` and returns the secured session.
    pub async fn accept<IO: SessionIo>(&self, io: IO) -> Result<TlsSession<IO>, TlsError> {
        let started = Instant::now();
        let conn = ServerConnection::new(Arc::clone(&self.config))
            .map_err(|e| TlsError::Config(format!("server profile {}: {e}", self.profile)))?;
        TlsSession::new(io, Connection::Server(conn), Side::Server)
            .handshake(Side::Server, "client".to_owned(), started)
            .await
    }
}
