// SPDX-License-Identifier: GPL-3.0-only
//! Fuzz tests of the frame parsers, the HPACK and Huffman decoders, and every
//! template function.
//!
//! cargo-fuzz needs a nightly toolchain, so these tests are a seeded loop on
//! stable Rust. Every round derives its generator from a seed, so a failure
//! prints the seed and the round that reproduce it.
//!
//! * `fuzz_short` runs a fixed number of rounds in a second or two and runs
//!   with every `cargo test`.
//! * `fuzz_60s` runs rounds for 60 seconds. It is ignored by default; run it
//!   with `cargo test -p fps-module-http2 --release --test fuzz -- --ignored fuzz_60s`.
//!   Set `FPS_FUZZ_SECONDS` to run for another duration.
//!
//! A round checks three kinds of properties.
//!
//! 1. Inputs of any shape, random octets and valid inputs with mutated octets,
//!    make the decoders return a result and never panic or allocate without
//!    bound.
//! 2. Header lists that the encoder writes, with every combination of Huffman
//!    and index mode and table size updates, decode to the same fields on a
//!    decoder that follows the same stream of blocks, and both tables end with
//!    the same size.
//! 3. Every function of the catalog, called with random arguments of the
//!    declared and of other types, returns `Ok` or an error and never panics.

use std::time::{Duration, Instant};

use fps_flow::function::NamedArg;
use fps_flow::{ArgValue, CallArgs, FnCall, TemplateFn, ValueType};
use fps_module_http2::frame::{self, FrameHeader};
use fps_module_http2::hpack::{Decoder, Encoder, HuffmanMode, IndexMode};
use fps_module_http2::{huffman, Http2, FUNCTIONS};
use rand::{Rng, RngCore, SeedableRng};
use rand_pcg::Pcg64Mcg;

const NAMES: &[&str] = &[
    ":method",
    ":path",
    ":status",
    ":authority",
    "accept",
    "accept-encoding",
    "authorization",
    "cookie",
    "set-cookie",
    "content-length",
    "content-type",
    "user-agent",
    "x-request-id",
    "x-custom",
    "etag",
];

fn random_bytes(rng: &mut Pcg64Mcg, max: usize) -> Vec<u8> {
    let len = if rng.gen_bool(0.1) {
        0
    } else {
        rng.gen_range(0..=max)
    };
    match rng.gen_range(0..3) {
        0 => (0..len).map(|_| rng.gen()).collect(),
        // Printable text that Huffman coding shortens.
        1 => (0..len).map(|_| rng.gen_range(b' '..=b'~')).collect(),
        // Few distinct octets, so values repeat and the table matters.
        _ => (0..len).map(|_| b"ab/ 0"[rng.gen_range(0..5)]).collect(),
    }
}

fn random_name(rng: &mut Pcg64Mcg) -> Vec<u8> {
    if rng.gen_bool(0.7) {
        NAMES[rng.gen_range(0..NAMES.len())].as_bytes().to_vec()
    } else {
        let len = rng.gen_range(1..12);
        (0..len).map(|_| rng.gen_range(b'a'..=b'z')).collect()
    }
}

fn random_list(rng: &mut Pcg64Mcg) -> Vec<(Vec<u8>, Vec<u8>)> {
    (0..rng.gen_range(0..12))
        .map(|_| {
            let name = random_name(rng);
            // Repeat a value now and then so exact table matches occur.
            let value = if rng.gen_bool(0.3) {
                b"GET".to_vec()
            } else {
                random_bytes(rng, 40)
            };
            (name, value)
        })
        .collect()
}

fn huffman_mode(rng: &mut Pcg64Mcg) -> HuffmanMode {
    [HuffmanMode::Auto, HuffmanMode::Always, HuffmanMode::Never][rng.gen_range(0..3)]
}

fn index_mode(rng: &mut Pcg64Mcg) -> IndexMode {
    [IndexMode::Auto, IndexMode::Off, IndexMode::All][rng.gen_range(0..3)]
}

/// Flips, inserts, deletes, and truncates octets.
fn mutate(rng: &mut Pcg64Mcg, data: &[u8]) -> Vec<u8> {
    let mut v = data.to_vec();
    for _ in 0..rng.gen_range(1..4) {
        match (rng.gen_range(0..5), v.is_empty()) {
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
            (_, false) => {
                let n = rng.gen_range(0..v.len());
                v.truncate(n);
            }
            _ => v.push(rng.gen()),
        }
    }
    v
}

