// SPDX-License-Identifier: GPL-3.0-only
//! Component tests of mutual TLS, enrollment, and renewal (AGT-08).
//!
//! A fake enrollment endpoint (HTTPS, `POST /enroll` and `POST /renew` with the
//! rules of the `internal/pki` package) and a fake `ControllerService` over
//! mutual TLS run in process on loopback ports. Both use a CA that the test
//! makes with rcgen. The real controller runs in `tests/controller_e2e.sh`.

use std::collections::HashSet;
use std::net::SocketAddr;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use bytes::Bytes;
use fps_agent::identity::{TlsConfig, TokenSource};
use fps_agent::pb::controller_service_server::{ControllerService, ControllerServiceServer};
use fps_agent::pb::{self, session_request};
use fps_agent::session::{serve, Backoff, Config, SessionError};
use http_body_util::{BodyExt as _, Full};
use hyper::service::service_fn;
use hyper::{Request, Response as HttpResponse, StatusCode};
use hyper_util::rt::TokioIo;
use rcgen::{
    BasicConstraints, Certificate, CertificateParams, CertificateSigningRequestParams,
    DistinguishedName, DnType, DnValue, ExtendedKeyUsagePurpose, IsCa, KeyPair, KeyUsagePurpose,
};
use rustls::server::WebPkiClientVerifier;
use rustls::RootCertStore;
use rustls_pki_types::PrivatePkcs8KeyDer;
use sha2::{Digest, Sha256};
use tokio::sync::oneshot;
use tokio_stream::wrappers::TcpListenerStream;
use tokio_stream::Stream;
use tonic::transport::{Certificate as TonicCert, Identity as TonicIdentity, ServerTlsConfig};
use tonic::{Request as GrpcRequest, Response, Status, Streaming};

const WAIT: Duration = Duration::from_secs(20);
const AGENT: &str = "mtls-agent";

fn provider() -> Arc<rustls::crypto::CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

fn key_pair() -> KeyPair {
    KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).expect("key")
}

/// A test CA.
struct Pki {
    cert: Certificate,
    key: KeyPair,
}

/// A certificate with its key.
struct Leaf {
    cert_pem: String,
    key_pem: String,
    cert_der: rustls_pki_types::CertificateDer<'static>,
    key_der: Vec<u8>,
}

impl Pki {
    fn new(name: &str) -> Self {
        let key = key_pair();
        let mut p = CertificateParams::new(Vec::<String>::new()).expect("ca params");
        p.is_ca = IsCa::Ca(BasicConstraints::Constrained(0));
        p.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
        let mut dn = DistinguishedName::new();
        dn.push(DnType::CommonName, name);
        p.distinguished_name = dn;
        let cert = p.self_signed(&key).expect("ca cert");
        Self { cert, key }
    }

    fn pem(&self) -> String {
        self.cert.pem()
    }

    fn fingerprint(&self) -> [u8; 32] {
        Sha256::digest(self.cert.der().as_ref()).into()
    }

    /// A certificate for `127.0.0.1` with the server authentication usage.
    fn server(&self) -> Leaf {
        let key = key_pair();
        let mut p = CertificateParams::new(vec!["127.0.0.1".to_owned()]).expect("server params");
        p.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
        let cert = p
            .signed_by(&key, &self.cert, &self.key)
            .expect("server cert");
        Leaf {
            cert_pem: cert.pem(),
            key_pem: key.serialize_pem(),
            cert_der: cert.der().clone(),
            key_der: key.serialize_der(),
        }
    }

    /// A client certificate for `id`, as the controller would issue it.
    fn client(&self, id: &str) -> Leaf {
        let key = key_pair();
        let mut p = CertificateParams::new(Vec::<String>::new()).expect("client params");
        let mut dn = DistinguishedName::new();
        dn.push(DnType::CommonName, id);
        p.distinguished_name = dn;
        p.extended_key_usages = vec![ExtendedKeyUsagePurpose::ClientAuth];
        let cert = p
            .signed_by(&key, &self.cert, &self.key)
            .expect("client cert");
        Leaf {
            cert_pem: cert.pem(),
            key_pem: key.serialize_pem(),
            cert_der: cert.der().clone(),
            key_der: key.serialize_der(),
        }
    }

