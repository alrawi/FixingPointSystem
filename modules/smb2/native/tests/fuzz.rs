// SPDX-License-Identifier: GPL-3.0-only
//! Fuzz tests of the message readers, the NTLM parsers, and every template
//! function.
//!
//! cargo-fuzz needs a nightly toolchain, so these tests are a seeded loop on
//! stable Rust. Every round derives its generator from a seed, so a failure
//! prints the seed and the round that reproduce it.
//!
//! * `fuzz_short` runs a fixed number of rounds in a few seconds and runs with
//!   every `cargo test`.
//! * `fuzz_60s` runs rounds for 60 seconds. It is ignored by default; run it
//!   with `cargo test -p fps-module-smb2 --release --test fuzz -- --ignored fuzz_60s`.
//!   Set `FPS_FUZZ_SECONDS` to run for another duration.
//!
//! A round checks two kinds of properties.
//!
//! 1. Bytes of any shape, random octets and valid messages with mutated
//!    octets, make every reader return a result and never panic. The readers
//!    are the ones that a flow points at bytes of the peer.
//! 2. Every function of the catalog, called with random arguments of the
//!    declared and of other types, returns `Ok` or an error and never panics
//!    or writes without bound.

mod common;

use std::time::{Duration, Instant};

use fps_flow::function::NamedArg;
use fps_flow::{ArgValue, CallArgs, FnCall, TemplateFn, ValueType};
use fps_module_smb2::{ntlm, wire, Smb2, FUNCTIONS};
use rand::{Rng, RngCore, SeedableRng};
use rand_pcg::Pcg64Mcg;

const READERS: &[&str] = &[
    "message_length",
    "header_field",
    "body_field",
    "security_blob",
    "ntlm_field",
    "data",
    "ntlm_session_key",
    "sign",
    "hex",
    "unhex",
];

const WORDS: &[&str] = &[
    "command",
    "status",
    "message_id",
    "session_id",
    "tree_id",
    "flags",
    "path",
    "name",
    "file_id",
    "length",
    "offset",
    "dialect",
    "dialects",
    "entries",
    "pattern",
    "user",
    "domain",
    "challenge",
    "type",
    "time",
    "success",
    "more_processing_required",
    "access_denied",
    "create",
    "read",
    "3.1.1",
    "2.1",
    "pipe",
    "disk",
    "enabled",
    "required",
    "\\\\host\\share",
    "docs\\a.txt",
    "0 directory d\n12 file f.txt",
    "00112233445566778899aabbccddeeff",
];

fn random_bytes(rng: &mut Pcg64Mcg, max: usize) -> Vec<u8> {
    let len = if rng.gen_bool(0.1) {
        0
    } else {
        rng.gen_range(0..=max)
    };
    (0..len).map(|_| rng.gen()).collect()
}

/// Flips, inserts, deletes, and truncates octets.
fn mutate(rng: &mut Pcg64Mcg, data: &[u8]) -> Vec<u8> {
    let mut v = data.to_vec();
    for _ in 0..rng.gen_range(1..5) {
        match (rng.gen_range(0..6), v.is_empty()) {
            (0, false) => {
                let i = rng.gen_range(0..v.len());
                v[i] ^= 1 << rng.gen_range(0..8);
            }
            (1, false) => {
                let i = rng.gen_range(0..v.len());
                v[i] = rng.gen();
            }
            (2, _) => {
                let i = rng.gen_range(0..=v.len());
                v.insert(i, rng.gen());
            }
            (3, false) => {
                let i = rng.gen_range(0..v.len());
                v.remove(i);
            }
            (4, false) => {
                // A field of 2 or 4 octets takes an extreme value.
                let i = rng.gen_range(0..v.len());
                for b in v.iter_mut().skip(i).take(rng.gen_range(1..5)) {
                    *b = [0x00, 0xFF, 0x7F, 0x80][rng.gen_range(0..4)];
                }
            }
            (_, false) => {
                let n = rng.gen_range(0..v.len());
                v.truncate(n);
            }
            _ => v.push(rng.gen()),
        }
    }
    v
}

