// SPDX-License-Identifier: GPL-3.0-only
//! Tests of the state machine, with a client and a server that exchange the
//! bytes of their calls directly. The scenario tests in `modules/ssh/tests`
//! run the same functions through the interpreter.

use fps_flow::function::NamedArg;
use fps_flow::{ArgValue, CallArgs, FnCall, FnError, FnErrorKind, TemplateFn};
use rand::{RngCore, SeedableRng};
use rand_pcg::Pcg64Mcg;

use crate::transport::Half;
use crate::wire::{put_string, Reader};
use crate::Ssh;

struct Peer {
    ssh: Ssh,
    rng: Pcg64Mcg,
}

impl Peer {
    fn new(seed: u64) -> Peer {
        Peer {
            ssh: Ssh::default(),
            rng: Pcg64Mcg::seed_from_u64(seed),
        }
    }

    fn try_call(
        &mut self,
        function: &str,
        positional: &[ArgValue<'_>],
    ) -> Result<Vec<u8>, FnError> {
        let named: [NamedArg<'_>; 0] = [];
        let call = FnCall {
            module: "ssh",
            function,
            args: CallArgs {
                positional,
                named: &named,
            },
        };
        let mut out = Vec::new();
        self.ssh.render(&call, &mut self.rng, &mut out)?;
        Ok(out)
    }

    fn call(&mut self, function: &str, positional: &[ArgValue<'_>]) -> Vec<u8> {
        self.try_call(function, positional)
            .unwrap_or_else(|e| panic!("{function}: {e}"))
    }

    /// Reads one packet from the front of `wire` the way a scenario does:
    /// 4 bytes, `size`, the rest, `recv`. Returns the value of `recv`.
    fn try_receive(&mut self, wire: &mut Vec<u8>, expect: &str) -> Result<usize, FnError> {
        let head: Vec<u8> = wire.drain(..4).collect();
        let n: usize = String::from_utf8(self.try_call("size", &[ArgValue::Bytes(&head)])?)
            .unwrap()
            .parse()
            .unwrap();
        let body: Vec<u8> = wire.drain(..n.min(wire.len())).collect();
        let r = self.try_call("recv", &[ArgValue::Bytes(&body), ArgValue::Text(expect)])?;
        Ok(String::from_utf8(r).unwrap().parse().unwrap())
    }

    fn receive(&mut self, wire: &mut Vec<u8>, expect: &str) -> usize {
        self.try_receive(wire, expect)
            .unwrap_or_else(|e| panic!("recv {expect}: {e}"))
    }
}

/// The two sides after the identification lines, the two `SSH_MSG_KEXINIT`
/// messages, and `SSH_MSG_KEX_ECDH_INIT`, and the reply of the server, which
/// nobody has delivered yet.
struct Midway {
    client: Peer,
    server: Peer,
    reply: Vec<u8>,
    c2s: Vec<u8>,
    s2c: Vec<u8>,
}

fn midway(client_seed: u64, server_seed: u64, host_seed: i64) -> Midway {
    let (mut client, mut server) = (Peer::new(client_seed), Peer::new(server_seed));
    let cb = client.call(
        "banner",
        &[ArgValue::Text("OpenSSH_9.6p1 Ubuntu-3ubuntu13.5")],
    );
    let sb = server.call("banner", &[ArgValue::Text("OpenSSH_9.7")]);
    assert_eq!(
        client.call("peer_banner", &[ArgValue::Bytes(&sb)]),
        b"OpenSSH_9.7"
    );
    assert_eq!(
        server.call("peer_banner", &[ArgValue::Bytes(&cb)]),
        b"OpenSSH_9.6p1 Ubuntu-3ubuntu13.5"
    );
    let (mut c2s, mut s2c) = (cb, sb);
    let ki_c = client.call("kexinit", &[]);
    let ki_s = server.call("kexinit", &[]);
    c2s.extend(&ki_c);
    s2c.extend(&ki_s);
    server.receive(&mut ki_c.clone(), "kexinit");
    client.receive(&mut ki_s.clone(), "kexinit");
    let init = client.call("ecdh_init", &[]);
    c2s.extend(&init);
    server.receive(&mut init.clone(), "ecdh_init");
    let reply = server.call("ecdh_reply", &[ArgValue::Int(host_seed)]);
    Midway {
        client,
        server,
        reply,
        c2s,
        s2c,
    }
}

/// Finishes the key exchange. Returns the two sides and the bytes that each
/// of them wrote.
fn handshake(client_seed: u64, server_seed: u64, host_seed: i64) -> (Peer, Peer, Vec<u8>, Vec<u8>) {
    let Midway {
        mut client,
        mut server,
        reply,
        mut c2s,
        mut s2c,
    } = midway(client_seed, server_seed, host_seed);
    let nk_s = server.call("newkeys", &[]);
    s2c.extend(&reply);
    s2c.extend(&nk_s);
    let mut to_client = reply;
    to_client.extend(nk_s);
    client.receive(&mut to_client, "ecdh_reply");
    client.receive(&mut to_client, "newkeys");
    assert!(to_client.is_empty());
    let nk_c = client.call("newkeys", &[]);
    c2s.extend(&nk_c);
    server.receive(&mut nk_c.clone(), "newkeys");
    (client, server, c2s, s2c)
}

fn pair() -> (Peer, Peer) {
    let (c, s, _, _) = handshake(1, 2, 1);
    (c, s)
}

/// The payloads of the clear packets in `wire`, which starts with the
/// identification line.
fn plain_payloads(wire: &[u8]) -> Vec<Vec<u8>> {
    let mut rest = &wire[wire.iter().position(|b| *b == b'\n').unwrap() + 1..];
    let mut out = Vec::new();
    while !rest.is_empty() {
        let len = u32::from_be_bytes(rest[..4].try_into().unwrap()) as usize;
        let pad = usize::from(rest[4]);
        out.push(rest[5..4 + len - pad].to_vec());
        rest = &rest[4 + len..];
    }
    out
}

/// Splits a `SSH_MSG_KEX_ECDH_REPLY` payload into the host key, the server
/// key, and the signature.
fn parse_reply(payload: &[u8]) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
    assert_eq!(payload[0], 31);
    let mut r = Reader::new(&payload[1..]);
    let k_s = r.string("k_s").unwrap().to_vec();
    let q_s = r.string("q_s").unwrap().to_vec();
    let sig = r.string("sig").unwrap().to_vec();
    assert!(r.rest().is_empty());
    (k_s, q_s, sig)
}

/// Builds a clear packet for an altered reply.
fn build_reply(k_s: &[u8], q_s: &[u8], sig: &[u8]) -> Vec<u8> {
    let mut p = vec![31];
    put_string(&mut p, k_s);
    put_string(&mut p, q_s);
    put_string(&mut p, sig);
    let mut out = Vec::new();
    Half::new().seal(&p, &mut Pcg64Mcg::seed_from_u64(0), &mut out);
    out
}

#[test]
fn both_sides_derive_the_same_keys() {
    let (mut c, mut s) = pair();
    let mut w = c.call("service_request", &[]);
    assert_eq!(s.receive(&mut w, "service_request"), 0);
    let mut w = s.call("service_accept", &[]);
    assert_eq!(c.receive(&mut w, "service_accept"), 0);
    // The packets are encrypted: the service name does not show on the wire.
    let w = c.call("service_request", &[]);
    assert!(!w.windows(8).any(|x| x == b"ssh-user"));
}

#[test]
fn the_plain_part_of_the_wire_is_a_real_key_exchange() {
    let (_, _, c2s, s2c) = handshake(1, 2, 1);
    assert!(c2s.starts_with(b"SSH-2.0-OpenSSH_9.6p1 Ubuntu-3ubuntu13.5\r\n"));
    assert!(s2c.starts_with(b"SSH-2.0-OpenSSH_9.7\r\n"));
    let client: Vec<u8> = plain_payloads(&c2s).iter().map(|p| p[0]).collect();
    let server: Vec<u8> = plain_payloads(&s2c).iter().map(|p| p[0]).collect();
    // KEXINIT, KEX_ECDH_INIT, NEWKEYS and KEXINIT, KEX_ECDH_REPLY, NEWKEYS.
    assert_eq!(client, [20, 30, 21]);
    assert_eq!(server, [20, 31, 21]);
    let (k_s, q_s, sig) = parse_reply(&plain_payloads(&s2c)[1]);
    // The key blob names ssh-ed25519 and holds 32 key bytes, and so does the
    // signature blob with 64 signature bytes.
    assert_eq!(&k_s[4..15], b"ssh-ed25519");
    assert_eq!(k_s.len(), 4 + 11 + 4 + 32);
    assert_eq!(q_s.len(), 32);
    assert_eq!(&sig[4..15], b"ssh-ed25519");
    assert_eq!(sig.len(), 4 + 11 + 4 + 64);
}

fn host_key_blob(s2c: &[u8]) -> Vec<u8> {
    parse_reply(&plain_payloads(s2c)[1]).0
}

#[test]
fn a_host_key_seed_gives_one_stable_host_key() {
    let blob = |client_seed, server_seed, host| {
        let (_, _, _, s2c) = handshake(client_seed, server_seed, host);
        host_key_blob(&s2c)
    };
    let a = blob(1, 2, 7);
    assert_eq!(a, blob(3, 4, 7), "the session does not change the host key");
    assert_ne!(a, blob(1, 2, 8), "another seed is another server");
}

#[test]
fn every_session_draws_its_own_ephemeral_keys() {
    let wire = |cs, ss| handshake(cs, ss, 1).2;
    assert_ne!(wire(1, 2), wire(3, 2));
    assert_eq!(wire(1, 2), wire(1, 2));
}

#[test]
fn a_forged_signature_fails_the_client() {
    let pad = usize::from(midway(1, 2, 1).reply[4]);
    // The clear packet ends with the padding, and the signature ends before it.
    for from_end in [pad + 1, pad + 30] {
        let mut m = midway(1, 2, 1);
        let at = m.reply.len() - from_end;
        m.reply[at] ^= 0x01;
        let err = m.client.try_receive(&mut m.reply, "ecdh_reply").unwrap_err();
        assert_eq!(err.kind, FnErrorKind::Protocol);
        assert!(err.message.contains("does not verify"), "{err}");
    }
    // The untouched reply verifies.
    let mut m = midway(1, 2, 1);
    m.client.receive(&mut m.reply, "ecdh_reply");
}

#[test]
fn a_swapped_host_key_fails_the_client() {
    // A man in the middle that keeps the signature of server A but offers the
    // key of server B cannot make the signature verify.
    let a = midway(1, 2, 1);
    let b = midway(1, 2, 2);
    let (_, q_s, sig) = parse_reply(&a.reply_payload());
    let (other_key, _, _) = parse_reply(&b.reply_payload());
    let mut forged = build_reply(&other_key, &q_s, &sig);
    let mut a = a;
    let err = a.client.try_receive(&mut forged, "ecdh_reply").unwrap_err();
    assert!(err.message.contains("does not verify"), "{err}");
}

impl Midway {
    fn reply_payload(&self) -> Vec<u8> {
        let mut wire = b"x\n".to_vec();
        wire.extend(&self.reply);
        plain_payloads(&wire).remove(0)
    }
}

#[test]
fn a_key_that_is_not_ed25519_fails_the_client() {
    let m = midway(1, 2, 1);
    let (_, q_s, sig) = parse_reply(&m.reply_payload());
    let mut blob = Vec::new();
    put_string(&mut blob, b"ssh-rsa");
    put_string(&mut blob, &[1, 2, 3]);
    let mut forged = build_reply(&blob, &q_s, &sig);
    let mut m = m;
    assert!(m.client.try_receive(&mut forged, "ecdh_reply").is_err());
}

#[test]
fn a_flipped_bit_in_an_encrypted_packet_fails_the_receiver() {
    let (mut c, mut s) = pair();
    let mut w = c.call("channel_data", &[ArgValue::Bytes(b"x")]);
    let at = w.len() - 20;
    w[at] ^= 0x80;
    let err = s.try_receive(&mut w, "channel_data").unwrap_err();
    assert_eq!(err.kind, FnErrorKind::Protocol);
    assert!(err.message.contains("tag"), "{err}");
}

#[test]
fn replayed_packets_fail_because_the_sequence_number_is_in_the_nonce() {
    let (mut c, mut s) = pair();
    let w = c.call("channel_data", &[ArgValue::Bytes(b"x")]);
    let mut first = w.clone();
    s.receive(&mut first, "channel_data");
    let mut again = w;
    assert!(s.try_receive(&mut again, "channel_data").is_err());
}

#[test]
fn a_wrong_key_fails_the_receiver() {
    // A session with other keys cannot read the packets of this one.
    let (mut c, _) = pair();
    let (_, mut other) = {
        let (c2, s2, _, _) = handshake(5, 6, 1);
        (c2, s2)
    };
    let mut w = c.call("channel_data", &[ArgValue::Bytes(b"x")]);
    assert!(other.try_receive(&mut w, "channel_data").is_err());
}

#[test]
fn recv_checks_the_message_that_the_scenario_expects() {
    let (mut c, mut s) = pair();
    let mut w = c.call("channel_eof", &[]);
    let err = s.try_receive(&mut w, "channel_close").unwrap_err();
    assert!(
        err.message
            .contains("expected channel_close, received channel_eof"),
        "{err}"
    );
}

#[test]
fn recv_returns_the_data_length_and_checks_the_channel() {
    let (mut c, mut s) = pair();
    for n in [1usize, 77, 5000, 32768] {
        let data = vec![b'q'; n];
        let mut w = c.call("channel_data", &[ArgValue::Bytes(&data)]);
        assert_eq!(s.receive(&mut w, "channel_data"), n);
        assert!(w.is_empty());
    }
    let mut w = s.call("window_adjust", &[ArgValue::Int(4096)]);
    assert_eq!(c.receive(&mut w, "window_adjust"), 0);
}

#[test]
fn every_message_of_a_session_round_trips() {
    let (mut c, mut s) = pair();
    let user = c.call("credential", &[ArgValue::Text("user")]);
    let user = std::str::from_utf8(&user).unwrap();
    let pw = c.call("credential", &[ArgValue::Text("password")]);
    let to_s: Vec<(Vec<u8>, &str)> = vec![
        (c.call("service_request", &[]), "service_request"),
        (c.call("auth_none", &[ArgValue::Text(user)]), "userauth_request"),
        (
            c.call("auth_password", &[ArgValue::Text(user), ArgValue::Bytes(&pw)]),
            "userauth_request",
        ),
        (c.call("channel_open", &[ArgValue::Int(2_097_152), ArgValue::Int(32_768)]), "channel_open"),
        (c.call("channel_request", &[ArgValue::Text("pty-req")]), "channel_request"),
        (c.call("channel_request", &[ArgValue::Text("shell")]), "channel_request"),
        (
            c.call(
                "channel_request",
                &[ArgValue::Text("exec"), ArgValue::Bytes(b"scp -t /srv/upload")],
            ),
            "channel_request",
        ),
        (c.call("channel_eof", &[]), "channel_eof"),
        (c.call("channel_close", &[]), "channel_close"),
        (c.call("disconnect", &[]), "disconnect"),
    ];
    for (mut w, name) in to_s {
        s.receive(&mut w, name);
        assert!(w.is_empty(), "{name}");
    }
    let to_c: Vec<(Vec<u8>, &str)> = vec![
        (s.call("service_accept", &[]), "service_accept"),
        (s.call("auth_failure", &[]), "userauth_failure"),
        (s.call("auth_success", &[]), "userauth_success"),
        (s.call("channel_confirm", &[ArgValue::Int(2_097_152), ArgValue::Int(32_768)]), "channel_confirm"),
        (s.call("channel_success", &[]), "channel_success"),
        (s.call("exit_status", &[ArgValue::Int(0)]), "channel_request"),
        (s.call("channel_eof", &[]), "channel_eof"),
        (s.call("channel_close", &[]), "channel_close"),
    ];
    for (mut w, name) in to_c {
        c.receive(&mut w, name);
        assert!(w.is_empty(), "{name}");
    }
}

#[test]
fn functions_called_in_the_wrong_order_fail_without_panicking() {
    let mut p = Peer::new(1);
    for f in ["ecdh_reply", "newkeys"] {
        let err = p.try_call(f, &[]).unwrap_err();
        assert_eq!(err.kind, FnErrorKind::Protocol, "{f}");
    }
    let err = p.try_call("recv", &[ArgValue::Bytes(b"x")]).unwrap_err();
    assert_eq!(err.kind, FnErrorKind::Protocol);
    let err = p.try_call("size", &[ArgValue::Bytes(b"xy")]).unwrap_err();
    assert_eq!(err.kind, FnErrorKind::Protocol);
    let err = p
        .try_call("peer_banner", &[ArgValue::Bytes(b"HTTP/1.1 200 OK\r\n")])
        .unwrap_err();
    assert_eq!(err.kind, FnErrorKind::Protocol);
    let err = p.try_call("nonsense", &[]).unwrap_err();
    assert_eq!(err.kind, FnErrorKind::Unsupported);
}

#[test]
fn hostile_bytes_never_panic() {
    // Feed every suffix of a real exchange and random bytes into fresh sides.
    // Each call returns a value or an error.
    let (_, _, c2s, s2c) = handshake(1, 2, 1);
    for wire in [&c2s, &s2c] {
        let body = &wire[wire.iter().position(|b| *b == b'\n').unwrap() + 1..];
        for cut in 0..body.len().min(500) {
            for peer_seed in [1u64, 2] {
                let mut p = Peer::new(peer_seed);
                let mut junk = body[cut..].to_vec();
                if junk.len() >= 4 {
                    let _ = p.try_receive(&mut junk, "any");
                }
            }
        }
    }
    let mut rng = Pcg64Mcg::seed_from_u64(9);
    for _ in 0..300 {
        let mut raw = vec![0u8; 64];
        rng.fill_bytes(&mut raw);
        let mut p = Peer::new(3);
        let _ = p.try_receive(&mut raw, "any");
    }
}

#[test]
fn chunk_div_rem_and_flag_do_what_the_scenarios_need() {
    let mut p = Peer::new(1);
    let num = |p: &mut Peer, f: &str, a: &[ArgValue<'_>]| -> i64 {
        String::from_utf8(p.call(f, a)).unwrap().parse().unwrap()
    };
    assert_eq!(
        num(&mut p, "div", &[ArgValue::Int(262_144), ArgValue::Int(32_768)]),
        8
    );
    assert_eq!(
        num(&mut p, "rem", &[ArgValue::Int(262_145), ArgValue::Int(32_768)]),
        1
    );
    assert_eq!(num(&mut p, "flag", &[ArgValue::Int(0)]), 0);
    assert_eq!(num(&mut p, "flag", &[ArgValue::Int(5)]), 1);
    assert!(p
        .try_call("div", &[ArgValue::Int(1), ArgValue::Int(0)])
        .is_err());
    for kind in ["text", "keys", "binary"] {
        for n in [0, 1, 1000, 32768] {
            assert_eq!(
                p.call("chunk", &[ArgValue::Int(n), ArgValue::Text(kind)])
                    .len(),
                n as usize
            );
        }
    }
    assert!(p.try_call("chunk", &[ArgValue::Int(32769)]).is_err());
    assert!(p
        .try_call("chunk", &[ArgValue::Int(1), ArgValue::Text("x")])
        .is_err());
    assert_eq!(
        p.call("scp_header", &[ArgValue::Int(12), ArgValue::Text("a.bin")]),
        b"C0644 12 a.bin\n"
    );
    assert!(p
        .try_call("scp_header", &[ArgValue::Int(12), ArgValue::Text("a/b")])
        .is_err());
}

#[test]
fn the_banner_rules_follow_rfc_4253() {
    let mut p = Peer::new(1);
    assert_eq!(
        p.call("banner", &[ArgValue::Text("OpenSSH_9.6p1 Ubuntu")]),
        b"SSH-2.0-OpenSSH_9.6p1 Ubuntu\r\n"
    );
    for bad in ["", "a-b", " x", "x\ny"] {
        assert!(
            p.try_call("banner", &[ArgValue::Text(bad)]).is_err(),
            "{bad:?}"
        );
    }
    // A peer banner that ends in a bare line feed is accepted.
    assert_eq!(
        p.call(
            "peer_banner",
            &[ArgValue::Bytes(b"SSH-2.0-dropbear_2022.83\n")]
        ),
        b"dropbear_2022.83"
    );
}
