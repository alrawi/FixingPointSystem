// SPDX-License-Identifier: GPL-3.0-only
//! Golden bytes and round trips of the SMB2 messages. The golden files hold the
//! messages of the exchange in `tests/common/mod.rs` as hexadecimal text, and
//! tshark dissected the same bytes without error in `tests/tshark.rs`.
//! `UPDATE_GOLDEN=1 cargo test -p fps-module-smb2 --test messages` rewrites
//! them after a deliberate change.

mod common;

use common::*;
use fps_flow::FnErrorKind;
use fps_module_smb2::wire::{self, to_hex};

fn golden_text(messages: &[(Dir, Vec<u8>)]) -> String {
    let mut s = String::new();
    for (i, (dir, m)) in messages.iter().enumerate() {
        let arrow = if *dir == Dir::ToServer { "C>S" } else { "S>C" };
        s.push_str(&format!(
            "{:02} {arrow} {:>16} {} octets\n{}\n",
            i,
            header(m, "command"),
            m.len(),
            to_hex(m)
        ));
    }
    s
}

#[test]
fn the_smb_3_1_1_session_matches_its_golden_bytes() {
    fps_golden::assert_golden!(
        "session_3_1_1.txt",
        golden_text(&session("2.0.2,2.1,3.0,3.0.2,3.1.1", "3.1.1", 3000))
    );
}

#[test]
fn the_smb_2_1_session_matches_its_golden_bytes() {
    fps_golden::assert_golden!(
        "session_2_1.txt",
        golden_text(&session("2.0.2,2.1", "2.1", 3000))
    );
}

#[test]
fn the_negotiate_request_has_the_header_that_the_specification_gives() {
    let m = ok(
        "negotiate",
        &[],
        &[
            named("dialects", text("2.0.2,2.1")),
            named("client_guid", bytes(&[0xAB; 16])),
        ],
    );
    // Transport header, then the 64 octet header.
    assert_eq!(&m[..4], [0, 0, 0, 64 + 36 + 4]);
    assert_eq!(&m[4..8], b"\xFESMB");
    assert_eq!(&m[8..10], 64u16.to_le_bytes(), "structure size");
    assert_eq!(
        &m[10..12],
        0u16.to_le_bytes(),
        "credit charge is 0 in a negotiate"
    );
    assert_eq!(&m[16..18], 0u16.to_le_bytes(), "command");
    assert_eq!(&m[18..20], 1u16.to_le_bytes(), "one credit requested");
    assert_eq!(&m[20..24], 0u32.to_le_bytes(), "not a response, not signed");
    // The body: structure size 36, two dialects, signing enabled, no capabilities for 2.x.
    let b = &m[4 + 64..];
    assert_eq!(&b[..2], 36u16.to_le_bytes());
    assert_eq!(&b[2..4], 2u16.to_le_bytes());
    assert_eq!(&b[4..6], 1u16.to_le_bytes());
    assert_eq!(&b[8..12], 0u32.to_le_bytes());
    assert_eq!(&b[12..28], [0xAB; 16]);
    assert_eq!(&b[36..], [0x02, 0x02, 0x10, 0x02]);
}

#[test]
fn offsets_inside_bodies_count_from_the_header() {
    let m = ok("tree_connect", &[text("\\\\h\\s"), int(3), int(77)], &[]);
    // PathOffset is 64 + 8 = 72, PathLength is 5 characters of UTF-16.
    let b = &m[4 + 64..];
    assert_eq!(&b[4..6], 72u16.to_le_bytes());
    assert_eq!(&b[6..8], 10u16.to_le_bytes());
    let create = ok("create", &[text("a\\b"), int(4), int(77), int(5)], &[]);
    let b = &create[4 + 64..];
    assert_eq!(&b[44..46], 120u16.to_le_bytes(), "NameOffset");
    assert_eq!(&b[46..48], 6u16.to_le_bytes(), "NameLength");
    let write = ok(
        "write",
        &[
            text(&"11".repeat(16)),
            bytes(b"abc"),
            int(6),
            int(77),
            int(5),
        ],
        &[],
    );
    let b = &write[4 + 64..];
    assert_eq!(
        &b[2..4],
        112u16.to_le_bytes(),
        "DataOffset of a write request"
    );
    let read = ok(
        "read_response",
        &[bytes(b"abc"), int(6), int(77), int(5)],
        &[],
    );
    assert_eq!(read[4 + 64 + 2], 80, "DataOffset of a read response");
}

