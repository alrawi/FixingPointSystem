// SPDX-License-Identifier: GPL-3.0-only
//! Interpreter tests of the ssh scenarios. Every scenario runs with both
//! sides against each other over an in memory network, and the bytes of the
//! connection are checked as far as the wire shows them: the identification
//! lines and the clear packets of the key exchange are parsed, and everything
//! after `SSH_MSG_NEWKEYS` must be ciphertext.
//!
//! A session that completes proves the key exchange. The client verifies the
//! signature of the exchange hash with the host key of the reply, and every
//! packet after `SSH_MSG_NEWKEYS` carries a Poly1305 tag, so the keys of the
//! two sides are equal or the session fails. The tests that flip a bit show
//! that failure.

mod common;

use common::{run, run_flipping, scenarios, scenarios_with, Conn, Run, SEED};
use fps_flow::Value;
use fps_interp::{FlowState, Side};

/// One clear packet of the key exchange.
struct Plain {
    start: usize,
    end: usize,
    message: u8,
}

/// Splits what a side wrote into its identification line and the clear
/// packets up to and including `SSH_MSG_NEWKEYS`. Returns the line, the
/// packets, and the offset where the ciphertext starts.
fn plain_prefix(wire: &[u8]) -> (String, Vec<Plain>, usize) {
    let nl = wire
        .iter()
        .position(|b| *b == b'\n')
        .expect("an identification line");
    let line = std::str::from_utf8(&wire[..nl + 1]).expect("text").to_string();
    let mut at = nl + 1;
    let mut packets = Vec::new();
    loop {
        let len = u32::from_be_bytes(wire[at..at + 4].try_into().unwrap()) as usize;
        assert!((12..=35_000).contains(&len) && (len + 4) % 8 == 0, "clear packet length {len}");
        let message = wire[at + 5];
        packets.push(Plain {
            start: at,
            end: at + 4 + len,
            message,
        });
        at += 4 + len;
        if message == 21 {
            return (line, packets, at);
        }
        assert!(packets.len() < 4, "no SSH_MSG_NEWKEYS after the key exchange");
    }
}

fn session(run: &Run) -> Conn {
    let conns = run.conns_of("session");
    assert_eq!(conns.len(), 1, "one connection per session");
    assert_eq!(conns[0].port, 22);
    conns[0].clone()
}

/// The checks that every scenario passes at every size.
fn check(name: &str, run: &Run) -> Conn {
    if let Err(e) = &run.result {
        panic!("{name}: {e}");
    }
    for (flow, state) in &run.states {
        assert_eq!(*state, Some(FlowState::Done), "{name} flow {flow}");
    }
    assert!(run.records.iter().all(|r| r.failed.is_none()), "{name}");
    let conn = session(run);
    let (c2s, s2c) = (conn.c2s(), conn.s2c());
    let (cl, cp, c_cipher) = plain_prefix(&c2s);
    let (sl, sp, s_cipher) = plain_prefix(&s2c);
    assert!(cl.starts_with("SSH-2.0-OpenSSH_") && cl.ends_with("\r\n"), "{name}: {cl:?}");
    assert!(sl.starts_with("SSH-2.0-OpenSSH_") && sl.ends_with("\r\n"), "{name}: {sl:?}");
    // The client sends SSH_MSG_KEXINIT, SSH_MSG_KEX_ECDH_INIT, SSH_MSG_NEWKEYS,
    // and the server SSH_MSG_KEXINIT, SSH_MSG_KEX_ECDH_REPLY, SSH_MSG_NEWKEYS.
    let types = |p: &[Plain]| p.iter().map(|x| x.message).collect::<Vec<_>>();
    assert_eq!(types(&cp), [20, 30, 21], "{name}");
    assert_eq!(types(&sp), [20, 31, 21], "{name}");
    assert!(c2s.len() > c_cipher && s2c.len() > s_cipher, "{name}: nothing after the key exchange");
    // Nothing that a scenario says after the key exchange shows on the wire.
    for (dir, wire) in [("c2s", &c2s[c_cipher..]), ("s2c", &s2c[s_cipher..])] {
        for needle in [
            "ssh-userauth",
            "ssh-connection",
            "password",
            "pty-req",
            "xterm",
            "C0644",
            "exit-status",
            "session",
        ] {
            assert!(
                !wire.windows(needle.len()).any(|w| w == needle.as_bytes()),
                "{name} {dir}: '{needle}' travels in the clear"
            );
        }
    }
    conn
}

/// The host key blob of the `SSH_MSG_KEX_ECDH_REPLY` in `s2c`.
fn host_key(s2c: &[u8]) -> Vec<u8> {
    let (_, packets, _) = plain_prefix(s2c);
    let reply = &s2c[packets[1].start..packets[1].end];
    // length, padding length, message number, then the string of the key.
    let n = u32::from_be_bytes(reply[6..10].try_into().unwrap()) as usize;
    reply[10..10 + n].to_vec()
}

