// SPDX-License-Identifier: GPL-3.0-only
//! Synthetic user names, passwords, file names, shell commands, and the text
//! that a shell prints.
//!
//! Every name is built from invented syllables, so no output names a real
//! person or a real file. Every password starts with `Tst-`, which marks it
//! as a fixture value. The commands and the output lines are short phrases
//! that we wrote for this module, and no line comes from a real system.

use crate::rng::Rng;

const ONSETS: &[&str] = &[
    "b", "d", "f", "g", "h", "k", "l", "m", "n", "p", "r", "s", "t", "v", "z", "br", "dr", "kl",
    "st", "tr",
];
const VOWELS: &[&str] = &["a", "e", "i", "o", "u", "ai", "ei", "ou"];
const CODAS: &[&str] = &["", "", "n", "r", "l", "s", "m", "k"];

const MONTHS: &[&str] = &[
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// What an operator types on a server. Each entry is one line.
const COMMANDS: &[&str] = &[
    "ls -la /var/log",
    "df -h",
    "uptime",
    "free -m",
    "cat /etc/os-release",
    "ps aux --sort=-%mem | head -n 15",
    "tail -n 50 /var/log/syslog",
    "systemctl status cron --no-pager",
    "du -sh /srv/* | sort -h",
    "ss -tln",
    "journalctl -u ssh -n 30 --no-pager",
    "ip -brief address",
];

/// Words of the invented log lines.
const SERVICES: &[&str] = &["cron", "backupd", "metricsd", "syncer", "rotate", "indexer"];
const MESSAGES: &[&str] = &[
    "job finished without errors",
    "queue length 0",
    "connection pool refreshed",
    "snapshot written",
    "retention check passed",
    "heartbeat accepted",
    "configuration reloaded",
    "cache warmed",
];

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

/// Returns a file name such as `bratos_zeilan-202603.tar.gz`.
pub(crate) fn file_name(r: &mut Rng, ext: &str) -> String {
    let a = word(r);
    let b = word(r);
    let month = r.range(1, 11);
    format!("{a}_{b}-2026{month:02}.{ext}")
}

/// Returns a command line that an operator might type.
pub(crate) fn command(r: &mut Rng) -> &'static str {
    r.pick(COMMANDS)
}

/// One line of invented output, without a line break.
fn text_line(r: &mut Rng) -> String {
    match r.below(3) {
        0 => {
            let size = r.range(120, 9_000_000);
            let day = r.range(1, 28);
            format!(
                "-rw-r--r-- 1 svc svc {size:>8} {} {day:>2} {:02}:{:02} {}",
                r.pick(MONTHS),
                r.range(0, 23),
                r.range(0, 59),
                file_name(r, "log")
            )
        }
        1 => format!(
            "{} {:02}:{:02}:{:02} host {}[{}]: {}",
            r.pick(MONTHS),
            r.range(0, 23),
            r.range(0, 59),
            r.range(0, 59),
            r.pick(SERVICES),
            r.range(300, 40_000),
            r.pick(MESSAGES)
        ),
        _ => format!(
            "{:<12} {:>5} {:>4}.{} {:>5}.{}",
            word(r),
            r.range(1, 32_000),
            r.range(0, 99),
            r.below(10),
            r.range(0, 99),
            r.below(10)
        ),
    }
}

/// Fills exactly `size` bytes of printable text: lines that end in CR LF, as
/// a pseudo terminal delivers them, cut at `size`.
pub(crate) fn text(r: &mut Rng, size: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(size + 128);
    while out.len() < size {
        out.extend_from_slice(text_line(r).as_bytes());
        out.extend_from_slice(b"\r\n");
    }
    out.truncate(size);
    out
}

/// Fills exactly `size` bytes of lower case letters and spaces, which is what
/// a person types.
pub(crate) fn keys(r: &mut Rng, size: usize) -> Vec<u8> {
    (0..size)
        .map(|_| {
            if r.below(8) == 0 {
                b' '
            } else {
                b'a' + u8::try_from(r.below(26)).unwrap_or(0)
            }
        })
        .collect()
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
            let f = file_name(&mut r, "gz");
            assert!(
                f.ends_with(".gz") && f.is_ascii() && !f.contains(' '),
                "{f}"
            );
        }
    }

    #[test]
    fn text_and_keys_have_exactly_the_requested_size() {
        let mut r = Rng::new(1);
        for size in [0, 1, 2, 77, 1000, 30_000] {
            let t = text(&mut r, size);
            assert_eq!(t.len(), size);
            assert!(t
                .iter()
                .all(|b| b.is_ascii_graphic() || *b == b' ' || *b == b'\r' || *b == b'\n'));
            let k = keys(&mut r, size);
            assert_eq!(k.len(), size);
            assert!(k.iter().all(|b| b.is_ascii_lowercase() || *b == b' '));
        }
    }

    #[test]
    fn commands_are_single_lines() {
        let mut r = Rng::new(2);
        for _ in 0..50 {
            let c = command(&mut r);
            assert!(c.is_ascii() && !c.contains('\n'));
        }
    }
}