fn call_with(
    rng: &mut Pcg64Mcg,
    function: &str,
    positional: &[ArgValue<'_>],
    named: &[NamedArg<'_>],
) -> Vec<u8> {
    let mut out = Vec::new();
    let c = FnCall {
        module: "smb2",
        function,
        args: CallArgs { positional, named },
    };
    // Any result is fine. A panic or an unbounded output is not.
    let _ = Smb2.render(&c, rng, &mut out);
    assert!(
        out.len() < 64 << 20,
        "{function} wrote {} octets",
        out.len()
    );
    out
}

/// Property 1: the readers on random and mutated bytes.
fn bytes_round(rng: &mut Pcg64Mcg, corpus: &[Vec<u8>]) {
    let input = match rng.gen_range(0..4) {
        0 => random_bytes(rng, 200),
        1 => {
            // Random octets behind a valid header start.
            let mut v = wire::PROTOCOL_ID.to_vec();
            v.extend_from_slice(&64u16.to_le_bytes());
            v.extend(random_bytes(rng, 300));
            v
        }
        _ => {
            let i = rng.gen_range(0..corpus.len());
            mutate(rng, &corpus[i])
        }
    };
    for f in READERS {
        let field = WORDS[rng.gen_range(0..WORDS.len())];
        let key = WORDS[rng.gen_range(0..WORDS.len())];
        let args = [ArgValue::Bytes(&input), ArgValue::Text(field)];
        let one = [ArgValue::Bytes(&input)];
        let two = [ArgValue::Bytes(&input), ArgValue::Text(key)];
        let positional: &[ArgValue<'_>] = match *f {
            "header_field" | "body_field" | "ntlm_field" => &args,
            "ntlm_session_key" | "sign" => &two,
            _ => &one,
        };
        call_with(rng, f, positional, &[]);
    }
    // The parsers behind them, called directly on the same bytes.
    let _ = wire::Message::parse(&input);
    let _ = wire::parse_directory_information(&input);
    let _ = wire::transport_length(&input);
    if let Ok(msg) = ntlm::find_message(&input) {
        let _ = ntlm::parse_challenge(msg);
        let _ = ntlm::parse_authenticate(msg);
        let _ = ntlm::field(msg, WORDS[rng.gen_range(0..WORDS.len())]);
    }
    // A mutated message that still parses must give its fields without a panic.
    if let Ok(m) = wire::Message::parse(&input) {
        for f in [
            "command",
            "status",
            "message_id",
            "session_id",
            "tree_id",
            "credits",
            "flags",
        ] {
            let _ = fps_module_smb2::read::header_field(&m, f);
        }
        let _ = fps_module_smb2::read::security_blob(&m);
        let _ = fps_module_smb2::read::data(&m);
        for f in WORDS {
            let _ = fps_module_smb2::read::body_field(&m, f);
        }
    }
}

fn int_arg(rng: &mut Pcg64Mcg) -> i64 {
    match rng.gen_range(0..6) {
        0 => rng.gen_range(-2..3),
        1 => rng.gen_range(0..300),
        2 => rng.gen_range(0..70000),
        3 => i64::from(rng.gen::<u32>()),
        4 => [
            i64::MIN,
            i64::MAX,
            0x7fff_ffff,
            0x8000_0000,
            16_777_215,
            16_777_216,
            65_535,
            65_536,
        ][rng.gen_range(0..8)],
        _ => rng.gen(),
    }
}

/// Property 2: one call with random arguments.
fn call_round(rng: &mut Pcg64Mcg, corpus: &[Vec<u8>]) {
    let spec = &FUNCTIONS[rng.gen_range(0..FUNCTIONS.len())];
    let n = rng.gen_range(0..=spec.params.len());
    // Draw every argument first and keep them alive for the call.
    let mut ints = Vec::new();
    let mut texts = Vec::new();
    let mut blobs = Vec::new();
    let mut bools = Vec::new();
    let mut kinds = Vec::new();
    for p in &spec.params[..n] {
        let declared = match p.ty {
            ValueType::Int => 0,
            ValueType::Boolean => 1,
            ValueType::Text => 2,
            _ => 3,
        };
        kinds.push(if rng.gen_bool(0.85) {
            declared
        } else {
            rng.gen_range(0..4)
        });
        ints.push(int_arg(rng));
        bools.push(rng.gen_bool(0.5));
        texts.push(if rng.gen_bool(0.7) {
            WORDS[rng.gen_range(0..WORDS.len())].to_owned()
        } else {
            String::from_utf8_lossy(&random_bytes(rng, 12)).into_owned()
        });
        blobs.push(match rng.gen_range(0..3) {
            0 => random_bytes(rng, 60),
            _ => {
                let i = rng.gen_range(0..corpus.len());
                mutate(rng, &corpus[i])
            }
        });
    }
    let values: Vec<ArgValue<'_>> = (0..n)
        .map(|i| match kinds[i] {
            0 => ArgValue::Int(ints[i]),
            1 => ArgValue::Boolean(bools[i]),
            2 => ArgValue::Text(&texts[i]),
            _ => ArgValue::Bytes(&blobs[i]),
        })
        .collect();
    let split = if rng.gen_bool(0.5) {
        n
    } else {
        rng.gen_range(0..=n)
    };
    let named: Vec<NamedArg<'_>> = spec.params[split..n]
        .iter()
        .zip(&values[split..])
        .map(|(p, v)| NamedArg {
            name: p.name,
            value: *v,
        })
        .collect();
    call_with(rng, spec.name, &values[..split], &named);
}

/// Valid messages and NTLM tokens that the mutations start from.
fn corpus() -> Vec<Vec<u8>> {
    let mut c: Vec<Vec<u8>> = common::session("2.0.2,2.1,3.0,3.0.2,3.1.1", "3.1.1", 300)
        .into_iter()
        .map(|(_, m)| m)
        .collect();
    c.extend(
        common::session("2.1", "2.1", 10)
            .into_iter()
            .map(|(_, m)| m[4..].to_vec()),
    );
    let ch = common::ok("ntlm_challenge", &[], &[]);
    c.push(common::ok(
        "ntlm_authenticate",
        &[common::bytes(&ch), common::text("u"), common::text("p")],
        &[],
    ));
    c.push(ch);
    c.push(common::ok("ntlm_negotiate", &[], &[]));
    c
}

fn round(seed: u64, rounds: u64, corpus: &[Vec<u8>]) {
    for r in 0..rounds {
        let mut rng = Pcg64Mcg::seed_from_u64(seed ^ r.wrapping_mul(0x9E37_79B9_7F4A_7C15));
        let ctx = format!("seed {seed} round {r}");
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            for _ in 0..20 {
                bytes_round(&mut rng, corpus);
            }
            for _ in 0..40 {
                call_round(&mut rng, corpus);
            }
        }));
        if let Err(e) = result {
            eprintln!("fuzz failure, reproduce with {ctx}");
            std::panic::resume_unwind(e);
        }
    }
}