#[tokio::test(start_paused = true)]
async fn every_scenario_completes_the_key_exchange_in_every_session() {
    let all = scenarios();
    assert_eq!(all.len(), 4);
    for (name, s) in &all {
        for session_id in [1, 7, 301] {
            let r = run(s, SEED, session_id).await;
            check(name, &r);
        }
    }
}

#[tokio::test(start_paused = true)]
async fn the_host_key_stays_and_the_ephemeral_keys_change() {
    let all = scenarios();
    let (name, s) = &all[0];
    let mut keys = Vec::new();
    let mut inits = Vec::new();
    for session_id in [1, 2, 3, 4] {
        let r = run(s, SEED, session_id).await;
        let conn = check(name, &r);
        keys.push(host_key(&conn.s2c()));
        let (_, p, _) = plain_prefix(&conn.c2s());
        inits.push(conn.c2s()[p[1].start..p[1].end].to_vec());
    }
    assert!(keys.windows(2).all(|w| w[0] == w[1]), "one server, one host key");
    inits.sort();
    inits.dedup();
    assert_eq!(inits.len(), 4, "every session has its own client key");
    // Another seed is another server.
    let other = scenarios_with(&[("host_key_seed", Value::Int(2))]);
    let r = run(&other[0].1, SEED, 1).await;
    let conn = check(name, &r);
    assert_ne!(host_key(&conn.s2c()), keys[0]);
}

#[tokio::test(start_paused = true)]
async fn the_banner_variables_reach_the_wire() {
    let all = scenarios_with(&[
        ("client_banner", Value::Str("PuTTY_Release_0.81".into())),
        ("server_banner", Value::Str("dropbear_2022.83".into())),
    ]);
    let r = run(&all[0].1, SEED, 1).await;
    r.result.as_ref().unwrap();
    let conn = session(&r);
    assert!(conn.c2s().starts_with(b"SSH-2.0-PuTTY_Release_0.81\r\n"));
    assert!(conn.s2c().starts_with(b"SSH-2.0-dropbear_2022.83\r\n"));
}

/// The fixed cost of everything except the file or the output: banners, the
/// key exchange, the authentication, the channel setup, and the close.
const FIXED: usize = 4_000;
/// What one channel data packet adds to its payload: the length, the padding
/// length, the message number, the channel, the string length, the padding
/// (at most 11 bytes), and the tag.
const PER_PACKET: usize = 4 + 1 + 9 + 11 + 16;

fn packets(bytes: i64, chunk: i64) -> usize {
    ((bytes + chunk - 1) / chunk) as usize
}

#[tokio::test(start_paused = true)]
async fn the_upload_moves_as_many_bytes_as_the_variable_says() {
    // 0 and 1 byte, a chunk, a chunk plus one byte, exactly one window step,
    // and sizes with one and three window adjustments and a tail.
    for size in [0i64, 1, 32_768, 32_769, 1_048_576, 1_048_577, 3_200_000] {
        let all = scenarios_with(&[("file_bytes", Value::Int(size))]);
        let (name, s) = all.iter().find(|(n, _)| n == "scp_upload").unwrap();
        let r = run(s, SEED, 1).await;
        let conn = check(name, &r);
        let (up, down) = (conn.c2s().len(), conn.s2c().len());
        let n = packets(size, 32_768);
        assert!(up >= size as usize, "{size}: the client wrote {up} bytes");
        assert!(
            up <= size as usize + FIXED + PER_PACKET * (n + 2),
            "{size}: the client wrote {up} bytes"
        );
        // The server sends acknowledgements and window adjustments only.
        let adjusts = (size / 32_768 / 32) as usize;
        assert!(down < FIXED + 40 * (adjusts + 4), "{size}: the server wrote {down} bytes");
    }
}

#[tokio::test(start_paused = true)]
async fn the_download_moves_as_many_bytes_as_the_variable_says() {
    for size in [0i64, 1, 32_768, 100_000, 1_048_576, 2_500_000] {
        let all = scenarios_with(&[("file_bytes", Value::Int(size))]);
        let (name, s) = all.iter().find(|(n, _)| n == "scp_download").unwrap();
        let r = run(s, SEED, 1).await;
        let conn = check(name, &r);
        let (up, down) = (conn.c2s().len(), conn.s2c().len());
        let n = packets(size, 32_768);
        assert!(down >= size as usize, "{size}: the server wrote {down} bytes");
        assert!(
            down <= size as usize + FIXED + PER_PACKET * (n + 2),
            "{size}: the server wrote {down} bytes"
        );
        let adjusts = (size / 32_768 / 32) as usize;
        assert!(up < FIXED + 40 * (adjusts + 4), "{size}: the client wrote {up} bytes");
    }
}

