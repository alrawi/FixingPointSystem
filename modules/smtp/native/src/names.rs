// SPDX-License-Identifier: GPL-3.0-only
//! Synthetic mailbox addresses, user names, and passwords.
//!
//! Every name is built from invented syllables, so no output names a real
//! person, and every address lies under the reserved `.test` domain. Every
//! password starts with `Tst-`, which marks it as a fixture value, and none of
//! them is a valid secret anywhere.

use crate::rng::Rng;

const ONSETS: &[&str] = &[
    "b", "d", "f", "g", "h", "k", "l", "m", "n", "p", "r", "s", "t", "v", "z", "br", "dr", "kl",
    "st", "tr",
];
const VOWELS: &[&str] = &["a", "e", "i", "o", "u", "ai", "ei", "ou"];
const CODAS: &[&str] = &["", "", "n", "r", "l", "s", "m", "k"];

/// Builds one invented word of two or three syllables.
pub(crate) fn word(r: &mut Rng) -> String {
    let syllables = r.range(2, 3);
    let mut w = String::new();
    for i in 0..syllables {
        w.push_str(r.pick(ONSETS));
        w.push_str(r.pick(VOWELS));
        if i + 1 == syllables {
            w.push_str(r.pick(CODAS));
        }
    }
    w
}

/// Returns the address `first.last@domain` that the pair `seed` and `index`
/// names. The same pair always gives the same address.
pub(crate) fn address(seed: i64, index: i64, domain: &str) -> String {
    let mixed = (seed as u64) ^ (index as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    let mut r = Rng::new(mixed);
    let first = word(&mut r);
    let last = word(&mut r);
    format!("{first}.{last}@{domain}")
}

/// Returns a password such as `Tst-Kamurai4821`.
pub(crate) fn password(r: &mut Rng) -> String {
    let mut w = word(r);
    if let Some(first) = w.get_mut(..1) {
        first.make_ascii_uppercase();
    }
    format!("Tst-{w}{:04}", r.below(10_000))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_address_depends_only_on_its_arguments() {
        assert_eq!(address(7, 1, "example.test"), address(7, 1, "example.test"));
        assert_ne!(address(7, 1, "example.test"), address(7, 2, "example.test"));
        assert_ne!(address(7, 1, "example.test"), address(8, 1, "example.test"));
    }

    #[test]
    fn addresses_are_plain_ascii_under_the_given_domain() {
        for seed in 0..200 {
            let a = address(seed, 1, "example.test");
            let (local, domain) = a.split_once('@').unwrap();
            assert_eq!(domain, "example.test");
            assert!(local.bytes().all(|b| b.is_ascii_lowercase() || b == b'.'));
            assert_eq!(local.matches('.').count(), 1, "{a}");
        }
    }

    #[test]
    fn passwords_are_marked_and_never_parse_as_numbers() {
        let mut r = Rng::new(3);
        for _ in 0..100 {
            let p = password(&mut r);
            assert!(p.starts_with("Tst-"), "{p}");
            assert!(p.parse::<f64>().is_err());
            assert!(p.is_ascii() && !p.contains(' '));
        }
    }
}
