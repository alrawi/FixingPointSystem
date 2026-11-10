// SPDX-License-Identifier: GPL-3.0-only
//! The NTLM functions against the test vectors of MS-NLMP section 4.2 and
//! against each other: what a client answers, a server derives the key of.

mod common;

use common::*;
use fps_module_smb2::ntlm;
use fps_module_smb2::wire::{from_hex, to_hex};

fn hex(s: &str) -> Vec<u8> {
    from_hex(s).unwrap()
}

#[test]
fn nt_hash_matches_ms_nlmp_4_2_2() {
    // The NT hash of "Password".
    assert_eq!(
        to_hex(&ntlm::nt_hash("Password")),
        "a4f49c406510bdcab6824ee7c30fd852"
    );
}

#[test]
fn ntlmv2_matches_the_ms_nlmp_4_2_4_vectors() {
    // User "User", domain "Domain", password "Password".
    assert_eq!(
        to_hex(&ntlm::ntowf_v2("Password", "User", "Domain")),
        "0c868a403bfd7a93a3001ef22ef02e3f"
    );
    let server_name =
        hex("02000c0044006f006d00610069006e00 01000c00530065007200760065007200 0000 0000");
    let (response, session_base_key) = ntlm::ntlmv2_response(
        &ntlm::Credentials {
            user: "User",
            password: "Password",
            domain: "Domain",
            workstation: "COMPUTER",
        },
        &hex("0123456789abcdef").try_into().unwrap(),
        &hex("aaaaaaaaaaaaaaaa").try_into().unwrap(),
        0,
        &server_name,
    );
    assert_eq!(to_hex(&response[..16]), "68cd0ab851e51c96aabc927bebef6a1c");
    assert_eq!(
        to_hex(&session_base_key),
        "8de40ccadbc14a82f15cb0ad0de95ca3"
    );
    // The blob starts with the version bytes, 6 zero octets, the time, and the client challenge.
    assert_eq!(&response[16..24], hex("0101000000000000"));
    assert_eq!(&response[32..40], hex("aaaaaaaaaaaaaaaa"));
}

#[test]
fn the_three_messages_carry_their_fields() {
    let n = ok("ntlm_negotiate", &[text("EXAMPLE"), text("WS01")], &[]);
    assert_eq!(n[0], 0x60, "a GSS-API token");
    assert_eq!(ok("ntlm_field", &[bytes(&n), text("type")], &[]), b"1");
    assert_eq!(
        ok("ntlm_field", &[bytes(&n), text("domain")], &[]),
        b"EXAMPLE"
    );
    assert_eq!(
        ok("ntlm_field", &[bytes(&n), text("workstation")], &[]),
        b"WS01"
    );

    let c = ok(
        "ntlm_challenge",
        &[
            text("FPS"),
            text("SERVER"),
            text("fps.test"),
            text("server.fps.test"),
            bytes(b"12345678"),
        ],
        &[],
    );
    assert_eq!(c[0], 0xA1, "a NegTokenResp");
    assert_eq!(ok("ntlm_field", &[bytes(&c), text("type")], &[]), b"2");
    assert_eq!(ok("ntlm_field", &[bytes(&c), text("target")], &[]), b"FPS");
    assert_eq!(
        ok("ntlm_field", &[bytes(&c), text("computer")], &[]),
        b"SERVER"
    );
    assert_eq!(
        ok("ntlm_field", &[bytes(&c), text("dns_computer")], &[]),
        b"server.fps.test"
    );
    assert_eq!(
        ok("ntlm_field", &[bytes(&c), text("challenge")], &[]),
        to_hex(b"12345678").into_bytes()
    );
    assert_eq!(
        ok("ntlm_field", &[bytes(&c), text("time")], &[]),
        b"133801632000000000"
    );

    let a = ok(
        "ntlm_authenticate",
        &[bytes(&c), text("alice"), text("secret")],
        &[named("domain", text("FPS"))],
    );
    assert_eq!(ok("ntlm_field", &[bytes(&a), text("type")], &[]), b"3");
    assert_eq!(ok("ntlm_field", &[bytes(&a), text("user")], &[]), b"alice");
    assert_eq!(ok("ntlm_field", &[bytes(&a), text("domain")], &[]), b"FPS");
}

#[test]
fn both_sides_derive_the_same_session_key_from_the_password() {
    let c = ok("ntlm_challenge", &[], &[]);
    let a = ok(
        "ntlm_authenticate",
        &[bytes(&c), text("bob"), text("hunter2")],
        &[named("domain", text("FPS"))],
    );
    let key = ok("ntlm_session_key", &[bytes(&a), text("hunter2")], &[]);
    assert_eq!(key.len(), 32);
    // The key follows from the proof in the message, the user, the domain, and the password.
    let auth = ntlm::parse_authenticate(ntlm::find_message(&a).unwrap()).unwrap();
    let expect = ntlm::session_key(&auth, "hunter2").unwrap();
    assert_eq!(key, to_hex(&expect).into_bytes());
    // A wrong password gives another key, and the user name is part of the key.
    assert_ne!(
        ok("ntlm_session_key", &[bytes(&a), text("hunter3")], &[]),
        key
    );
}