#[tokio::test(start_paused = true)]
async fn the_remote_command_output_follows_the_variable() {
    for size in [0i64, 1, 8_000, 32_768, 100_000, 1_500_000] {
        let all = scenarios_with(&[("output_bytes", Value::Int(size))]);
        let (name, s) = all.iter().find(|(n, _)| n == "remote_command").unwrap();
        let r = run(s, SEED, 1).await;
        let conn = check(name, &r);
        let down = conn.s2c().len();
        let n = packets(size, 32_768);
        assert!(down >= size as usize, "{size}: {down}");
        assert!(down <= size as usize + FIXED + PER_PACKET * n, "{size}: {down}");
    }
}

#[tokio::test(start_paused = true)]
async fn the_shell_session_grows_by_a_fixed_number_of_packets_per_command() {
    // Per command the client sends one packet for each typed key, one for
    // Enter, and one window adjustment. Every one of them has a 10 byte
    // payload, which OpenSSH sends as a 36 byte packet, so the difference of
    // two runs is exact.
    let bytes = |commands: i64, typed: i64| {
        let all = scenarios_with(&[
            ("commands", Value::Int(commands)),
            ("typed", Value::Int(typed)),
        ]);
        let s = all[0].1.clone();
        async move {
            let r = run(&s, SEED, 1).await;
            let conn = check("interactive_shell", &r);
            (conn.c2s().len(), conn.s2c().len())
        }
    };
    let (c1, s1) = bytes(1, 14).await;
    let (c3, s3) = bytes(3, 14).await;
    let (c3b, _) = bytes(3, 20).await;
    let (c0, _) = bytes(0, 14).await;
    assert_eq!(c3 - c1, 2 * (14 + 2) * 36);
    assert_eq!(c1 - c0, (14 + 2) * 36);
    assert_eq!(c3b - c3, 3 * 6 * 36);
    // The server echoes every key and prints output and a prompt.
    assert!(s3 > s1 + 2 * 14 * 36, "{s1} {s3}");
}

#[tokio::test(start_paused = true)]
async fn a_long_shell_session_stays_inside_the_window() {
    let all = scenarios_with(&[
        ("commands", Value::Int(120)),
        ("typed", Value::Int(3)),
        ("think_min_ms", Value::Int(1)),
        ("think_max_ms", Value::Int(2)),
        ("key_gap_min_ms", Value::Int(0)),
        ("key_gap_max_ms", Value::Int(1)),
        ("output_min", Value::Int(20_000)),
        ("output_max", Value::Int(30_000)),
    ]);
    let r = run(&all[0].1, SEED, 1).await;
    let conn = check("interactive_shell", &r);
    // 120 outputs of at least 20 kB are more than the 2 MiB window, so the
    // session passes only because the client grants the output back.
    assert!(conn.s2c().len() > 120 * 20_000);
    assert!(conn.s2c().len() > 2 * 1_048_576);
}

#[tokio::test(start_paused = true)]
async fn sessions_draw_their_own_output() {
    let all = scenarios();
    let (_, s) = all.iter().find(|(n, _)| n == "interactive_shell").unwrap();
    let mut sizes = Vec::new();
    for seed in 1..=8 {
        let r = run(s, seed, 1).await;
        sizes.push(check("interactive_shell", &r).s2c().len());
    }
    sizes.sort_unstable();
    sizes.dedup();
    assert!(sizes.len() >= 6, "{sizes:?}");
}

#[tokio::test(start_paused = true)]
async fn a_bit_flipped_after_the_key_exchange_fails_the_server() {
    let all = scenarios();
    for (name, s) in &all {
        let clean = run(s, SEED, 1).await;
        let (_, _, cipher_at) = plain_prefix(&session(&clean).c2s());
        // The first byte after SSH_MSG_NEWKEYS is the encrypted length of the
        // service request, and the 20th byte lies inside its ciphertext.
        for at in [cipher_at, cipher_at + 20] {
            let r = run_flipping(s, SEED, 1, Some(at)).await;
            assert!(r.result.is_err(), "{name}: a flipped bit at {at} went unnoticed");
            assert!(
                r.records
                    .iter()
                    .any(|x| x.side == Side::Server && x.failed.is_some()),
                "{name}: the server did not fail"
            );
        }
    }
}

#[tokio::test(start_paused = true)]
async fn a_bit_flipped_in_the_client_key_leaves_the_sides_with_different_keys() {
    let all = scenarios();
    let (name, s) = &all[0];
    let clean = run(s, SEED, 1).await;
    let (_, packets, _) = plain_prefix(&session(&clean).c2s());
    // Inside the 32 byte key of SSH_MSG_KEX_ECDH_INIT.
    let at = packets[1].start + 4 + 1 + 1 + 4 + 5;
    let r = run_flipping(s, SEED, 1, Some(at)).await;
    assert!(r.result.is_err(), "{name}");
}