    /// Signs the public key of a CSR for client authentication, as the
    /// controller does, and returns the common name and the PEM.
    fn issue(&self, csr_pem: &str) -> (String, String) {
        let mut csr = CertificateSigningRequestParams::from_pem(csr_pem).expect("a valid CSR");
        let cn = match csr.params.distinguished_name.get(&DnType::CommonName) {
            Some(DnValue::Utf8String(s)) => s.clone(),
            Some(DnValue::PrintableString(s)) => s.as_str().to_owned(),
            other => panic!("the CSR has no common name: {other:?}"),
        };
        csr.params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ClientAuth];
        let cert = csr.signed_by(&self.cert, &self.key).expect("client cert");
        (cn, cert.pem())
    }
}

fn rfc3339(t: SystemTime) -> String {
    let secs = t.duration_since(UNIX_EPOCH).expect("after 1970").as_secs() as i64;
    time::OffsetDateTime::from_unix_timestamp(secs)
        .expect("time")
        .format(&time::format_description::well_known::Rfc3339)
        .expect("format")
}

/// What the fake enrollment endpoint saw.
#[derive(Default)]
struct EnrollState {
    tokens: Mutex<HashSet<String>>,
    enrolls: AtomicU32,
    renews: AtomicU32,
    /// The common name of every CSR.
    names: Mutex<Vec<String>>,
    /// How long after an enrollment `renew_at` lies, in milliseconds.
    renew_after_ms: Mutex<u64>,
}

struct Enroller {
    addr: SocketAddr,
    state: Arc<EnrollState>,
}

/// Serves the endpoint on a loopback port. The server certificate comes from
/// `server_pki` and the CA of the certificates that it issues is `issuer`.
async fn enroller(server_pki: &Pki, issuer: Arc<Pki>, tokens: &[&str]) -> Enroller {
    let state = Arc::new(EnrollState::default());
    state
        .tokens
        .lock()
        .expect("tokens")
        .extend(tokens.iter().map(|t| (*t).to_owned()));
    *state.renew_after_ms.lock().expect("renew_after") = 3_600_000;
    let leaf = server_pki.server();
    let mut roots = RootCertStore::empty();
    roots.add(issuer.cert.der().clone()).expect("root");
    let verifier = WebPkiClientVerifier::builder_with_provider(Arc::new(roots), provider())
        .allow_unauthenticated()
        .build()
        .expect("client verifier");
    let mut config = rustls::ServerConfig::builder_with_provider(provider())
        .with_safe_default_protocol_versions()
        .expect("versions")
        .with_client_cert_verifier(verifier)
        .with_single_cert(
            vec![leaf.cert_der.clone()],
            PrivatePkcs8KeyDer::from(leaf.key_der.clone()).into(),
        )
        .expect("server config");
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(config));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");
    let st = Arc::clone(&state);
    tokio::spawn(async move {
        loop {
            let Ok((tcp, _)) = listener.accept().await else {
                return;
            };
            let acceptor = acceptor.clone();
            let st = Arc::clone(&st);
            let issuer = Arc::clone(&issuer);
            tokio::spawn(async move {
                let Ok(tls) = acceptor.accept(tcp).await else {
                    return;
                };
                let has_cert = tls.get_ref().1.peer_certificates().is_some();
                let svc = service_fn(move |req| {
                    let st = Arc::clone(&st);
                    let issuer = Arc::clone(&issuer);
                    async move {
                        Ok::<_, std::convert::Infallible>(handle(req, &st, &issuer, has_cert).await)
                    }
                });
                let _ = hyper::server::conn::http1::Builder::new()
                    .serve_connection(TokioIo::new(tls), svc)
                    .await;
            });
        }
    });
    Enroller { addr, state }
}

