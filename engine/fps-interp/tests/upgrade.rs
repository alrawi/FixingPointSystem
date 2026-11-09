// SPDX-License-Identifier: GPL-3.0-only
//! The `tls_upgrade` action with a stand in upgrader, which wraps the
//! transport in a byte mask instead of TLS. The tests check the swap in place,
//! the roles, and the failure reasons. The runtime tests run real TLS.

mod common;

use std::io;
use std::pin::Pin;
use std::sync::Mutex;
use std::task::{Context, Poll};

use common::{compile, Tap};
use fps_flow::function::NoFunctions;
use fps_flow::program::{Op, TlsRole};
use fps_flow::template::Filler;
use fps_interp::{
    session_rng, BoxedIo, Failure, Reason, Session, SessionCtx, Side, TlsUpgrader, UpgradeError,
    UpgradeFuture, UpgradeRequest,
};
use fps_io::memory::duplex;
use fps_io::SessionIo;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

const MASK: u8 = 0x5a;

/// Masks every byte that crosses it, in both directions.
struct Masked<IO>(IO);

impl<IO: AsyncRead + Unpin> AsyncRead for Masked<IO> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let before = buf.filled().len();
        let r = Pin::new(&mut self.0).poll_read(cx, buf);
        if let Poll::Ready(Ok(())) = &r {
            for b in &mut buf.filled_mut()[before..] {
                *b ^= MASK;
            }
        }
        r
    }
}

impl<IO: AsyncWrite + Unpin> AsyncWrite for Masked<IO> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        data: &[u8],
    ) -> Poll<io::Result<usize>> {
        let masked: Vec<u8> = data.iter().map(|b| b ^ MASK).collect();
        Pin::new(&mut self.0).poll_write(cx, &masked)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.0).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.0).poll_shutdown(cx)
    }
}

impl<IO: SessionIo> SessionIo for Masked<IO> {
    fn abort(&mut self) {
        self.0.abort();
    }
}

/// Wraps the transport in [`Masked`] and records the requests it saw.
#[derive(Default)]
struct MaskUpgrader {
    seen: Mutex<Vec<(TlsRole, String, Option<String>)>>,
    fail: Option<io::ErrorKind>,
    unknown: bool,
}

impl TlsUpgrader for MaskUpgrader {
    fn upgrade<'io>(&self, io: BoxedIo<'io>, request: UpgradeRequest<'_>) -> UpgradeFuture<'io> {
        self.seen.lock().unwrap().push((
            request.role,
            request.profile.to_owned(),
            request.server_name.map(str::to_owned),
        ));
        let result: Result<BoxedIo<'io>, UpgradeError> = if self.unknown {
            Err(UpgradeError::Unavailable("no such profile".into()))
        } else if let Some(kind) = self.fail {
            Err(UpgradeError::Handshake(io::Error::new(kind, "bad hello")))
        } else {
            Ok(Box::new(Masked(io)))
        };
        Box::pin(std::future::ready(result))
    }
}

const STARTTLS: &str = r#"
scenario: starttls
flows:
  - id: s
    module: m
    client:
      - connect: { port: 25 }
      - send: "STARTTLS\r\n"
      - expect: { pattern: "220 go\r\n", timeout_ms: 1000 }
      - tls_upgrade: { profile: mail, server_name: mx.example.test }
      - send: "secret one\r\n"
      - expect: { pattern: "250 done\r\n", timeout_ms: 1000 }
      - close: fin
    server:
      - accept: { port: 25 }
      - expect: { pattern: "STARTTLS\r\n", timeout_ms: 1000 }
      - send: "220 go\r\n"
      - tls_upgrade: { profile: mail }
      - expect: { pattern: "secret one\r\n", timeout_ms: 1000 }
      - send: "250 done\r\n"
      - close: fin
"#;

async fn run_side(
    scenario: &fps_flow::Scenario,
    side: Side,
    io: &mut impl SessionIo,
    upgrader: Option<&dyn TlsUpgrader>,
) -> Result<(), Failure> {
    let flow = &scenario.flows[0];
    let program = match side {
        Side::Client => &flow.client,
        Side::Server => flow.server.as_ref().unwrap(),
    };
    let filler = Filler::new(1);
    let mut functions = NoFunctions;
    let mut session = Session::new(SessionCtx {
        vars: &scenario.vars,
        filler: &filler,
        flow: "s",
        side,
        session_id: 1,
        rng: session_rng(1, 1, side),
        functions: &mut functions,
        limits: fps_interp::Limits::default(),
    });
    if let Some(u) = upgrader {
        session.set_tls_upgrader(u);
    }
    session.run(io, &program.ops, std::future::pending()).await
}