/// Property 2, and the mutated blocks of property 1.
fn hpack_round(rng: &mut Pcg64Mcg) {
    let table = if rng.gen_bool(0.5) {
        4096
    } else {
        rng.gen_range(0..600)
    };
    let mut enc = Encoder::with_max_size(table);
    let mut dec = Decoder::with_max_size(table);
    let mut blocks = Vec::new();
    for _ in 0..rng.gen_range(1..6) {
        let mut block = Vec::new();
        if rng.gen_bool(0.15) {
            // The decoder allows the size that its caller set, so stay inside.
            enc.set_max_size(rng.gen_range(0..=table), &mut block);
        }
        let list = random_list(rng);
        let (h, i) = (huffman_mode(rng), index_mode(rng));
        for (n, v) in &list {
            enc.encode(n, v, h, i, &mut block);
        }
        let got = dec
            .decode(&block)
            .unwrap_or_else(|e| panic!("own block does not decode: {e:?}"));
        assert_eq!(got.len(), list.len());
        for (g, (n, v)) in got.iter().zip(&list) {
            assert_eq!((&g.name, &g.value), (n, v));
        }
        assert_eq!(enc.size(), dec.size(), "tables diverged");
        assert_eq!(enc.len(), dec.len(), "tables diverged");
        assert!(dec.size() <= dec.max_size());
        blocks.push(block);
    }
    // The same blocks with damage, into a decoder in a random state.
    let mut victim = Decoder::with_max_size(table);
    for b in &blocks {
        let _ = victim.decode(&mutate(rng, b));
        assert!(victim.size() <= victim.max_size());
    }
}

/// Property 1 for the decoders and parsers.
fn bytes_round(rng: &mut Pcg64Mcg) {
    let data = random_bytes(rng, 300);
    let _ = Decoder::new().decode(&data);
    let _ = huffman::decode(&data, &mut Vec::new());
    let _ = FrameHeader::parse(&data);
    let _ = frame::split(&data);
    let _ = frame::settings_pairs(&data);
    let _ = frame::strip_padding(rng.gen(), &data);
    let _ = frame::block_fragment(rng.gen_range(0..12), rng.gen(), &data);

    // A Huffman string that the encoder wrote, damaged.
    let s = random_bytes(rng, 60);
    let mut coded = Vec::new();
    huffman::encode(&s, &mut coded);
    assert_eq!(coded.len(), huffman::encoded_len(&s));
    let mut back = Vec::new();
    huffman::decode(&coded, &mut back).unwrap();
    assert_eq!(back, s);
    let _ = huffman::decode(&mutate(rng, &coded), &mut Vec::new());

    // A stream of frames, damaged, and split the way a reader does.
    let mut stream = Vec::new();
    let _ = frame::data(&mut stream, 1, &s, rng.gen(), None, 16384);
    let _ = frame::ping(&mut stream, rng.gen(), rng.gen());
    let _ = frame::goaway(&mut stream, rng.gen_range(0..100), 0, &s);
    let _ = frame::window_update(&mut stream, 3, rng.gen_range(0..1000));
    let mut rest = &mutate(rng, &stream)[..];
    while let Ok((_, tail)) = frame::split(rest) {
        rest = tail;
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
        ][rng.gen_range(0..6)],
        _ => rng.gen(),
    }
}