fn reply(status: StatusCode, body: String) -> HttpResponse<Full<Bytes>> {
    HttpResponse::builder()
        .status(status)
        .header("Content-Type", "application/json")
        .body(Full::new(Bytes::from(body)))
        .expect("response")
}

async fn handle(
    req: Request<hyper::body::Incoming>,
    st: &EnrollState,
    issuer: &Pki,
    has_cert: bool,
) -> HttpResponse<Full<Bytes>> {
    let path = req.uri().path().to_owned();
    let body = req.into_body().collect().await.expect("body").to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).expect("a JSON body");
    let denied = || {
        reply(
            StatusCode::UNAUTHORIZED,
            r#"{"error":"credentials not accepted"}"#.into(),
        )
    };
    match path.as_str() {
        "/enroll" => {
            let token = json["token"].as_str().expect("token").to_owned();
            if !st.tokens.lock().expect("tokens").remove(&token) {
                return denied();
            }
            st.enrolls.fetch_add(1, Ordering::SeqCst);
        }
        "/renew" => {
            if !has_cert {
                return denied();
            }
            st.renews.fetch_add(1, Ordering::SeqCst);
        }
        _ => return reply(StatusCode::NOT_FOUND, "{}".into()),
    }
    let (cn, cert_pem) = issuer.issue(json["csr_pem"].as_str().expect("csr_pem"));
    st.names.lock().expect("names").push(cn.clone());
    let now = SystemTime::now();
    let renew_in = Duration::from_millis(*st.renew_after_ms.lock().expect("renew_after"));
    // A renewal pushes `renew_at` far out, so the test sees one renewal.
    if path == "/renew" {
        *st.renew_after_ms.lock().expect("renew_after") = 3_600_000;
    }
    let answer = serde_json::json!({
        "agent_id": cn,
        "certificate_pem": cert_pem,
        "ca_pem": issuer.pem(),
        "not_before": rfc3339(now),
        "not_after": rfc3339(now + Duration::from_secs(7200)),
        "renew_at": rfc3339(now + renew_in),
    });
    reply(StatusCode::OK, answer.to_string())
}

/// What the fake controller saw and how it behaves.
#[derive(Default)]
struct ControllerState {
    /// Whether the peer certificate named the agent, per registration.
    registrations: Mutex<Vec<(String, bool)>>,
    sessions: AtomicU32,
    /// Answers every `Register` with `UNAUTHENTICATED`.
    unauthenticated: bool,
}

struct Fake(Arc<ControllerState>);

#[tonic::async_trait]
impl ControllerService for Fake {
    async fn register(
        &self,
        req: GrpcRequest<pb::RegisterRequest>,
    ) -> Result<Response<pb::RegisterResponse>, Status> {
        if self.0.unauthenticated {
            return Err(Status::unauthenticated("no identity"));
        }
        let named = req.peer_certs().is_some_and(|certs| {
            certs.iter().any(|c| {
                c.as_ref()
                    .windows(AGENT.len())
                    .any(|w| w == AGENT.as_bytes())
            })
        });
        let id = req.into_inner().agent_id;
        self.0
            .registrations
            .lock()
            .expect("registrations")
            .push((id.clone(), named));
        Ok(Response::new(pb::RegisterResponse {
            controller_id: "fake".into(),
            agent_id: id,
            session_token: "token".into(),
            heartbeat_interval: Some(prost_types::Duration {
                seconds: 0,
                nanos: 200_000_000,
            }),
            heartbeat_timeout: Some(prost_types::Duration {
                seconds: 60,
                nanos: 0,
            }),
        }))
    }

    async fn upload_capture(
        &self,
        _req: GrpcRequest<tonic::Streaming<pb::UploadCaptureRequest>>,
    ) -> Result<Response<pb::UploadCaptureResponse>, Status> {
        Err(Status::unimplemented(
            "the fake controller takes no uploads",
        ))
    }

