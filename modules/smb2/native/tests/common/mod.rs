// SPDX-License-Identifier: GPL-3.0-only
//! Helpers that the integration tests share: calls of the template functions
//! with a seeded generator, and the exchange that the tshark and golden tests
//! encode.

#![allow(dead_code)] // each test target uses some of the helpers

use fps_flow::function::NamedArg;
use fps_flow::{ArgValue, CallArgs, FnCall, FnError, TemplateFn};
use fps_module_smb2::Smb2;
use rand::SeedableRng;
use rand_pcg::Pcg64Mcg;

pub fn call_with(
    rng: &mut Pcg64Mcg,
    function: &str,
    positional: &[ArgValue<'_>],
    named: &[NamedArg<'_>],
) -> Result<Vec<u8>, FnError> {
    let mut out = Vec::new();
    let c = FnCall {
        module: "smb2",
        function,
        args: CallArgs { positional, named },
    };
    Smb2.render(&c, rng, &mut out).map(|()| out)
}

/// Calls a function with a fresh generator of seed 42.
pub fn call(
    function: &str,
    positional: &[ArgValue<'_>],
    named: &[NamedArg<'_>],
) -> Result<Vec<u8>, FnError> {
    call_with(&mut Pcg64Mcg::seed_from_u64(42), function, positional, named)
}

pub fn ok(function: &str, positional: &[ArgValue<'_>], named: &[NamedArg<'_>]) -> Vec<u8> {
    match call(function, positional, named) {
        Ok(v) => v,
        Err(e) => panic!("{function} failed: {e}"),
    }
}

pub fn int(v: i64) -> ArgValue<'static> {
    ArgValue::Int(v)
}

pub fn text(v: &str) -> ArgValue<'_> {
    ArgValue::Text(v)
}

pub fn bytes(v: &[u8]) -> ArgValue<'_> {
    ArgValue::Bytes(v)
}

pub fn boolean(v: bool) -> ArgValue<'static> {
    ArgValue::Boolean(v)
}

pub fn named<'a>(name: &'a str, value: ArgValue<'a>) -> NamedArg<'a> {
    NamedArg { name, value }
}

/// Reads a header or body field of a message as text.
pub fn header(message: &[u8], field: &str) -> String {
    String::from_utf8(ok("header_field", &[bytes(message), text(field)], &[])).unwrap()
}

pub fn body(message: &[u8], field: &str) -> String {
    String::from_utf8(ok("body_field", &[bytes(message), text(field)], &[])).unwrap()
}

/// The direction of one message of an exchange.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dir {
    /// From the client to the server.
    ToServer,
    /// From the server to the client.
    ToClient,
}

