// SPDX-License-Identifier: GPL-3.0-only
//! Tests of the content generators: the duplicate payload ratio over 10,000
//! bodies per generator type, determinism for a given seed, the size
//! parameter, and the framing of every output format.

use std::collections::HashSet;

use fps_flow::function::NamedArg;
use fps_flow::{ArgValue, CallArgs, FnCall, TemplateFn};
use fps_module_content::{Content, BINARY_KINDS, LANGUAGES};

const BODIES: usize = 10_000;

/// The acceptance bound of MOD-03: at most 1% of the bodies repeat an earlier one.
const MAX_DUPLICATE_RATIO: f64 = 0.01;

type Rng = rand_pcg::Pcg64Mcg;

fn session(seed: u64) -> Rng {
    Rng::new(u128::from(seed) | 1 << 100)
}

fn render(function: &str, rng: &mut Rng, named: &[NamedArg<'_>]) -> Vec<u8> {
    let mut out = Vec::new();
    let c = FnCall {
        module: "content",
        function,
        args: CallArgs {
            positional: &[],
            named,
        },
    };
    Content.render(&c, rng, &mut out).unwrap();
    out
}

fn kw<'a>(name: &'a str, value: ArgValue<'a>) -> NamedArg<'a> {
    NamedArg { name, value }
}

/// One generator under test: a function and its fixed arguments.
struct Case {
    label: String,
    function: &'static str,
    lang: Option<&'static str>,
    kind: Option<&'static str>,
}

fn cases() -> Vec<Case> {
    let mut v = vec![
        Case {
            label: "html".into(),
            function: "html",
            lang: None,
            kind: None,
        },
        Case {
            label: "json".into(),
            function: "json",
            lang: None,
            kind: None,
        },
    ];
    for lang in LANGUAGES {
        for function in ["text", "email"] {
            v.push(Case {
                label: format!("{function}:{lang}"),
                function,
                lang: Some(lang),
                kind: None,
            });
        }
    }
    for kind in BINARY_KINDS {
        v.push(Case {
            label: format!("binary:{kind}"),
            function: "binary",
            lang: None,
            kind: Some(kind),
        });
    }
    v
}

impl Case {
    fn call(&self, rng: &mut Rng, size: Option<i64>, seed: Option<i64>) -> Vec<u8> {
        let mut named = Vec::new();
        if let Some(l) = self.lang {
            named.push(kw("lang", ArgValue::Text(l)));
        }
        if let Some(k) = self.kind {
            named.push(kw("kind", ArgValue::Text(k)));
        }
        if let Some(s) = size {
            named.push(kw("size", ArgValue::Int(s)));
        }
        if let Some(s) = seed {
            named.push(kw("seed", ArgValue::Int(s)));
        }
        render(self.function, rng, &named)
    }
}

fn duplicate_ratio(bodies: impl Iterator<Item = Vec<u8>>) -> f64 {
    let mut seen = HashSet::new();
    let mut total = 0usize;
    for b in bodies {
        total += 1;
        seen.insert(b);
    }
    (total - seen.len()) as f64 / total as f64
}

#[test]
fn duplicate_ratio_over_10000_session_bodies_is_at_most_one_percent() {
    for case in cases() {
        // The seed stays unset, so the session generator varies the bodies.
        let mut rng = session(1);
        let ratio = duplicate_ratio((0..BODIES).map(|_| case.call(&mut rng, None, None)));
        assert!(
            ratio <= MAX_DUPLICATE_RATIO,
            "{}: duplicate ratio {ratio}",
            case.label
        );
    }
}

#[test]
fn duplicate_ratio_over_10000_explicit_seeds_is_at_most_one_percent() {
    for case in cases() {
        let mut rng = session(2);
        let ratio = duplicate_ratio((0..BODIES as i64).map(|s| case.call(&mut rng, None, Some(s))));
        assert!(
            ratio <= MAX_DUPLICATE_RATIO,
            "{}: duplicate ratio {ratio}",
            case.label
        );
    }
}

#[test]
fn duplicate_ratio_holds_for_small_bodies() {
    for case in cases() {
        let mut rng = session(3);
        let ratio = duplicate_ratio((0..BODIES).map(|_| case.call(&mut rng, Some(512), None)));
        assert!(
            ratio <= MAX_DUPLICATE_RATIO,
            "{}: duplicate ratio {ratio} at 512 bytes",
            case.label
        );
    }
}

#[test]
fn bodies_are_deterministic_for_a_given_seed() {
    for case in cases() {
        // Without a seed argument, the session generator decides.
        let a = case.call(&mut session(7), None, None);
        let b = case.call(&mut session(7), None, None);
        assert_eq!(a, b, "{}: same session seed", case.label);
        let c = case.call(&mut session(8), None, None);
        assert_ne!(a, c, "{}: other session seed", case.label);
        // With a seed argument, the session generator does not matter.
        let a = case.call(&mut session(7), None, Some(42));
        let b = case.call(&mut session(99), None, Some(42));
        assert_eq!(a, b, "{}: same seed argument", case.label);
        let c = case.call(&mut session(7), None, Some(43));
        assert_ne!(a, c, "{}: other seed argument", case.label);
    }
}

#[test]
fn a_session_replays_a_whole_sequence_of_calls() {
    let run = |seed| {
        let mut rng = session(seed);
        cases()
            .iter()
            .flat_map(|c| c.call(&mut rng, Some(700), None))
            .collect::<Vec<u8>>()
    };
    assert_eq!(run(5), run(5));
}

#[test]
fn bodies_honour_the_size_parameter() {
    for case in cases() {
        for size in [0usize, 1, 100, 1024, 2048, 10_000, 100_000, 300_000] {
            let mut rng = session(size as u64);
            let b = case.call(&mut rng, Some(size as i64), Some(1));
            assert!(
                b.len() >= size,
                "{} at {size}: {} bytes",
                case.label,
                b.len()
            );
            if case.kind == Some("bin") {
                assert_eq!(b.len(), size, "{}", case.label);
            } else if size >= 2048 {
                assert!(
                    b.len() <= size + 1024,
                    "{} at {size}: {} bytes",
                    case.label,
                    b.len()
                );
            }
        }
    }
}

#[test]
fn the_largest_size_is_accepted() {
    let mut rng = session(1);
    let b = render(
        "binary",
        &mut rng,
        &[
            kw("kind", ArgValue::Text("jpeg")),
            kw("size", ArgValue::Int(16 * 1024 * 1024)),
            kw("seed", ArgValue::Int(1)),
        ],
    );
    assert!(b.len() >= 16 * 1024 * 1024);
}

fn text_of(b: Vec<u8>) -> String {
    String::from_utf8(b).unwrap()
}

#[test]
fn html_pages_are_well_formed() {
    for seed in 0..50 {
        let mut rng = session(seed);
        let s = text_of(render(
            "html",
            &mut rng,
            &[
                kw("size", ArgValue::Int(20_000)),
                kw("seed", ArgValue::Int(seed as i64)),
            ],
        ));
        assert!(s.starts_with("<!DOCTYPE html>\n<html lang=\"en\">"));
        assert!(s.ends_with("</body>\n</html>\n"));
        for tag in [
            "section", "ul", "table", "tbody", "main", "header", "footer", "p",
        ] {
            let open =
                s.matches(&format!("<{tag}>")).count() + s.matches(&format!("<{tag} ")).count();
            let close = s.matches(&format!("</{tag}>")).count();
            assert_eq!(open, close, "<{tag}> in seed {seed}");
        }
    }
}

#[test]
fn json_documents_parse_and_hold_records() {
    for seed in 0..50 {
        let mut rng = session(seed);
        let b = render(
            "json",
            &mut rng,
            &[
                kw("size", ArgValue::Int(30_000)),
                kw("seed", ArgValue::Int(seed as i64)),
            ],
        );
        let v: serde_json::Value = serde_json::from_slice(&b).unwrap();
        assert_eq!(v["status"], "ok");
        let data = v["data"].as_array().unwrap();
        assert!(data.len() > 10);
        for item in data {
            assert!(item["id"].as_str().unwrap().len() == 12);
            assert!(item["owner"]["email"]
                .as_str()
                .unwrap()
                .ends_with("@example.test"));
        }
    }
    // An empty size still gives a valid document.
    let b = render("json", &mut session(1), &[kw("size", ArgValue::Int(0))]);
    let v: serde_json::Value = serde_json::from_slice(&b).unwrap();
    assert!(v["data"].as_array().unwrap().is_empty());
}

#[test]
fn text_uses_the_script_of_its_language() {
    let script = |lang: &str, f: fn(char) -> bool| {
        let mut rng = session(1);
        let s = text_of(render(
            "text",
            &mut rng,
            &[
                kw("lang", ArgValue::Text(lang)),
                kw("size", ArgValue::Int(2000)),
            ],
        ));
        assert!(s.chars().filter(|c| f(*c)).count() > 200, "{lang}");
        assert!(s.ends_with('\n') && !s.ends_with("\n\n"), "{lang}");
    };
    script("ru", |c| ('\u{400}'..='\u{4FF}').contains(&c));
    script("zh", |c| ('\u{4E00}'..='\u{9FFF}').contains(&c));
    for l in ["en", "de", "fr", "es", "it"] {
        script(l, |c| c.is_ascii_alphabetic());
    }
}

#[test]
fn emails_have_the_expected_parts_and_no_real_addresses() {
    for lang in LANGUAGES {
        let mut rng = session(4);
        let s = text_of(render(
            "email",
            &mut rng,
            &[
                kw("lang", ArgValue::Text(lang)),
                kw("size", ArgValue::Int(3000)),
            ],
        ));
        assert!(s.contains("\nRef: "), "{lang}");
        assert!(s.contains("@example.test\n"), "{lang}");
        let at = s.matches('@').count();
        assert_eq!(at, s.matches("@example.test").count(), "{lang}");
    }
}

#[test]
fn links_stay_in_the_reserved_test_domain() {
    for function in ["html", "json"] {
        let mut rng = session(6);
        let s = text_of(render(
            function,
            &mut rng,
            &[kw("size", ArgValue::Int(50_000))],
        ));
        for part in s.split("https://").skip(1) {
            let host: String = part
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '.' || *c == '-')
                .collect();
            assert!(host.ends_with(".example.test"), "{function}: {host}");
        }
    }
}