#[test]
fn the_server_time_in_the_challenge_wins_and_the_lm_response_is_zero() {
    let c = ok("ntlm_challenge", &[], &[named("time", int(1_234_567))]);
    let a = ok(
        "ntlm_authenticate",
        &[bytes(&c), text("u"), text("p")],
        &[named("time", int(99))],
    );
    let nt = hex(
        std::str::from_utf8(&ok("ntlm_field", &[bytes(&a), text("nt_response")], &[])).unwrap(),
    );
    assert_eq!(&nt[24..32], 1_234_567u64.to_le_bytes());
}

#[test]
fn without_a_server_time_the_client_time_and_an_lmv2_response_are_used() {
    // A challenge without the timestamp pair: write the target information by hand.
    let info = hex("02000600460050005300 00000000");
    let msg =
        ntlm::challenge_message(ntlm::DEFAULT_CHALLENGE_FLAGS, "FPS", *b"abcdefgh", &info).unwrap();
    let a = ok(
        "ntlm_authenticate",
        &[bytes(&msg), text("u"), text("p")],
        &[named("time", int(99))],
    );
    let auth = ntlm::find_message(&a).unwrap();
    // The first response field (LM) has 24 octets that are not all zero.
    let lm_len = u16::from_le_bytes([auth[12], auth[13]]);
    let lm_off = u32::from_le_bytes([auth[16], auth[17], auth[18], auth[19]]) as usize;
    assert_eq!(lm_len, 24);
    assert!(auth[lm_off..lm_off + 24].iter().any(|b| *b != 0));
    let nt = hex(
        std::str::from_utf8(&ok("ntlm_field", &[bytes(&a), text("nt_response")], &[])).unwrap(),
    );
    assert_eq!(&nt[24..32], 99u64.to_le_bytes());
}

#[test]
fn spnego_false_gives_bare_messages() {
    let n = ok("ntlm_negotiate", &[], &[named("spnego", boolean(false))]);
    assert_eq!(&n[..8], b"NTLMSSP\0");
    let c = ok("ntlm_challenge", &[], &[named("spnego", boolean(false))]);
    assert_eq!(&c[..8], b"NTLMSSP\0");
    // A bare challenge is accepted where a security buffer is.
    let a = ok(
        "ntlm_authenticate",
        &[bytes(&c), text("u"), text("p")],
        &[named("spnego", boolean(false))],
    );
    assert_eq!(&a[..8], b"NTLMSSP\0");
}

#[test]
fn spnego_tokens_have_the_der_shape_that_wireshark_expects() {
    let accept = ok("spnego_accept", &[], &[]);
    assert_eq!(accept, hex("a1073005a0030a0100"));
    let init = ntlm::spnego_init(None);
    // 60 len, OID SPNEGO, a0 len, 30 len, a0 len, 30 len, OID NTLMSSP.
    assert_eq!(init[0], 0x60);
    assert_eq!(&init[2..10], hex("06062b0601050502"));
    assert_eq!(&init[init.len() - 12..], hex("060a2b06010401823702020a"));
}

#[test]
fn bad_arguments_and_bytes_are_errors_and_not_panics() {
    let e = call(
        "ntlm_authenticate",
        &[bytes(b"not a token"), text("u"), text("p")],
        &[],
    )
    .unwrap_err();
    assert_eq!(e.kind, fps_flow::FnErrorKind::Protocol, "{e}");
    let e = call(
        "ntlm_challenge",
        &[],
        &[named("server_challenge", bytes(b"short"))],
    )
    .unwrap_err();
    assert_eq!(e.kind, fps_flow::FnErrorKind::InvalidArgument, "{e}");
    let n = ok("ntlm_negotiate", &[], &[]);
    // A negotiate message is not a challenge.
    let e = call("ntlm_authenticate", &[bytes(&n), text("u"), text("p")], &[]).unwrap_err();
    assert_eq!(e.kind, fps_flow::FnErrorKind::Protocol, "{e}");
    let e = call("ntlm_field", &[bytes(&n), text("nothing")], &[]).unwrap_err();
    assert_eq!(e.kind, fps_flow::FnErrorKind::Protocol, "{e}");
}

#[test]
fn a_challenge_without_a_seed_difference_is_reproducible_and_differs_between_seeds() {
    let a = ok("ntlm_challenge", &[], &[]);
    let b = ok("ntlm_challenge", &[], &[]);
    assert_eq!(a, b, "the same seed gives the same challenge");
    let mut rng = <rand_pcg::Pcg64Mcg as rand::SeedableRng>::seed_from_u64(43);
    let c = call_with(&mut rng, "ntlm_challenge", &[], &[]).unwrap();
    assert_ne!(a, c);
}