/// One whole SMB2 session as a list of messages with the direction that each
/// travels. The exchange authenticates with NTLM, opens a share, reads and
/// writes a file, lists a directory, hits two errors, and tears the session down.
///
/// `dialects` is what the client offers and `dialect` is what the server picks.
pub fn session(dialects: &str, dialect: &str, big_read: usize) -> Vec<(Dir, Vec<u8>)> {
    use Dir::{ToClient as S, ToServer as C};
    let sid = 0x0000_4000_0000_0001_i64;
    let tid = 5;
    let mut rng = Pcg64Mcg::seed_from_u64(7);
    let mut run = |f: &str, p: &[ArgValue<'_>], n: &[NamedArg<'_>]| match call_with(&mut rng, f, p, n) {
        Ok(v) => v,
        Err(e) => panic!("{f}: {e}"),
    };
    let mut m: Vec<(Dir, Vec<u8>)> = Vec::new();
    let modern = dialect.starts_with('3');
    let charge = if dialect == "2.0.2" { 0 } else { -1 };

    m.push((C, run("negotiate", &[], &[named("dialects", text(dialects)), named("signing", text("enabled"))])));
    m.push((S, run("negotiate_response", &[], &[named("dialect", text(dialect)), named("signing", text("enabled"))])));

    let n1 = run("ntlm_negotiate", &[], &[named("workstation", text("WS01"))]);
    m.push((C, run("session_setup", &[bytes(&n1), int(1)], &[named("credit_charge", int(charge))])));
    let ch = run("ntlm_challenge", &[], &[named("target", text("FPS")), named("computer", text("FPSSERVER")), named("dns_domain", text("fps.test")), named("dns_computer", text("fpsserver.fps.test"))]);
    m.push((S, run("session_setup_response", &[bytes(&ch), int(1), int(sid)], &[named("status", text("more_processing_required")), named("credit_charge", int(charge))])));
    let auth = run(
        "ntlm_authenticate",
        &[bytes(&ch), text("alice"), text("Passw0rd!")],
        &[named("domain", text("FPS")), named("workstation", text("WS01"))],
    );
    m.push((C, run("session_setup", &[bytes(&auth), int(2)], &[named("session_id", int(sid)), named("credit_charge", int(charge))])));
    let done = run("spnego_accept", &[], &[]);
    m.push((S, run("session_setup_response", &[bytes(&done), int(2), int(sid)], &[named("credit_charge", int(charge))])));

    m.push((C, run("tree_connect", &[text("\\\\fpsserver\\share"), int(3), int(sid)], &[named("credit_charge", int(charge))])));
    m.push((S, run("tree_connect_response", &[int(3), int(sid), int(tid)], &[named("share_flags", int(0x30)), named("credit_charge", int(charge))])));

    // Open and read a file.
    let fid = "0100000000000000ffffffff00000000";
    m.push((C, run("create", &[text("docs\\report.bin"), int(4), int(sid), int(tid)], &[named("credit_charge", int(charge))])));
    m.push((S, run("create_response", &[int(4), int(sid), int(tid)], &[named("file_id", text(fid)), named("size", int(big_read as i64)), named("credit_charge", int(charge))])));
    let data: Vec<u8> = (0..big_read).map(|i| (i % 251) as u8).collect();
    m.push((C, run("read", &[text(fid), int(big_read as i64), int(5), int(sid), int(tid)], &[named("credit_charge", int(charge))])));
    m.push((S, run("read_response", &[bytes(&data), int(5), int(sid), int(tid)], &[named("credit_charge", int(charge))])));

    // Create a file and write to it.
    let fid2 = "0200000000000000feffffff00000000";
    m.push((C, run("create", &[text("upload\\new.txt"), int(6), int(sid), int(tid)], &[named("desired_access", int(0x0012_019F)), named("disposition", int(2)), named("file_attributes", int(0x80)), named("credit_charge", int(charge))])));
    m.push((S, run("create_response", &[int(6), int(sid), int(tid)], &[named("file_id", text(fid2)), named("action", int(2)), named("attributes", int(0x80)), named("credit_charge", int(charge))])));
    let payload: Vec<u8> = (0..3000).map(|i| (i % 97) as u8).collect();
    m.push((C, run("write", &[text(fid2), bytes(&payload), int(7), int(sid), int(tid)], &[named("offset", int(0)), named("credit_charge", int(charge))])));
    m.push((S, run("write_response", &[int(3000), int(7), int(sid), int(tid)], &[named("credit_charge", int(charge))])));
    m.push((C, run("close", &[text(fid2), int(8), int(sid), int(tid)], &[named("flags", int(1)), named("credit_charge", int(charge))])));
    m.push((S, run("close_response", &[int(8), int(sid), int(tid)], &[named("flags", int(1)), named("size", int(3000)), named("credit_charge", int(charge))])));

    // List the root directory.
    let fid3 = "0300000000000000fdffffff00000000";
    m.push((C, run("create", &[text(""), int(9), int(sid), int(tid)], &[named("options", int(1)), named("share_access", int(7)), named("credit_charge", int(charge))])));
    m.push((S, run("create_response", &[int(9), int(sid), int(tid)], &[named("file_id", text(fid3)), named("attributes", int(0x10)), named("credit_charge", int(charge))])));
    m.push((C, run("query_directory", &[text(fid3), int(10), int(sid), int(tid)], &[named("credit_charge", int(charge))])));
    m.push((S, run("query_directory_response", &[text("0 directory docs\n0 directory upload\n1048576 file big file.bin\n12 file a.txt"), int(10), int(sid), int(tid)], &[named("credit_charge", int(charge))])));
    m.push((C, run("query_directory", &[text(fid3), int(11), int(sid), int(tid)], &[named("restart", boolean(false)), named("credit_charge", int(charge))])));
    m.push((S, run("error_response", &[text("query_directory"), text("no_more_files"), int(11), int(sid), int(tid)], &[named("credit_charge", int(charge))])));

    // A missing file and a refused one.
    m.push((C, run("create", &[text("docs\\missing.txt"), int(12), int(sid), int(tid)], &[named("credit_charge", int(charge))])));
    m.push((S, run("error_response", &[text("create"), text("object_name_not_found"), int(12), int(sid), int(tid)], &[named("credit_charge", int(charge))])));
    m.push((C, run("create", &[text("secret.txt"), int(13), int(sid), int(tid)], &[named("credit_charge", int(charge))])));
    m.push((S, run("error_response", &[text("create"), text("access_denied"), int(13), int(sid), int(tid)], &[named("credit_charge", int(charge))])));

    m.push((C, run("close", &[text(fid3), int(14), int(sid), int(tid)], &[named("credit_charge", int(charge))])));
    m.push((S, run("close_response", &[int(14), int(sid), int(tid)], &[named("credit_charge", int(charge))])));
    m.push((C, run("tree_disconnect", &[int(15), int(sid), int(tid)], &[named("credit_charge", int(charge))])));
    m.push((S, run("tree_disconnect_response", &[int(15), int(sid), int(tid)], &[named("credit_charge", int(charge))])));
    m.push((C, run("logoff", &[int(16), int(sid)], &[named("credit_charge", int(charge))])));
    m.push((S, run("logoff_response", &[int(16), int(sid)], &[named("credit_charge", int(charge))])));
    let _ = modern;
    m
}