    type SessionStream =
        Pin<Box<dyn Stream<Item = Result<pb::SessionResponse, Status>> + Send + 'static>>;

    async fn session(
        &self,
        req: GrpcRequest<Streaming<pb::SessionRequest>>,
    ) -> Result<Response<Self::SessionStream>, Status> {
        let mut stream = req.into_inner();
        match stream.message().await?.and_then(|m| m.body) {
            Some(session_request::Body::Open(_)) => {}
            _ => return Err(Status::unauthenticated("open first")),
        }
        self.0.sessions.fetch_add(1, Ordering::SeqCst);
        tokio::spawn(async move { while let Ok(Some(_)) = stream.message().await {} });
        Ok(Response::new(Box::pin(tokio_stream::pending())))
    }
}

/// Serves the fake controller over TLS and returns its address. Clients must
/// present a certificate of `client_ca`.
async fn controller(
    server: &Leaf,
    client_ca: &Pki,
    unauthenticated: bool,
) -> (SocketAddr, Arc<ControllerState>) {
    let state = Arc::new(ControllerState {
        unauthenticated,
        ..ControllerState::default()
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");
    let tls = ServerTlsConfig::new()
        .identity(TonicIdentity::from_pem(&server.cert_pem, &server.key_pem))
        .client_ca_root(TonicCert::from_pem(client_ca.pem()));
    let svc = ControllerServiceServer::new(Fake(Arc::clone(&state)));
    tokio::spawn(async move {
        let _ = tonic::transport::Server::builder()
            .tls_config(tls)
            .expect("tls config")
            .add_service(svc)
            .serve_with_incoming(TcpListenerStream::new(listener))
            .await;
    });
    (addr, state)
}

struct TestDir(PathBuf);

impl TestDir {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("fps-mtls-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("dir");
        Self(dir)
    }

    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn write(path: &Path, text: &str) {
    std::fs::write(path, text).expect("write");
}

/// Writes the files of an enrolled agent, valid for an hour and due for
/// renewal in a day.
fn seed_identity(dir: &Path, leaf: &Leaf) {
    std::fs::create_dir_all(dir).expect("cert dir");
    write(&dir.join("agent.crt"), &leaf.cert_pem);
    write(&dir.join("agent.key"), &leaf.key_pem);
    std::fs::set_permissions(
        dir.join("agent.key"),
        std::fs::Permissions::from_mode(0o600),
    )
    .expect("mode");
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time")
        .as_secs();
    write(
        &dir.join("identity.json"),
        &format!(
            r#"{{"not_after":{},"renew_at":{}}}"#,
            now + 3600,
            now + 86_400
        ),
    );
}

struct Agent {
    stop: Option<oneshot::Sender<()>>,
    task: tokio::task::JoinHandle<Result<(), SessionError>>,
}

impl Agent {
    fn start(controller: SocketAddr, tls: TlsConfig) -> Self {
        let mut cfg = Config::new(
            format!("https://{controller}"),
            AGENT.into(),
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../contracts/examples/modules"),
        );
        cfg.tls = Some(tls);
        cfg.redial_limit = Some(Duration::from_secs(3));
        cfg.backoff = Backoff {
            initial: Duration::from_millis(50),
            max: Duration::from_millis(400),
        };
        let (stop, rx) = oneshot::channel::<()>();
        let task = tokio::spawn(serve(cfg, async {
            let _ = rx.await;
        }));
        Self {
            stop: Some(stop),
            task,
        }
    }

    /// Waits for the agent to end on its own.
    async fn ended(self) -> Result<(), SessionError> {
        tokio::time::timeout(WAIT, self.task)
            .await
            .expect("the agent ends within the wait")
            .expect("the agent task")
    }

    async fn shutdown(mut self) {
        if let Some(s) = self.stop.take() {
            let _ = s.send(());
        }
        tokio::time::timeout(WAIT, self.task)
            .await
            .expect("the agent stops")
            .expect("the agent task")
            .expect("a clean stop");
    }
}

async fn wait_for(what: &str, mut cond: impl FnMut() -> bool) {
    let deadline = std::time::Instant::now() + WAIT;
    while !cond() {
        assert!(std::time::Instant::now() < deadline, "timed out: {what}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

fn tls(dir: &Path, enroll: SocketAddr) -> TlsConfig {
    TlsConfig {
        ca_file: None,
        ca_sha256: None,
        token: None,
        enroll_address: enroll.to_string(),
        dir: dir.to_owned(),
    }
}

fn permanent(result: Result<(), SessionError>) -> String {
    match result {
        Err(SessionError::Permanent { reason }) => reason,
        other => panic!("want a permanent failure, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn an_agent_enrolls_with_a_token_registers_and_renews() {
    let pki = Arc::new(Pki::new("test ca"));
    let enroll = enroller(&pki, Arc::clone(&pki), &["good.token"]).await;
    *enroll.state.renew_after_ms.lock().expect("lock") = 1500;
    let (addr, ctl) = controller(&pki.server(), &pki, false).await;
    let dir = TestDir::new("enroll");
    write(&dir.path("ca.crt"), &pki.pem());
    write(&dir.path("token"), "good.token\n");
    let mut cfg = tls(&dir.path("pki"), enroll.addr);
    cfg.ca_file = Some(dir.path("ca.crt"));
    cfg.token = Some(TokenSource::File(dir.path("token")));
    let agent = Agent::start(addr, cfg);

    wait_for("a session over mutual TLS", || {
        ctl.sessions.load(Ordering::SeqCst) == 1
    })
    .await;
    assert_eq!(
        *ctl.registrations.lock().expect("registrations"),
        vec![(AGENT.to_owned(), true)],
        "the controller reads the identity from the certificate"
    );
    assert_eq!(*enroll.state.names.lock().expect("names"), vec![AGENT]);
    assert!(!dir.path("token").exists(), "the token file is deleted");
    let key = dir.path("pki/agent.key");
    let mode = std::fs::metadata(&key).expect("key").permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);
    let first = std::fs::read_to_string(dir.path("pki/agent.crt")).expect("cert");

    wait_for("the renewal at renew_at", || {
        enroll.state.renews.load(Ordering::SeqCst) == 1
    })
    .await;
    wait_for("the new certificate on disk", || {
        std::fs::read_to_string(dir.path("pki/agent.crt")).is_ok_and(|c| c != first)
    })
    .await;
    // The renewal does not touch the open session.
    assert_eq!(ctl.sessions.load(Ordering::SeqCst), 1);
    assert_eq!(enroll.state.enrolls.load(Ordering::SeqCst), 1);
    agent.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_pinned_fingerprint_replaces_the_ca_file() {
    let pki = Arc::new(Pki::new("pinned ca"));
    let enroll = enroller(&pki, Arc::clone(&pki), &["good.token"]).await;
    let (addr, ctl) = controller(&pki.server(), &pki, false).await;
    let dir = TestDir::new("pin");
    let mut cfg = tls(&dir.path("pki"), enroll.addr);
    cfg.ca_sha256 = Some(pki.fingerprint());
    cfg.token = Some(TokenSource::File({
        write(&dir.path("token"), "good.token");
        dir.path("token")
    }));
    let agent = Agent::start(addr, cfg);
    wait_for("a session", || ctl.sessions.load(Ordering::SeqCst) == 1).await;
    assert_eq!(
        std::fs::read_to_string(dir.path("pki/ca.pem")).expect("the stored CA"),
        pki.pem()
    );
    agent.shutdown().await;

    // The next start needs neither the token nor the pin: it trusts the stored CA.
    let mut again = tls(&dir.path("pki"), enroll.addr);
    again.ca_sha256 = Some(pki.fingerprint());
    let agent = Agent::start(addr, again);
    wait_for("a second session", || {
        ctl.sessions.load(Ordering::SeqCst) == 2
    })
    .await;
    assert_eq!(enroll.state.enrolls.load(Ordering::SeqCst), 1);
    agent.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_new_token_file_replaces_a_certificate_that_the_controller_refuses() {
    let pki = Arc::new(Pki::new("test ca"));
    let enroll = enroller(&pki, Arc::clone(&pki), &["new.token"]).await;
    let (addr, ctl) = controller(&pki.server(), &pki, false).await;
    let dir = TestDir::new("new-token");
    write(&dir.path("ca.crt"), &pki.pem());
    // The old certificate is valid on the clock of the agent, but the
    // controller revoked it, which only the controller knows.
    let old = pki.client(AGENT);
    seed_identity(&dir.path("pki"), &old);
    write(&dir.path("token"), "new.token");
    let mut cfg = tls(&dir.path("pki"), enroll.addr);
    cfg.ca_file = Some(dir.path("ca.crt"));
    cfg.token = Some(TokenSource::File(dir.path("token")));
    let agent = Agent::start(addr, cfg);
    wait_for("a session", || ctl.sessions.load(Ordering::SeqCst) == 1).await;
    assert_eq!(enroll.state.enrolls.load(Ordering::SeqCst), 1);
    assert_ne!(
        std::fs::read_to_string(dir.path("pki/agent.crt")).expect("cert"),
        old.cert_pem
    );
    assert!(!dir.path("token").exists());
    agent.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_wrong_pin_is_permanent_and_stores_nothing() {
    let pki = Arc::new(Pki::new("real ca"));
    let enroll = enroller(&pki, Arc::clone(&pki), &["good.token"]).await;
    let (addr, _ctl) = controller(&pki.server(), &pki, false).await;
    let dir = TestDir::new("wrong-pin");
    let mut cfg = tls(&dir.path("pki"), enroll.addr);
    cfg.ca_sha256 = Some(Pki::new("other ca").fingerprint());
    write(&dir.path("token"), "good.token");
    cfg.token = Some(TokenSource::File(dir.path("token")));
    let reason = permanent(Agent::start(addr, cfg).ended().await);
    assert!(reason.contains("pinned fingerprint"), "{reason}");
    assert!(!dir.path("pki/agent.key").exists());
    assert!(!dir.path("pki/agent.crt").exists());
    assert!(
        dir.path("token").exists(),
        "the token stays for another try"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_refused_token_is_permanent() {
    let pki = Arc::new(Pki::new("test ca"));
    let enroll = enroller(&pki, Arc::clone(&pki), &["good.token"]).await;
    let (addr, _ctl) = controller(&pki.server(), &pki, false).await;
    let dir = TestDir::new("refused");
    write(&dir.path("ca.crt"), &pki.pem());
    write(&dir.path("token"), "used.token");
    let mut cfg = tls(&dir.path("pki"), enroll.addr);
    cfg.ca_file = Some(dir.path("ca.crt"));
    cfg.token = Some(TokenSource::File(dir.path("token")));
    let reason = permanent(Agent::start(addr, cfg).ended().await);
    assert!(reason.contains("refused the enrollment token"), "{reason}");
    assert!(!dir.path("pki/agent.key").exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn no_certificate_and_no_token_is_permanent() {
    let pki = Arc::new(Pki::new("test ca"));
    let enroll = enroller(&pki, Arc::clone(&pki), &[]).await;
    let (addr, _ctl) = controller(&pki.server(), &pki, false).await;
    let dir = TestDir::new("no-token");
    write(&dir.path("ca.crt"), &pki.pem());
    let mut cfg = tls(&dir.path("pki"), enroll.addr);
    cfg.ca_file = Some(dir.path("ca.crt"));
    cfg.token = Some(TokenSource::Env("FPS_TEST_NO_SUCH_TOKEN".into()));
    let reason = permanent(Agent::start(addr, cfg).ended().await);
    assert!(reason.contains("no enrollment token"), "{reason}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_controller_of_another_ca_is_a_permanent_handshake_failure() {
    let agent_ca = Arc::new(Pki::new("agent ca"));
    let rogue = Arc::new(Pki::new("rogue ca"));
    let enroll = enroller(&agent_ca, Arc::clone(&agent_ca), &[]).await;
    // The server certificate comes from the rogue CA.
    let (addr, ctl) = controller(&rogue.server(), &agent_ca, false).await;
    let dir = TestDir::new("rogue-server");
    write(&dir.path("ca.crt"), &agent_ca.pem());
    seed_identity(&dir.path("pki"), &agent_ca.client(AGENT));
    let mut cfg = tls(&dir.path("pki"), enroll.addr);
    cfg.ca_file = Some(dir.path("ca.crt"));
    let reason = permanent(Agent::start(addr, cfg).ended().await);
    assert!(reason.contains("TLS failure"), "{reason}");
    assert!(reason.contains("UnknownIssuer"), "{reason}");
    assert_eq!(ctl.sessions.load(Ordering::SeqCst), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_certificate_that_the_controller_refuses_is_a_permanent_failure() {
    let pki = Arc::new(Pki::new("test ca"));
    let rogue = Pki::new("rogue ca");
    let enroll = enroller(&pki, Arc::clone(&pki), &[]).await;
    let (addr, ctl) = controller(&pki.server(), &pki, false).await;
    let dir = TestDir::new("rogue-client");
    write(&dir.path("ca.crt"), &pki.pem());
    // The agent holds a certificate that the controller CA did not issue.
    seed_identity(&dir.path("pki"), &rogue.client(AGENT));
    let mut cfg = tls(&dir.path("pki"), enroll.addr);
    cfg.ca_file = Some(dir.path("ca.crt"));
    let reason = permanent(Agent::start(addr, cfg).ended().await);
    assert!(reason.contains("TLS failure"), "{reason}");
    assert_eq!(ctl.sessions.load(Ordering::SeqCst), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn unauthenticated_is_permanent() {
    let pki = Arc::new(Pki::new("test ca"));
    let enroll = enroller(&pki, Arc::clone(&pki), &[]).await;
    let (addr, _ctl) = controller(&pki.server(), &pki, true).await;
    let dir = TestDir::new("unauthenticated");
    write(&dir.path("ca.crt"), &pki.pem());
    seed_identity(&dir.path("pki"), &pki.client(AGENT));
    let mut cfg = tls(&dir.path("pki"), enroll.addr);
    cfg.ca_file = Some(dir.path("ca.crt"));
    let reason = permanent(Agent::start(addr, cfg).ended().await);
    assert!(reason.contains("UNAUTHENTICATED"), "{reason}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_peer_that_does_not_speak_tls_is_a_permanent_failure() {
    let pki = Arc::new(Pki::new("test ca"));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(async move {
        use tokio::io::AsyncWriteExt as _;
        while let Ok((mut s, _)) = listener.accept().await {
            let _ = s
                .write_all(b"HTTP/1.1 200 OK\r\n\r\nnot tls at all\r\n")
                .await;
        }
    });
    let dir = TestDir::new("plain-peer");
    write(&dir.path("ca.crt"), &pki.pem());
    seed_identity(&dir.path("pki"), &pki.client(AGENT));
    let mut cfg = tls(&dir.path("pki"), addr);
    cfg.ca_file = Some(dir.path("ca.crt"));
    let reason = permanent(Agent::start(addr, cfg).ended().await);
    assert!(reason.contains("TLS failure"), "{reason}");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unreachable_controller_is_not_permanent() {
    let pki = Arc::new(Pki::new("test ca"));
    let dir = TestDir::new("unreachable");
    write(&dir.path("ca.crt"), &pki.pem());
    seed_identity(&dir.path("pki"), &pki.client(AGENT));
    // Nothing listens on port 1, so the dials fail until the limit.
    let addr: SocketAddr = "127.0.0.1:1".parse().expect("addr");
    let mut cfg = tls(&dir.path("pki"), addr);
    cfg.ca_file = Some(dir.path("ca.crt"));
    let mut full = Config::new(
        format!("https://{addr}"),
        AGENT.into(),
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../contracts/examples/modules"),
    );
    full.tls = Some(cfg);
    full.redial_limit = Some(Duration::from_millis(300));
    full.backoff = Backoff {
        initial: Duration::from_millis(50),
        max: Duration::from_millis(100),
    };
    let result = serve(full, std::future::pending()).await;
    assert!(
        matches!(result, Err(SessionError::RedialLimit { .. })),
        "{result:?}"
    );
}

#[tokio::test]
async fn the_scheme_and_the_tls_settings_must_agree() {
    let dir = TestDir::new("scheme");
    let modules =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../contracts/examples/modules");
    let https = Config::new("https://127.0.0.1:1".into(), AGENT.into(), modules.clone());
    assert!(matches!(
        serve(https, std::future::pending()).await,
        Err(SessionError::Setup(_))
    ));
    let mut http = Config::new("http://127.0.0.1:1".into(), AGENT.into(), modules);
    http.tls = Some(tls(&dir.path("pki"), "127.0.0.1:2".parse().expect("addr")));
    assert!(matches!(
        serve(http, std::future::pending()).await,
        Err(SessionError::Setup(_))
    ));
}

fn connect(args: &[&str]) -> std::process::Output {
    let modules =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../contracts/examples/modules");
    Command::new(env!("CARGO_BIN_EXE_fps-agent"))
        .arg("connect")
        .args(["--agent-id", AGENT, "--modules"])
        .arg(modules)
        .args(args)
        .output()
        .expect("run fps-agent")
}

#[test]
fn the_binary_ends_with_exit_code_2_and_a_clear_error_on_a_handshake_failure() {
    if fps_agent::require_kernel(&fps_agent::SystemRoot::host()).is_err() {
        return;
    }
    let pki = Pki::new("test ca");
    let dir = TestDir::new("binary");
    write(&dir.path("ca.crt"), &pki.pem());
    seed_identity(&dir.path("pki"), &pki.client(AGENT));
    // A server that answers with plain text, where TLS is due.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("addr");
    std::thread::spawn(move || {
        use std::io::Write as _;
        while let Ok((mut s, _)) = listener.accept() {
            let _ = s.write_all(b"HTTP/1.1 200 OK\r\n\r\nplain\r\n");
        }
    });
    let out = connect(&[
        "--controller",
        &format!("https://{addr}"),
        "--cert-dir",
        &dir.path("pki").to_string_lossy(),
        "--tls-ca",
        &dir.path("ca.crt").to_string_lossy(),
    ]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(2), "{stderr}");
    assert!(stderr.contains("fps-agent: error:"), "{stderr}");
    assert!(stderr.contains("TLS failure"), "{stderr}");
}

#[test]
fn the_tls_flags_are_checked_against_the_scheme() {
    let out = connect(&["--controller", "https://127.0.0.1:1"]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(2), "{stderr}");
    assert!(stderr.contains("--cert-dir"), "{stderr}");

    let out = connect(&[
        "--controller",
        "https://127.0.0.1:1",
        "--cert-dir",
        "/nonexistent/fps-pki",
    ]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(2), "{stderr}");
    assert!(stderr.contains("--tls-ca"), "{stderr}");

    let out = connect(&[
        "--controller",
        "http://127.0.0.1:1",
        "--tls-ca",
        "/nonexistent/ca.pem",
    ]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(2), "{stderr}");
    assert!(stderr.contains("insecure_plain_grpc"), "{stderr}");

    let out = connect(&[
        "--controller",
        "https://127.0.0.1:1",
        "--cert-dir",
        "/nonexistent/fps-pki",
        "--tls-ca-sha256",
        "abcd",
    ]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(2), "{stderr}");
    assert!(stderr.contains("fingerprint"), "{stderr}");
}