// Format checks for the binary kinds.

fn file(kind: &str, size: i64, seed: i64) -> Vec<u8> {
    render(
        "binary",
        &mut session(1),
        &[
            kw("kind", ArgValue::Text(kind)),
            kw("size", ArgValue::Int(size)),
            kw("seed", ArgValue::Int(seed)),
        ],
    )
}

fn crc32(data: &[u8]) -> u32 {
    let mut c = 0xFFFF_FFFFu32;
    for &b in data {
        c ^= u32::from(b);
        for _ in 0..8 {
            c = if c & 1 == 1 {
                0xEDB8_8320 ^ (c >> 1)
            } else {
                c >> 1
            };
        }
    }
    !c
}

fn be32(b: &[u8]) -> usize {
    u32::from_be_bytes([b[0], b[1], b[2], b[3]]) as usize
}

fn le16(b: &[u8]) -> usize {
    usize::from(u16::from_le_bytes([b[0], b[1]]))
}

fn le32(b: &[u8]) -> usize {
    u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize
}

/// Reads stored deflate blocks and returns the data and the bytes consumed.
fn inflate_stored(mut s: &[u8]) -> (Vec<u8>, usize) {
    let start = s.len();
    let mut out = Vec::new();
    loop {
        let last = s[0] & 1 == 1;
        assert_eq!(s[0] >> 1, 0, "block type must be stored");
        let len = le16(&s[1..]);
        assert_eq!(len ^ 0xFFFF, le16(&s[3..]));
        out.extend_from_slice(&s[5..5 + len]);
        s = &s[5 + len..];
        if last {
            return (out, start - s.len());
        }
    }
}

