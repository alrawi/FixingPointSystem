// SPDX-License-Identifier: GPL-3.0-only
//! JSON documents in the shape of a paged API response.
//!
//! The document holds a status, a request id, paging links, a `data` array
//! that grows by one record at a time until the document reaches the size,
//! and a `meta` object. Every string comes from the word lists, so none needs
//! escaping. Names come from invented syllables, and addresses end in the
//! reserved `.test` domain.

use crate::prose::{lang, person};
use crate::rng::Rng;
use crate::words::{ADJECTIVES, NOUNS, STATUSES, TAGS};

fn record(r: &mut Rng, s: &mut String) {
    let noun = r.pick(NOUNS);
    let (first, last) = person(r);
    let mut note = String::new();
    if let Some(en) = lang("en") {
        en.sentence(r, &mut note);
    }
    let tags: Vec<String> = (0..r.range(1, 3))
        .map(|_| format!("\"{}\"", r.pick(TAGS)))
        .collect();
    s.push_str(&format!(
        "{{\"id\":\"{}\",\"type\":\"{noun}\",\"name\":\"{} {noun}\",\"owner\":{{\"name\":\"{first} {last}\",\"email\":\"{}.{}@example.test\"}},\"status\":\"{}\",\"score\":{}.{:02},\"count\":{},\"active\":{},\"tags\":[{}],\"created\":\"{}-{:02}-{:02}T{:02}:{:02}:{:02}Z\",\"url\":\"https://api.example.test/v1/{noun}s/{}\",\"note\":\"{note}\"}}",
        r.hex(12),
        r.pick(ADJECTIVES),
        first.to_lowercase(),
        last.to_lowercase(),
        r.pick(STATUSES),
        r.below(1000),
        r.below(100),
        r.below(100_000),
        r.chance(1, 2),
        tags.join(","),
        r.range(2020, 2039),
        r.range(1, 12),
        r.range(1, 28),
        r.below(24),
        r.below(60),
        r.below(60),
        r.hex(8)
    ));
}

/// Returns a document of at least `size` bytes.
pub(crate) fn generate(r: &mut Rng, size: usize) -> Vec<u8> {
    let page = r.range(1, 500);
    let per_page = r.range(10, 100);
    let mut s = format!(
        "{{\"status\":\"ok\",\"request_id\":\"{}\",\"page\":{page},\"per_page\":{per_page},\"links\":{{\"self\":\"https://api.example.test/v1/{}s?page={page}\",\"next\":\"https://api.example.test/v1/{}s?page={}\"}},\"data\":[",
        r.hex(16),
        r.pick(NOUNS),
        r.pick(NOUNS),
        page + 1
    );
    let tail = format!(
        "],\"meta\":{{\"total\":{},\"elapsed_ms\":{},\"cache\":\"{}\"}}}}\n",
        r.below(1_000_000),
        r.below(900),
        if r.chance(1, 2) { "hit" } else { "miss" }
    );
    let mut first = true;
    while s.len() + tail.len() < size {
        if !first {
            s.push(',');
        }
        first = false;
        record(r, &mut s);
    }
    s.push_str(&tail);
    s.into_bytes()
}