#[test]
fn every_field_that_a_writer_set_comes_back_from_the_readers() {
    let sid = 0x0000_4000_0000_0001_i64;
    let fid = "0100000000000000ffffffff00000000";
    let create = ok(
        "create",
        &[text("dir\\file name.txt"), int(9), int(sid), int(5)],
        &[
            named("desired_access", int(0x1234)),
            named("disposition", int(5)),
            named("options", int(0x21)),
            named("oplock", int(8)),
        ],
    );
    assert_eq!(header(&create, "command"), "create");
    assert_eq!(header(&create, "message_id"), "9");
    assert_eq!(header(&create, "session_id"), sid.to_string());
    assert_eq!(header(&create, "tree_id"), "5");
    assert_eq!(header(&create, "response"), "false");
    assert_eq!(header(&create, "credit_charge"), "1");
    assert_eq!(body(&create, "name"), "dir\\file name.txt");
    assert_eq!(body(&create, "desired_access"), 0x1234.to_string());
    assert_eq!(body(&create, "disposition"), "5");
    assert_eq!(body(&create, "options"), 0x21.to_string());
    assert_eq!(body(&create, "oplock"), "8");

    let resp = ok(
        "create_response",
        &[int(9), int(sid), int(5)],
        &[
            named("file_id", text(fid)),
            named("size", int(5000)),
            named("action", int(2)),
        ],
    );
    assert_eq!(body(&resp, "file_id"), fid);
    assert_eq!(body(&resp, "size"), "5000");
    assert_eq!(body(&resp, "allocation_size"), "8192");
    assert_eq!(body(&resp, "action"), "2");
    assert_eq!(header(&resp, "response"), "true");

    let read = ok(
        "read",
        &[text(fid), int(200_000), int(10), int(sid), int(5)],
        &[named("offset", int(65_536))],
    );
    assert_eq!(body(&read, "file_id"), fid);
    assert_eq!(body(&read, "length"), "200000");
    assert_eq!(body(&read, "offset"), "65536");
    assert_eq!(
        header(&read, "credit_charge"),
        "4",
        "one credit per 64 KiB, rounded up"
    );

    let data: Vec<u8> = (0..=255u8).cycle().take(1000).collect();
    let rr = ok(
        "read_response",
        &[bytes(&data), int(10), int(sid), int(5)],
        &[named("remaining", int(7))],
    );
    assert_eq!(ok("data", &[bytes(&rr)], &[]), data);
    assert_eq!(body(&rr, "length"), "1000");
    assert_eq!(body(&rr, "remaining"), "7");

    let w = ok(
        "write",
        &[text(fid), bytes(&data), int(11), int(sid), int(5)],
        &[named("offset", int(4096)), named("flags", int(1))],
    );
    assert_eq!(ok("data", &[bytes(&w)], &[]), data);
    assert_eq!(body(&w, "offset"), "4096");
    assert_eq!(body(&w, "length"), "1000");
    assert_eq!(body(&w, "flags"), "1");
    let wr = ok(
        "write_response",
        &[int(1000), int(11), int(sid), int(5)],
        &[],
    );
    assert_eq!(body(&wr, "count"), "1000");

    let close = ok(
        "close",
        &[text(fid), int(12), int(sid), int(5)],
        &[named("flags", int(1))],
    );
    assert_eq!(body(&close, "file_id"), fid);
    assert_eq!(body(&close, "flags"), "1");
    let cr = ok(
        "close_response",
        &[int(12), int(sid), int(5)],
        &[
            named("flags", int(1)),
            named("size", int(1000)),
            named("attributes", int(0x80)),
        ],
    );
    assert_eq!(body(&cr, "size"), "1000");
    assert_eq!(body(&cr, "attributes"), "128");
    let plain = ok(
        "close_response",
        &[int(12), int(sid), int(5)],
        &[named("size", int(1000))],
    );
    assert_eq!(
        body(&plain, "size"),
        "0",
        "no attributes unless the flag asks for them"
    );

    let tc = ok(
        "tree_connect_response",
        &[int(3), int(sid), int(5)],
        &[
            named("share_type", text("pipe")),
            named("maximal_access", int(0x1)),
        ],
    );
    assert_eq!(header(&tc, "tree_id"), "5");
    assert_eq!(body(&tc, "share_type"), "pipe");
    assert_eq!(body(&tc, "maximal_access"), "1");

    let q = ok(
        "query_directory",
        &[text(fid), int(13), int(sid), int(5)],
        &[
            named("pattern", text("*.txt")),
            named("single", boolean(true)),
        ],
    );
    assert_eq!(body(&q, "pattern"), "*.txt");
    assert_eq!(body(&q, "flags"), "3");
    assert_eq!(body(&q, "output_length"), "65536");
    let listing = "0 directory docs\n1048576 file big file.bin\n12 file a.txt";
    let qr = ok(
        "query_directory_response",
        &[text(listing), int(13), int(sid), int(5)],
        &[],
    );
    assert_eq!(body(&qr, "entry_count"), "3");
    assert_eq!(body(&qr, "entries"), listing);
}

