// SPDX-License-Identifier: GPL-3.0-only
//! Plain text email bodies: a greeting, prose, a reference line, a closing,
//! and a signature. The sender and the recipient are invented names, and the
//! signature address lies in the reserved `.test` domain.

use crate::prose::{person, Lang};
use crate::rng::Rng;

const PREFIXES: &[&str] = &["INV", "TKT", "ORD", "REQ", "CASE", "BKG"];

/// Returns a body of at least `size` bytes.
pub(crate) fn generate(r: &mut Rng, l: &Lang, size: usize) -> Vec<u8> {
    let (to, _) = person(r);
    let (first, last) = person(r);
    let head = format!("{} {to},\n\n", l.greeting);
    let tail = format!(
        "\nRef: {}-{:06}\n\n{},\n{first} {last}\nSupport desk\n{}.{}@example.test\n",
        r.pick(PREFIXES),
        r.below(1_000_000),
        l.closing,
        first.to_lowercase(),
        last.to_lowercase()
    );
    let body_size = size.saturating_sub(head.len() + tail.len());
    let mut body = l.prose(r, body_size);
    if body.is_empty() {
        // A body without prose would leave the greeting and the signature
        // joined by a single blank line, so one sentence stays.
        l.sentence(r, &mut body);
        body.push('\n');
    }
    format!("{head}{body}{tail}").into_bytes()
}