#[test]
fn png_files_have_valid_chunks() {
    for seed in 0..20 {
        let f = file("png", 30_000, seed);
        assert_eq!(&f[..8], b"\x89PNG\r\n\x1a\n");
        let mut pos = 8;
        let mut kinds = Vec::new();
        let (mut w, mut h) = (0, 0);
        let mut idat = Vec::new();
        while pos < f.len() {
            let len = be32(&f[pos..]);
            let kind = &f[pos + 4..pos + 8];
            let data = &f[pos + 8..pos + 8 + len];
            let crc = be32(&f[pos + 8 + len..]);
            assert_eq!(
                crc as u32,
                crc32(&f[pos + 4..pos + 8 + len]),
                "crc of {kind:?}"
            );
            if kind == b"IHDR" {
                w = be32(data);
                h = be32(&data[4..]);
                assert_eq!(&data[8..], &[8, 2, 0, 0, 0]);
            }
            if kind == b"IDAT" {
                idat.extend_from_slice(data);
            }
            kinds.push(kind.to_vec());
            pos += 12 + len;
        }
        assert_eq!(pos, f.len());
        assert_eq!(kinds.first().unwrap(), b"IHDR");
        assert_eq!(kinds.last().unwrap(), b"IEND");
        assert_eq!(&idat[..2], &[0x78, 0x01]);
        let (raw, used) = inflate_stored(&idat[2..]);
        assert_eq!(raw.len(), h * (1 + 3 * w));
        assert_eq!(idat.len(), 2 + used + 4);
        // The zlib trailer is the Adler-32 checksum of the raw data.
        let (mut a, mut b) = (1u32, 0u32);
        for &x in &raw {
            a = (a + u32::from(x)) % 65_521;
            b = (b + a) % 65_521;
        }
        assert_eq!(be32(&idat[2 + used..]) as u32, (b << 16) | a);
    }
}