/// An argument of the declared type, or now and then of another type.
fn arg_of(
    rng: &mut Pcg64Mcg,
    ty: ValueType,
    store: &mut Vec<Vec<u8>>,
    texts: &mut Vec<String>,
) -> usize {
    // Returns the index of the arg kind: the caller builds the ArgValue from
    // the stores, which keeps the borrows simple.
    let kind = if rng.gen_bool(0.85) {
        match ty {
            ValueType::Int => 0,
            ValueType::Boolean => 1,
            ValueType::Text => 2,
            _ => 3,
        }
    } else {
        rng.gen_range(0..4)
    };
    match kind {
        2 => {
            let t = if rng.gen_bool(0.6) {
                [
                    "auto",
                    "off",
                    "all",
                    "always",
                    "never",
                    "NO_ERROR",
                    "stream_id",
                    "length",
                    "flags",
                    "initial_window_size",
                    "end_headers",
                    ":path",
                    "CANCEL",
                ][rng.gen_range(0..13)]
                .to_string()
            } else {
                String::from_utf8_lossy(&random_bytes(rng, 10)).into_owned()
            };
            texts.push(t);
        }
        3 => {
            // A header list, a frame, or noise.
            let b = match rng.gen_range(0..4) {
                0 => {
                    let mut s = Vec::new();
                    for (n, v) in random_list(rng) {
                        if v.iter().all(|c| !matches!(c, b'\n' | b'\r' | 0)) {
                            s.extend_from_slice(&n);
                            s.extend_from_slice(b": ");
                            s.extend_from_slice(&v);
                            s.push(b'\n');
                        }
                    }
                    s
                }
                1 => {
                    let mut f = Vec::new();
                    let _ = frame::data(&mut f, 1, &random_bytes(rng, 30), false, None, 16384);
                    f
                }
                2 => {
                    let mut f = Vec::new();
                    let _ = frame::settings(&mut f, &[(rng.gen_range(1..7), rng.gen())]);
                    f
                }
                _ => random_bytes(rng, 40),
            };
            store.push(b);
        }
        _ => {}
    }
    kind
}

/// Property 3.
fn call_round(rng: &mut Pcg64Mcg, h: &mut Http2) {
    let spec = &FUNCTIONS[rng.gen_range(0..FUNCTIONS.len())];
    let n = rng.gen_range(0..=spec.params.len());
    let mut store = Vec::new();
    let mut texts = Vec::new();
    let mut kinds = Vec::new();
    let mut ints = Vec::new();
    let mut bools = Vec::new();
    for p in &spec.params[..n] {
        kinds.push(arg_of(rng, p.ty, &mut store, &mut texts));
        ints.push(int_arg(rng));
        bools.push(rng.gen_bool(0.5));
    }
    let (mut si, mut ti) = (0, 0);
    let mut values = Vec::new();
    for (i, k) in kinds.iter().enumerate() {
        values.push(match k {
            0 => ArgValue::Int(ints[i]),
            1 => ArgValue::Boolean(bools[i]),
            2 => {
                ti += 1;
                ArgValue::Text(&texts[ti - 1])
            }
            _ => {
                si += 1;
                ArgValue::Bytes(&store[si - 1])
            }
        });
    }
    // Some arguments by name, the others by position.
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
    let call = FnCall {
        module: "http2",
        function: spec.name,
        args: CallArgs {
            positional: &values[..split],
            named: &named,
        },
    };
    let mut out = Vec::new();
    // Any result is fine. A panic or an unbounded output is not.
    let _ = h.render(&call, rng, &mut out);
    assert!(
        out.len() < 64 << 20,
        "{} wrote {} octets",
        spec.name,
        out.len()
    );
}

fn round(seed: u64, rounds: u64) {
    for r in 0..rounds {
        let mut rng = Pcg64Mcg::seed_from_u64(seed ^ r.wrapping_mul(0x9E37_79B9_7F4A_7C15));
        let ctx = format!("seed {seed} round {r}");
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            hpack_round(&mut rng);
            for _ in 0..8 {
                bytes_round(&mut rng);
            }
            // One module value for the whole round, so its HPACK state evolves
            // across calls as it does on a connection.
            let mut h = Http2::default();
            for _ in 0..40 {
                call_round(&mut rng, &mut h);
            }
        }));
        if let Err(e) = result {
            eprintln!("fuzz failure, reproduce with {ctx}");
            std::panic::resume_unwind(e);
        }
    }
}

fn run_for(seconds: u64, seed: u64) -> u64 {
    let end = Instant::now() + Duration::from_secs(seconds);
    let mut rounds = 0;
    let mut next = seed;
    while Instant::now() < end {
        round(next, 50);
        rounds += 50;
        next = next.wrapping_add(1);
    }
    rounds
}

#[test]
fn fuzz_short() {
    round(0xF005_BA11, 300);
}

#[test]
fn rounds_are_reproducible() {
    // The same seed draws the same inputs, so a failure can be replayed.
    let mut a = Pcg64Mcg::seed_from_u64(5);
    let mut b = Pcg64Mcg::seed_from_u64(5);
    assert_eq!(random_list(&mut a), random_list(&mut b));
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
    let rounds = run_for(seconds, seed);
    eprintln!("fuzz_60s: {rounds} rounds in {seconds} s, no panic");
}
