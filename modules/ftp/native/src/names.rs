// SPDX-License-Identifier: GPL-3.0-only
//! Synthetic user names, passwords, file names, and directory listings.
//!
//! Every name is built from invented syllables, so no output names a real
//! person or a real file. Every password starts with `Tst-`, which marks it as
//! a fixture value, and none of them is a valid secret anywhere.

use crate::rng::Rng;

const ONSETS: &[&str] = &[
    "b", "d", "f", "g", "h", "k", "l", "m", "n", "p", "r", "s", "t", "v", "z", "br", "dr", "kl",
    "st", "tr",
];
const VOWELS: &[&str] = &["a", "e", "i", "o", "u", "ai", "ei", "ou"];
const CODAS: &[&str] = &["", "", "n", "r", "l", "s", "m", "k"];

/// The extensions of the files in a listing.
const EXTENSIONS: &[&str] = &[
    "pdf", "zip", "png", "gz", "txt", "csv", "log", "jpg", "bin", "tar.gz",
];

const MONTHS: &[&str] = &[
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

const PERMISSIONS: &[&str] = &["-rw-r--r--", "-rw-r--r--", "-rw-rw-r--", "-rwxr-xr-x"];

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

/// Returns a user name such as `kamur.teilos`.
pub(crate) fn user(r: &mut Rng) -> String {
    let first = word(r);
    let last = word(r);
    format!("{first}.{last}")
}

/// Returns a password such as `Tst-Kamurai4821`.
pub(crate) fn password(r: &mut Rng) -> String {
    let mut w = word(r);
    if let Some(first) = w.get_mut(..1) {
        first.make_ascii_uppercase();
    }
    format!("Tst-{w}{:04}", r.below(10_000))
}

/// Returns a file name such as `bratos_zeilan-202603.pdf`.
pub(crate) fn file_name(r: &mut Rng, ext: &str) -> String {
    let a = word(r);
    let b = word(r);
    let month = r.range(1, 11);
    format!("{a}_{b}-2026{month:02}.{ext}")
}

/// Renders `count` lines of an `ls -l` listing as an FTP server sends them
/// for `LIST`: a mode, a link count, an owner and a group, a size, a date, and
/// a name, each line ending in CRLF. About one entry in six is a directory.
pub(crate) fn listing(r: &mut Rng, count: usize, out: &mut Vec<u8>) {
    for _ in 0..count {
        let dir = r.below(6) == 0;
        let (mode, links, size, name) = if dir {
            ("drwxr-xr-x", r.range(2, 5), 4096, word(r))
        } else {
            let ext = *r.pick(EXTENSIONS);
            let size = r.range(120, 90_000_000);
            (*r.pick(PERMISSIONS), 1, size, file_name(r, ext))
        };
        let month = *r.pick(MONTHS);
        let day = r.range(1, 28);
        // A recent entry shows the time of day and an old one shows the year,
        // as ls does.
        let when = if r.below(4) == 0 {
            format!("{day:>2}  2025")
        } else {
            format!("{day:>2} {:02}:{:02}", r.range(0, 23), r.range(0, 59))
        };
        let line =
            format!("{mode} {links:>4} ftp      ftp      {size:>9} {month} {when} {name}\r\n");
        out.extend_from_slice(line.as_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn users_and_file_names_are_plain_ascii() {
        let mut r = Rng::new(9);
        for _ in 0..100 {
            let u = user(&mut r);
            assert!(
                u.bytes().all(|b| b.is_ascii_lowercase() || b == b'.'),
                "{u}"
            );
            let f = file_name(&mut r, "pdf");
            assert!(
                f.ends_with(".pdf") && f.is_ascii() && !f.contains(' '),
                "{f}"
            );
        }
    }

    #[test]
    fn listing_lines_have_the_ls_layout() {
        let mut r = Rng::new(5);
        let mut out = Vec::new();
        listing(&mut r, 60, &mut out);
        let text = String::from_utf8(out).unwrap();
        let lines: Vec<&str> = text.split("\r\n").collect();
        assert_eq!(lines.len(), 61);
        assert_eq!(lines[60], "");
        for l in &lines[..60] {
            let f: Vec<&str> = l.split_whitespace().collect();
            assert!(f.len() >= 9, "{l}");
            assert_eq!(f[0].len(), 10, "{l}");
            assert!(f[0].starts_with('-') || f[0].starts_with('d'), "{l}");
            assert!(f[4].parse::<u64>().is_ok(), "{l}");
        }
        assert!(lines.iter().any(|l| l.starts_with('d')));
        assert!(lines.iter().any(|l| l.starts_with('-')));
    }
}