#[test]
fn gzip_files_carry_a_checked_payload() {
    for size in [100, 5000, 200_000] {
        let f = file("gzip", size, 3);
        assert_eq!(&f[..4], &[0x1F, 0x8B, 8, 0]);
        let (data, used) = inflate_stored(&f[10..]);
        assert_eq!(f.len(), 10 + used + 8);
        assert_eq!(le32(&f[10 + used..]) as u32, crc32(&data));
        assert_eq!(le32(&f[14 + used..]), data.len());
        assert!(data.is_ascii());
    }
}

#[test]
fn zip_files_have_consistent_headers() {
    for size in [100, 5000, 200_000] {
        let f = file("zip", size, 3);
        assert_eq!(&f[..4], b"PK\x03\x04");
        let (crc, csize, usize_, nlen) = (
            le32(&f[14..]),
            le32(&f[18..]),
            le32(&f[22..]),
            le16(&f[26..]),
        );
        assert_eq!(csize, usize_);
        let data = &f[30 + nlen..30 + nlen + csize];
        assert_eq!(crc as u32, crc32(data));
        let name = &f[30..30 + nlen];
        assert!(name.starts_with(b"report-") && name.ends_with(b".txt"));
        let eocd = f.len() - 22;
        assert_eq!(&f[eocd..eocd + 4], b"PK\x05\x06");
        let (cd_size, cd_off) = (le32(&f[eocd + 12..]), le32(&f[eocd + 16..]));
        assert_eq!(cd_off, 30 + nlen + csize);
        assert_eq!(cd_off + cd_size, eocd);
        assert_eq!(&f[cd_off..cd_off + 4], b"PK\x01\x02");
        assert_eq!(le32(&f[cd_off + 16..]), crc);
        assert_eq!(le32(&f[cd_off + 42..]), 0, "local header offset");
    }
}