#[test]
fn negotiate_round_trips_dialects_contexts_and_security() {
    let req = ok(
        "negotiate",
        &[],
        &[
            named("dialects", text("2.0.2,2.1,3.0,3.0.2,3.1.1")),
            named("signing", text("required")),
        ],
    );
    assert_eq!(body(&req, "dialects"), "2.0.2,2.1,3.0,3.0.2,3.1.1");
    assert_eq!(body(&req, "dialect_count"), "5");
    assert_eq!(body(&req, "security_mode"), "3");
    assert_eq!(body(&req, "capabilities"), "7");
    assert_eq!(body(&req, "guid").len(), 32);
    // Contexts start at an offset that is a multiple of 8 and that the body states.
    let b = &req[4 + 64..];
    let off = u32::from_le_bytes([b[28], b[29], b[30], b[31]]) as usize;
    assert_eq!(off % 8, 0);
    assert_eq!(u16::from_le_bytes([b[32], b[33]]), 2);
    assert_eq!(
        &req[4 + off..4 + off + 2],
        1u16.to_le_bytes(),
        "the first context is the preauthentication one"
    );

    let old = ok("negotiate", &[], &[named("dialects", text("2.1"))]);
    assert_eq!(
        body(&old, "capabilities"),
        "0",
        "a 2.x client offers no capabilities"
    );

    let resp = ok(
        "negotiate_response",
        &[],
        &[
            named("dialect", text("3.1.1")),
            named("max_size", int(1_048_576)),
        ],
    );
    assert_eq!(body(&resp, "dialect"), "3.1.1");
    assert_eq!(body(&resp, "max_read"), "1048576");
    let token = ok("security_blob", &[bytes(&resp)], &[]);
    assert_eq!(
        token[0], 0x60,
        "the default token is an initial SPNEGO token"
    );
    assert!(token
        .windows(10)
        .any(|w| w == [0x2B, 0x06, 0x01, 0x04, 0x01, 0x82, 0x37, 0x02, 0x02, 0x0A]));
}

#[test]
fn security_buffers_travel_through_session_setup_in_both_directions() {
    let n = ok("ntlm_negotiate", &[], &[]);
    let req = ok(
        "session_setup",
        &[bytes(&n), int(1)],
        &[
            named("previous_session_id", int(-5)),
            named("binding", boolean(true)),
        ],
    );
    assert_eq!(ok("security_blob", &[bytes(&req)], &[]), n);
    assert_eq!(body(&req, "binding"), "true");
    assert_eq!(body(&req, "previous_session_id"), "-5");
    let c = ok("ntlm_challenge", &[], &[]);
    let resp = ok(
        "session_setup_response",
        &[bytes(&c), int(1), int(-1)],
        &[
            named("status", text("more_processing_required")),
            named("session_flags", int(1)),
        ],
    );
    assert_eq!(ok("security_blob", &[bytes(&resp)], &[]), c);
    assert_eq!(header(&resp, "status"), "more_processing_required");
    assert_eq!(header(&resp, "status_code"), 0xC000_0016u32.to_string());
    assert_eq!(
        header(&resp, "session_id"),
        "-1",
        "all 64 bits round trip as two's complement"
    );
    assert_eq!(body(&resp, "session_flags"), "1");
    // The id that a response carries goes into the next request unchanged.
    let next = ok("tree_connect", &[text("\\\\h\\s"), int(2), int(-1)], &[]);
    assert_eq!(header(&next, "session_id"), "-1");
    assert_eq!(&next[4 + 40..4 + 48], [0xFF; 8]);
}