fn run_for(seconds: u64, seed: u64, corpus: &[Vec<u8>]) -> u64 {
    let end = Instant::now() + Duration::from_secs(seconds);
    let mut rounds = 0;
    let mut next = seed;
    while Instant::now() < end {
        round(next, 50, corpus);
        rounds += 50;
        next = next.wrapping_add(1);
    }
    rounds
}

#[test]
fn fuzz_short() {
    round(0xF005_BA11, 150, &corpus());
}

#[test]
fn rounds_are_reproducible() {
    // The same seed draws the same inputs, so a failure can be replayed.
    let mut a = Pcg64Mcg::seed_from_u64(5);
    let mut b = Pcg64Mcg::seed_from_u64(5);
    let c = corpus();
    assert_eq!(mutate(&mut a, &c[3]), mutate(&mut b, &c[3]));
    assert_eq!(a.next_u64(), b.next_u64());
}

#[test]
#[ignore = "runs for 60 seconds, use --release and --ignored"]
fn fuzz_60s() {
    let seconds = std::env::var("FPS_FUZZ_SECONDS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(60);
    let seed = std::env::var("FPS_FUZZ_SEED")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1);
    let rounds = run_for(seconds, seed, &corpus());
    eprintln!("fuzz_60s: {rounds} rounds in {seconds} s, no panic");
}