#[tokio::test(start_paused = true)]
async fn the_second_half_runs_over_the_upgraded_transport() {
    let s = compile(STARTTLS);
    let (a, b) = duplex(4096);
    let (mut a, wire) = Tap::new(a);
    let (client_up, server_up) = (MaskUpgrader::default(), MaskUpgrader::default());
    let (c, v) = tokio::join!(
        run_side(&s, Side::Client, &mut a, Some(&client_up)),
        async {
            let mut b = b;
            run_side(&s, Side::Server, &mut b, Some(&server_up)).await
        }
    );
    c.unwrap();
    v.unwrap();
    // The first half crosses in the clear, the second masked.
    let wire = wire.lock().unwrap().clone();
    let text = String::from_utf8_lossy(&wire);
    assert!(text.starts_with("STARTTLS\r\n"), "{text:?}");
    assert!(!text.contains("secret"), "{text:?}");
    let masked: Vec<u8> = b"secret one\r\n".iter().map(|b| b ^ MASK).collect();
    assert!(wire.ends_with(&masked));
    // Each side took the role of its side, and the client named the server.
    assert_eq!(
        *client_up.seen.lock().unwrap(),
        vec![(
            TlsRole::Client,
            "mail".into(),
            Some("mx.example.test".into())
        )]
    );
    assert_eq!(
        *server_up.seen.lock().unwrap(),
        vec![(TlsRole::Server, "mail".into(), None)]
    );
}

#[tokio::test(start_paused = true)]
async fn an_explicit_role_overrides_the_side() {
    let s = compile(&STARTTLS.replace(
        "tls_upgrade: { profile: mail }",
        "tls_upgrade: { profile: mail, role: client }",
    ));
    assert!(matches!(
        s.flows[0].server.as_ref().unwrap().ops[2],
        Op::UpgradeTls(ref u) if u.role == Some(TlsRole::Client)
    ));
}

#[tokio::test(start_paused = true)]
async fn a_failed_handshake_is_a_tls_failure() {
    let s = compile(STARTTLS);
    let (mut a, mut b) = duplex(4096);
    let failing = MaskUpgrader {
        fail: Some(io::ErrorKind::InvalidData),
        ..MaskUpgrader::default()
    };
    let ok = MaskUpgrader::default();
    let (c, _v) = tokio::join!(
        run_side(&s, Side::Client, &mut a, Some(&failing)),
        run_side(&s, Side::Server, &mut b, Some(&ok)),
    );
    let f = c.unwrap_err();
    assert_eq!(f.reason, Reason::TlsFailure, "{f}");
    assert_eq!(f.action_type, "tls_upgrade");
    assert_eq!(f.action_index, 3);
}

#[tokio::test(start_paused = true)]
async fn an_unknown_profile_is_an_internal_failure() {
    let s = compile(STARTTLS);
    let (mut a, mut b) = duplex(4096);
    let unknown = MaskUpgrader {
        unknown: true,
        ..MaskUpgrader::default()
    };
    let (c, _v) = tokio::join!(
        run_side(&s, Side::Client, &mut a, Some(&unknown)),
        run_side(&s, Side::Server, &mut b, None),
    );
    let f = c.unwrap_err();
    assert_eq!(f.reason, Reason::Internal, "{f}");
    assert_eq!(f.action_type, "tls_upgrade");
}

#[tokio::test(start_paused = true)]
async fn a_session_without_an_upgrader_fails_internal() {
    let s = compile(STARTTLS);
    let (mut a, mut b) = duplex(4096);
    let (c, v) = tokio::join!(
        run_side(&s, Side::Client, &mut a, None),
        run_side(&s, Side::Server, &mut b, None),
    );
    assert_eq!(c.unwrap_err().reason, Reason::Internal);
    assert_eq!(v.unwrap_err().reason, Reason::Internal);
}

#[tokio::test(start_paused = true)]
async fn bytes_buffered_before_the_handshake_fail_the_upgrade() {
    // The server sends more than the client expects, so the surplus sits in
    // the read buffer when the client upgrades.
    let text = STARTTLS.replace("- send: \"220 go\\r\\n\"", "- send: \"220 go\\r\\nearly\"");
    let s = compile(&text);
    let (mut a, mut b) = duplex(4096);
    let (up_a, up_b) = (MaskUpgrader::default(), MaskUpgrader::default());
    let (c, _v) = tokio::join!(
        run_side(&s, Side::Client, &mut a, Some(&up_a)),
        run_side(&s, Side::Server, &mut b, Some(&up_b)),
    );
    let f = c.unwrap_err();
    assert_eq!(f.reason, Reason::Internal, "{f}");
    assert!(up_a.seen.lock().unwrap().is_empty());
}