#[test]
fn pdf_files_have_a_correct_cross_reference_table() {
    for size in [100, 5000, 200_000] {
        let f = file("pdf", size, 3);
        let s: Vec<u8> = f.clone();
        assert!(s.starts_with(b"%PDF-1.4\n"));
        assert!(s.ends_with(b"%%EOF\n"));
        let text = String::from_utf8_lossy(&s);
        let startxref: usize = text
            .rsplit("startxref\n")
            .next()
            .unwrap()
            .lines()
            .next()
            .unwrap()
            .parse()
            .unwrap();
        assert!(s[startxref..].starts_with(b"xref\n0 6\n"));
        for n in 1..=5 {
            let entry = startxref + b"xref\n0 6\n".len() + n * 20;
            let off: usize = std::str::from_utf8(&s[entry..entry + 10])
                .unwrap()
                .parse()
                .unwrap();
            assert!(
                s[off..].starts_with(format!("{n} 0 obj\n").as_bytes()),
                "object {n}"
            );
        }
        // The stream length matches the bytes between the stream keywords.
        let a = text.find("<< /Length ").unwrap() + 11;
        let len: usize = text[a..]
            .split_whitespace()
            .next()
            .unwrap()
            .parse()
            .unwrap();
        let begin = text.find("stream\n").unwrap() + 7;
        assert_eq!(&text[begin + len..begin + len + 9], "endstream");
    }
}

#[test]
fn gif_files_decode_to_the_declared_pixel_count() {
    for size in [100, 5000, 200_000, 3_000_000] {
        let f = file("gif", size, 3);
        assert_eq!(&f[..6], b"GIF89a");
        let (w, h) = (le16(&f[6..]), le16(&f[8..]));
        assert_eq!(f[10], 0xF7);
        let mut pos = 13 + 768;
        assert_eq!(f[pos], 0x2C);
        assert_eq!((le16(&f[pos + 5..]), le16(&f[pos + 7..])), (w, h));
        assert_eq!(f[pos + 9], 0, "no local color table");
        assert_eq!(f[pos + 10], 8, "minimum code size");
        pos += 11;
        let mut data = Vec::new();
        loop {
            let n = usize::from(f[pos]);
            pos += 1;
            if n == 0 {
                break;
            }
            data.extend_from_slice(&f[pos..pos + n]);
            pos += n;
        }
        assert_eq!(f[pos], 0x3B);
        assert_eq!(pos + 1, f.len());
        // Read the 9 bit codes. A clear code starts every group, and every
        // other code is a literal, so the table never outgrows 9 bits.
        let (mut acc, mut nbits, mut pixels, mut in_group) = (0u32, 0, 0usize, 0);
        let mut ended = false;
        for &byte in &data {
            acc |= u32::from(byte) << nbits;
            nbits += 8;
            while nbits >= 9 {
                let code = acc & 0x1FF;
                acc >>= 9;
                nbits -= 9;
                assert!(!ended, "data after the end code");
                match code {
                    256 => in_group = 0,
                    257 => ended = true,
                    c => {
                        assert!(c < 256);
                        in_group += 1;
                        assert!(in_group <= 200);
                        pixels += 1;
                    }
                }
            }
        }
        assert!(ended);
        assert_eq!(pixels, w * h);
    }
}

#[test]
fn jpeg_files_walk_as_marker_segments() {
    for size in [100, 5000, 200_000] {
        let f = file("jpeg", size, 3);
        assert_eq!(&f[..4], &[0xFF, 0xD8, 0xFF, 0xE0]);
        assert_eq!(&f[6..11], b"JFIF\0");
        let mut pos = 2;
        while f[pos] == 0xFF && f[pos + 1] != 0xD9 {
            assert!(f[pos + 1] == 0xE0 || f[pos + 1] == 0xFE);
            pos += 2 + usize::from(u16::from_be_bytes([f[pos + 2], f[pos + 3]]));
        }
        assert_eq!(&f[pos..], &[0xFF, 0xD9]);
    }
}