#[test]
fn error_responses_carry_the_status_and_the_command_of_the_request() {
    let e = ok(
        "error_response",
        &[text("read"), text("end_of_file"), int(5), int(7), int(9)],
        &[],
    );
    assert_eq!(header(&e, "command"), "read");
    assert_eq!(header(&e, "status"), "end_of_file");
    assert_eq!(header(&e, "response"), "true");
    assert_eq!(e.len(), 4 + 64 + 9);
    let by_number = ok(
        "error_response",
        &[text("create"), text("0xC0000022"), int(5)],
        &[],
    );
    assert_eq!(header(&by_number, "status"), "access_denied");
    let err = call("body_field", &[bytes(&e), text("length")], &[]).unwrap_err();
    assert_eq!(err.kind, FnErrorKind::Protocol);
    assert!(err.message.contains("error response"), "{err}");
    let err = call(
        "error_response",
        &[text("read"), text("success"), int(5)],
        &[],
    )
    .unwrap_err();
    assert_eq!(err.kind, FnErrorKind::InvalidArgument);
}

#[test]
fn readers_take_a_message_with_or_without_the_transport_header() {
    let m = ok("logoff", &[int(3), int(8)], &[]);
    assert_eq!(header(&m, "command"), "logoff");
    assert_eq!(header(&m[4..], "command"), "logoff");
    assert_eq!(
        ok("message_length", &[bytes(&m[..4])], &[]),
        (m.len() - 4).to_string().into_bytes()
    );
    let e = call("message_length", &[bytes(&[1, 0, 0, 5])], &[]).unwrap_err();
    assert_eq!(e.kind, FnErrorKind::Protocol);
}

#[test]
fn signing_sets_the_flag_and_the_signature_matches_hmac_sha256() {
    use hmac::{Hmac, Mac};
    let m = ok("tree_disconnect", &[int(4), int(8), int(2)], &[]);
    let key = [0x42u8; 16];
    let signed = ok("sign", &[bytes(&m), text(&to_hex(&key))], &[]);
    assert_eq!(&signed[..4], &m[..4], "the transport header stays");
    assert_eq!(header(&signed, "signed"), "true");
    let sig = &signed[4 + 48..4 + 64];
    assert_ne!(sig, [0u8; 16]);
    // An independent computation of the signature over the message with a zero signature field.
    let mut unsigned = signed[4..].to_vec();
    unsigned[48..64].fill(0);
    let mut mac = <Hmac<sha2::Sha256> as Mac>::new_from_slice(&key).unwrap();
    mac.update(&unsigned);
    assert_eq!(sig, &mac.finalize().into_bytes()[..16]);
    // Signing is deterministic and does not depend on a signature that was there.
    assert_eq!(
        ok("sign", &[bytes(&signed), text(&to_hex(&key))], &[]),
        signed
    );
}

#[test]
fn the_session_key_of_ntlm_signs_both_directions() {
    let c = ok("ntlm_challenge", &[], &[]);
    let a = ok(
        "ntlm_authenticate",
        &[bytes(&c), text("carol"), text("pw")],
        &[],
    );
    let client_key = ok("ntlm_session_key", &[bytes(&a), text("pw")], &[]);
    let server_key = ok("ntlm_session_key", &[bytes(&a), text("pw")], &[]);
    assert_eq!(client_key, server_key);
    let m = ok("logoff", &[int(3), int(8)], &[]);
    let s1 = ok(
        "sign",
        &[bytes(&m), text(std::str::from_utf8(&client_key).unwrap())],
        &[],
    );
    let s2 = ok(
        "sign",
        &[bytes(&m), text(std::str::from_utf8(&server_key).unwrap())],
        &[],
    );
    assert_eq!(s1, s2);
}

#[test]
fn helpers_convert_and_count() {
    assert_eq!(ok("hex", &[bytes(&[0, 0xFF, 0x10])], &[]), b"00ff10");
    assert_eq!(ok("unhex", &[text("00ff10")], &[]), [0, 0xFF, 0x10]);
    assert_eq!(ok("unhex", &[text("00:FF 10")], &[]), [0, 0xFF, 0x10]);
    assert_eq!(
        call("unhex", &[text("abc")], &[]).unwrap_err().kind,
        FnErrorKind::InvalidArgument
    );
    assert_eq!(
        call("unhex", &[text("zz")], &[]).unwrap_err().kind,
        FnErrorKind::InvalidArgument
    );
    for (total, size, want) in [
        (0, 1000, "1"),
        (1, 1000, "1"),
        (1000, 1000, "1"),
        (1001, 1000, "2"),
        (1_048_576, 65_536, "16"),
    ] {
        assert_eq!(
            ok("chunks", &[int(total), int(size)], &[]),
            want.as_bytes(),
            "{total}/{size}"
        );
    }
    assert_eq!(
        call("chunks", &[int(10), int(0)], &[]).unwrap_err().kind,
        FnErrorKind::InvalidArgument
    );
}

#[test]
fn bad_arguments_are_invalid_argument_errors() {
    let not_hex = "zz".repeat(16);
    let zero_id = "00".repeat(16);
    let cases: Vec<(&str, Vec<fps_flow::ArgValue<'_>>)> = vec![
        ("create", vec![text("x"), int(-1), int(0), int(0)]),
        ("create", vec![text("x"), int(0), int(0), int(1 << 40)]),
        ("read", vec![text("short"), int(1), int(0), int(0), int(0)]),
        ("read", vec![text(&not_hex), int(1), int(0), int(0), int(0)]),
        (
            "read",
            vec![text(&zero_id), int(-1), int(0), int(0), int(0)],
        ),
        ("tree_connect", vec![text(""), int(0), int(0)]),
        ("negotiate_response", vec![text("9.9")]),
        ("negotiate", vec![text("1.0")]),
        (
            "session_setup_response",
            vec![bytes(b""), int(0), int(0), text("no_such_status")],
        ),
        (
            "query_directory_response",
            vec![text("not a listing"), int(0), int(0), int(0)],
        ),
        (
            "query_directory_response",
            vec![text("1 file a/b"), int(0), int(0), int(0)],
        ),
        (
            "error_response",
            vec![text("frobnicate"), text("access_denied"), int(0)],
        ),
    ];
    for (f, args) in cases {
        let e = call(f, &args, &[]).unwrap_err();
        assert_eq!(e.kind, FnErrorKind::InvalidArgument, "{f}: {e}");
    }
    let e = call(
        "write",
        &[
            text(&"00".repeat(16)),
            bytes(&vec![0; 0x100_0000]),
            int(0),
            int(0),
            int(0),
        ],
        &[],
    )
    .unwrap_err();
    assert_eq!(
        e.kind,
        FnErrorKind::InvalidArgument,
        "a write beyond the transport length: {e}"
    );
}

#[test]
fn every_prefix_of_every_message_is_an_error_or_a_value_and_never_a_panic() {
    let messages = session("2.0.2,2.1,3.0,3.0.2,3.1.1", "3.1.1", 100);
    for (_, m) in &messages {
        for cut in 0..m.len() {
            for f in ["header_field", "body_field"] {
                for field in [
                    "command", "file_id", "name", "length", "dialects", "path", "entries",
                    "pattern",
                ] {
                    let _ = call(f, &[bytes(&m[..cut]), text(field)], &[]);
                }
            }
            for f in ["security_blob", "data"] {
                let _ = call(f, &[bytes(&m[..cut])], &[]);
            }
        }
    }
}

#[test]
fn status_and_dialect_names_round_trip() {
    for (code, name) in wire::STATUS_NAMES {
        assert_eq!(wire::status_code(name), Some(*code));
        assert_eq!(wire::status_name(*code), *name);
    }
    assert_eq!(wire::status_name(0xDEAD_BEEF), "0xdeadbeef");
    assert_eq!(wire::status_code("3221225506"), Some(0xC000_0022));
    for (code, name) in wire::DIALECTS {
        assert_eq!(wire::dialect_code(name), Some(*code));
        assert_eq!(wire::dialect_name(*code), *name);
    }
}
