// SPDX-License-Identifier: GPL-3.0-only
//! Server replies of the POP3 dialogue.
//!
//! A flow cannot branch on what it reads, so the server side of a scenario
//! answers each command with one function call that reads the command line
//! from `last`. These functions parse the subset of RFC 1939 that the
//! scenarios use, the command name and its numeric arguments, and render the
//! status line and the multi line payload with the dot stuffing of section 3.
//! A command that the subset does not cover fails the calling session with the
//! reason `extension_protocol_violation`, because the bytes came from the
//! peer. A message number outside the mailbox gets the `-ERR` that a server
//! writes, because a real client may ask for one.

use fps_flow::{FnError, FnErrorKind};

use crate::catalog::{MAX_MESSAGES, UID_BASE};
use crate::md5;
use crate::mime;
use crate::rng::Rng;

fn invalid(message: String) -> FnError {
    FnError::new(FnErrorKind::InvalidArgument, message)
}

fn protocol(message: String) -> FnError {
    FnError::new(FnErrorKind::Protocol, message)
}

fn push(out: &mut Vec<u8>, text: &str) {
    out.extend_from_slice(text.as_bytes());
}

/// One parsed command line.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Command<'a> {
    /// The command name in upper case.
    pub name: String,
    /// The arguments after the command name.
    pub args: Vec<&'a str>,
}

/// Parses `line` as one command line of RFC 1939, a keyword of three or four
/// letters and up to two arguments, ended by CRLF.
pub(crate) fn parse_command<'a>(function: &str, line: &'a [u8]) -> Result<Command<'a>, FnError> {
    let text = std::str::from_utf8(line)
        .map_err(|_| protocol(format!("the command of '{function}' is not UTF-8 text")))?;
    let body = text
        .strip_suffix("\r\n")
        .ok_or_else(|| protocol(format!("the command of '{function}' does not end in CRLF")))?;
    if body.bytes().any(|b| b == b'\r' || b == b'\n' || b == 0) {
        return Err(protocol(format!(
            "the command of '{function}' is more than one line"
        )));
    }
    let mut words = body.split(' ');
    let name = words.next().unwrap_or("").to_ascii_uppercase();
    if !(3..=4).contains(&name.len()) || !name.bytes().all(|b| b.is_ascii_alphabetic()) {
        return Err(protocol(format!(
            "the command of '{function}' has no valid command name"
        )));
    }
    let args: Vec<&str> = words.collect();
    if args.len() > 2 || args.iter().any(|a| a.is_empty()) {
        return Err(protocol(format!(
            "the command of '{function}' has a bad argument list"
        )));
    }
    Ok(Command { name, args })
}

/// Parses the command line and checks that it is `name` with `min` to `max`
/// arguments.
fn expect<'a>(
    function: &str,
    line: &'a [u8],
    name: &str,
    min: usize,
    max: usize,
) -> Result<Command<'a>, FnError> {
    let c = parse_command(function, line)?;
    if c.name != name {
        return Err(protocol(format!(
            "'{function}' answers {name}, not {}",
            c.name
        )));
    }
    if !(min..=max).contains(&c.args.len()) {
        return Err(protocol(format!(
            "{name} in the command of '{function}' takes {min} to {max} arguments, not {}",
            c.args.len()
        )));
    }
    Ok(c)
}

/// Parses a number of a command. RFC 1939 writes it as digits without a sign.
fn number(function: &str, what: &str, arg: &str) -> Result<i64, FnError> {
    let bad = || {
        protocol(format!(
            "'{arg}' is not a {what} in the command of '{function}'"
        ))
    };
    if arg.is_empty() || arg.len() > 10 || !arg.bytes().all(|b| b.is_ascii_digit()) {
        return Err(bad());
    }
    arg.parse().map_err(|_| bad())
}

fn check_count(function: &str, count: i64) -> Result<(), FnError> {
    if (0..=MAX_MESSAGES).contains(&count) {
        Ok(())
    } else {
        Err(invalid(format!(
            "'count' of '{function}' must be between 0 and {MAX_MESSAGES}, not {count}"
        )))
    }
}

/// Returns `value` plus `step` as text. The counter never goes below zero.
pub(crate) fn next(value: &str, step: i64) -> Result<String, FnError> {
    let n: i64 = value
        .parse()
        .map_err(|_| invalid(format!("'{value}' is not a number for 'next' to count")))?;
    n.checked_add(step)
        .filter(|n| *n >= 0)
        .map(|n| n.to_string())
        .ok_or_else(|| invalid(format!("'step' of 'next' takes '{value}' below zero")))
}

/// Draws the timestamp banner `<process-ID.clock@host>` of RFC 1939. The
/// clock lies in 2026.
pub(crate) fn banner(host: &str, r: &mut Rng) -> String {
    let pid = r.range(1000, 99_999);
    let clock = 1_767_225_600 + r.below(31_536_000);
    format!("<{pid}.{clock}@{host}>")
}

/// Finds the timestamp banner in a greeting line, the text from the first
/// `<` to the next `>`, as RFC 1939 section 7 defines it.
fn find_banner(greeting: &str) -> Option<&str> {
    let start = greeting.find('<')?;
    let end = start + greeting[start..].find('>')? + 1;
    let banner = &greeting[start..end];
    let inner = &banner[1..banner.len() - 1];
    let ok = inner.contains('@')
        && inner
            .bytes()
            .all(|b| b.is_ascii_graphic() && b != b'<' && b != b'>');
    ok.then_some(banner)
}

/// Returns the digest of the `APOP` command, the MD5 of the banner of the
/// greeting and the secret in lower case hexadecimal.
pub(crate) fn apop_digest(greeting: &[u8], secret: &str) -> Result<String, FnError> {
    let text = std::str::from_utf8(greeting)
        .map_err(|_| protocol("the greeting for 'apop_digest' is not UTF-8 text".into()))?;
    let line = text.split(['\r', '\n']).next().unwrap_or("");
    if !line.starts_with("+OK") {
        return Err(protocol(
            "the greeting for 'apop_digest' is not a +OK line".into(),
        ));
    }
    let banner = find_banner(line).ok_or_else(|| {
        protocol("the greeting for 'apop_digest' has no timestamp banner <id@host>".into())
    })?;
    if secret.is_empty() || secret.bytes().any(|b| b == b'\r' || b == b'\n' || b == 0) {
        return Err(invalid(
            "'secret' of 'apop_digest' must be non empty text without a line break".into(),
        ));
    }
    let mut data = banner.as_bytes().to_vec();
    data.extend_from_slice(secret.as_bytes());
    Ok(md5::hex_digest(&data))
}

/// Appends `data` as the multi line payload of RFC 1939 section 3. Every line
/// break is CRLF, a line that starts with a dot gets a second dot, and a line
/// with the single dot ends the payload.
pub(crate) fn dot_stuff(data: &[u8], out: &mut Vec<u8>) {
    let normal = mime::crlf(data);
    let mut at_line_start = true;
    for &b in &normal {
        if at_line_start && b == b'.' {
            out.push(b'.');
        }
        out.push(b);
        at_line_start = b == b'\n';
    }
    if !normal.is_empty() && !normal.ends_with(b"\r\n") {
        out.extend_from_slice(b"\r\n");
    }
    out.extend_from_slice(b".\r\n");
}

/// The size of message `n` of the mailbox `mailbox` in octets. It depends only
/// on the two numbers, so `STAT`, `LIST`, and a later `LIST n` agree.
fn size_of(mailbox: i64, n: i64) -> i64 {
    let mut r = Rng::new((mailbox as u64) ^ (n as u64).wrapping_mul(0xA24B_AED4_963E_E407));
    r.range(1500, 200_000) as i64
}

/// The unique identifier of message `n` in the format of Dovecot, the message
/// UID and the UID validity as two groups of eight upper case hex digits.
fn uidl_of(mailbox: i64, n: i64) -> String {
    let mut r = Rng::new((mailbox as u64) ^ 0x7A1D);
    let validity = 0x5000_0000 + r.below(0x1000_0000) as i64;
    format!("{:08X}{validity:08X}", UID_BASE + n)
}

fn total_size(mailbox: i64, count: i64) -> i64 {
    (1..=count).map(|n| size_of(mailbox, n)).sum()
}

/// Appends the answer to `STAT`.
pub(crate) fn stat(count: i64, mailbox: i64, out: &mut Vec<u8>) -> Result<(), FnError> {
    check_count("stat_reply", count)?;
    push(
        out,
        &format!("+OK {count} {}\r\n", total_size(mailbox, count)),
    );
    Ok(())
}

fn no_message(n: i64, out: &mut Vec<u8>) {
    push(out, &format!("-ERR There's no message {n}.\r\n"));
}

/// Appends the answer to `LIST` or `LIST n`.
pub(crate) fn list(
    command: &[u8],
    count: i64,
    mailbox: i64,
    out: &mut Vec<u8>,
) -> Result<(), FnError> {
    check_count("list_reply", count)?;
    let c = expect("list_reply", command, "LIST", 0, 1)?;
    if let Some(arg) = c.args.first() {
        let n = number("list_reply", "message number", arg)?;
        if n < 1 || n > count {
            no_message(n, out);
        } else {
            push(out, &format!("+OK {n} {}\r\n", size_of(mailbox, n)));
        }
        return Ok(());
    }
    push(out, &format!("+OK {count} messages:\r\n"));
    for n in 1..=count {
        push(out, &format!("{n} {}\r\n", size_of(mailbox, n)));
    }
    push(out, ".\r\n");
    Ok(())
}

/// Appends the answer to `UIDL` or `UIDL n`.
pub(crate) fn uidl(
    command: &[u8],
    count: i64,
    mailbox: i64,
    out: &mut Vec<u8>,
) -> Result<(), FnError> {
    check_count("uidl_reply", count)?;
    let c = expect("uidl_reply", command, "UIDL", 0, 1)?;
    if let Some(arg) = c.args.first() {
        let n = number("uidl_reply", "message number", arg)?;
        if n < 1 || n > count {
            no_message(n, out);
        } else {
            push(out, &format!("+OK {n} {}\r\n", uidl_of(mailbox, n)));
        }
        return Ok(());
    }
    push(out, "+OK\r\n");
    for n in 1..=count {
        push(out, &format!("{n} {}\r\n", uidl_of(mailbox, n)));
    }
    push(out, ".\r\n");
    Ok(())
}

/// Appends the answer to `RETR n`, the status line with the size of the
/// message and then the dot stuffed message.
pub(crate) fn retr(
    command: &[u8],
    message: &[u8],
    count: i64,
    out: &mut Vec<u8>,
) -> Result<(), FnError> {
    check_count("retr_reply", count)?;
    let c = expect("retr_reply", command, "RETR", 1, 1)?;
    let n = number("retr_reply", "message number", c.args[0])?;
    if n < 1 || n > count {
        no_message(n, out);
        return Ok(());
    }
    let normal = mime::crlf(message);
    push(out, &format!("+OK {} octets\r\n", normal.len()));
    dot_stuff(&normal, out);
    Ok(())
}

/// Appends the answer to `TOP n lines`, the status line, the headers, the
/// blank line, and the first `lines` lines of the body, dot stuffed.
pub(crate) fn top(
    command: &[u8],
    message: &[u8],
    count: i64,
    out: &mut Vec<u8>,
) -> Result<(), FnError> {
    check_count("top_reply", count)?;
    let c = expect("top_reply", command, "TOP", 2, 2)?;
    let n = number("top_reply", "message number", c.args[0])?;
    let lines = number("top_reply", "line count", c.args[1])?;
    if n < 1 || n > count {
        no_message(n, out);
        return Ok(());
    }
    let normal = mime::crlf(message);
    let split = normal
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .map(|p| p + 4);
    let (head, body) = match split {
        Some(p) => normal.split_at(p),
        None => (&normal[..], &b""[..]),
    };
    let mut shown = head.to_vec();
    for line in body.split_inclusive(|&b| b == b'\n').take(lines as usize) {
        shown.extend_from_slice(line);
    }
    push(out, "+OK\r\n");
    dot_stuff(&shown, out);
    Ok(())
}

/// Appends the answer to `DELE n`.
pub(crate) fn dele(command: &[u8], count: i64, out: &mut Vec<u8>) -> Result<(), FnError> {
    check_count("dele_reply", count)?;
    let c = expect("dele_reply", command, "DELE", 1, 1)?;
    let n = number("dele_reply", "message number", c.args[0])?;
    if n < 1 || n > count {
        no_message(n, out);
    } else {
        push(out, "+OK Marked to be deleted.\r\n");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(f: impl FnOnce(&mut Vec<u8>) -> Result<(), FnError>) -> String {
        let mut out = Vec::new();
        f(&mut out).unwrap();
        String::from_utf8(out).unwrap()
    }

    const MSG: &[u8] =
        b"From: a.b@example.test\r\nSubject: Hi\r\n\r\nfirst\r\n.dot\r\n.\r\nlast\r\nend\r\n";

    #[test]
    fn commands_parse_in_any_case() {
        let c = parse_command("t", b"top 3 10\r\n").unwrap();
        assert_eq!((c.name.as_str(), c.args), ("TOP", vec!["3", "10"]));
        for bad in [
            &b"RETR 1"[..],
            b"RETR 1\r\nDELE 1\r\n",
            b"\r\n",
            b"RETR  1\r\n",
            b"RETR 1 2 3\r\n",
            b"R3TR 1\r\n",
            b"\xff\r\n",
        ] {
            assert_eq!(
                parse_command("t", bad).unwrap_err().kind,
                FnErrorKind::Protocol,
                "{bad:?}"
            );
        }
    }

    #[test]
    fn the_counter_counts_numbers_only() {
        assert_eq!(next("0", 1).unwrap(), "1");
        assert_eq!(next("9", 1).unwrap(), "10");
        assert_eq!(next("5", -5).unwrap(), "0");
        assert!(next("5", -6).is_err());
        assert!(next("A001", 1).is_err());
    }

    #[test]
    fn a_banner_has_the_form_of_rfc_1939() {
        let b = banner("pop3.example.test", &mut Rng::new(1));
        assert!(
            b.starts_with('<') && b.ends_with("@pop3.example.test>"),
            "{b}"
        );
        let (id, _) = b[1..].split_once('@').unwrap();
        let (pid, clock) = id.split_once('.').unwrap();
        assert!(pid.parse::<u32>().is_ok() && clock.parse::<u64>().unwrap() > 1_767_225_600);
        assert_ne!(b, banner("pop3.example.test", &mut Rng::new(2)));
    }

    #[test]
    fn the_apop_digest_is_the_md5_of_banner_and_secret() {
        // The example of RFC 1939 section 7.
        let g = b"+OK POP3 server ready <1896.697170952@dbc.mtview.ca.us>\r\n";
        assert_eq!(
            apop_digest(g, "tanstaaf").unwrap(),
            "c4c9334bac560ecc979e58001b3e22fb"
        );
        for (greeting, secret) in [
            (&b"+OK ready\r\n"[..], "x"),
            (b"+OK <no-at-sign>\r\n", "x"),
            (b"-ERR <1.2@h>\r\n", "x"),
            (b"+OK <1.2@h>\r\n", ""),
            (b"+OK <1.2@h>\r\n", "a\r\nb"),
        ] {
            assert!(apop_digest(greeting, secret).is_err(), "{greeting:?}");
        }
    }

    #[test]
    fn dot_stuffing_doubles_leading_dots_and_ends_with_the_dot_line() {
        let mut out = Vec::new();
        dot_stuff(b"a\n.b\n.\n..c", &mut out);
        assert_eq!(out, b"a\r\n..b\r\n..\r\n...c\r\n.\r\n");
        let mut empty = Vec::new();
        dot_stuff(b"", &mut empty);
        assert_eq!(empty, b".\r\n");
    }

    #[test]
    fn stat_and_list_agree_on_the_sizes() {
        let l = run(|o| list(b"LIST\r\n", 3, 99, o));
        let lines: Vec<&str> = l.split("\r\n").collect();
        assert_eq!(lines[0], "+OK 3 messages:");
        assert_eq!(lines.len(), 3 + 1 + 1 + 1, "{lines:?}");
        assert_eq!(lines[4], ".");
        let sizes: Vec<i64> = (1..=3)
            .map(|i| lines[i].split(' ').nth(1).unwrap().parse().unwrap())
            .collect();
        let s = run(|o| stat(3, 99, o));
        assert_eq!(s, format!("+OK 3 {}\r\n", sizes.iter().sum::<i64>()));
        let one = run(|o| list(b"LIST 2\r\n", 3, 99, o));
        assert_eq!(one, format!("+OK 2 {}\r\n", sizes[1]));
        assert_eq!(
            run(|o| list(b"LIST 4\r\n", 3, 99, o)),
            "-ERR There's no message 4.\r\n"
        );
        assert_eq!(
            run(|o| list(b"LIST\r\n", 0, 1, o)),
            "+OK 0 messages:\r\n.\r\n"
        );
    }

    #[test]
    fn uidl_lists_stable_identifiers_in_the_dovecot_format() {
        let u = run(|o| uidl(b"UIDL\r\n", 2, 7, o));
        let lines: Vec<&str> = u.split("\r\n").collect();
        assert_eq!(lines[0], "+OK");
        assert!(lines[1].starts_with("1 000003E9"), "{u}");
        assert!(lines[2].starts_with("2 000003EA"), "{u}");
        assert_eq!(lines[1].len(), 2 + 16);
        assert_eq!(&lines[1][2 + 8..], &lines[2][2 + 8..], "one validity");
        let one = run(|o| uidl(b"UIDL 2\r\n", 2, 7, o));
        assert_eq!(one, format!("+OK {}\r\n", lines[2]));
        assert_ne!(u, run(|o| uidl(b"UIDL\r\n", 2, 8, o)));
    }

    #[test]
    fn retr_stuffs_the_message_and_reports_its_size() {
        let r = run(|o| retr(b"RETR 1\r\n", MSG, 2, o));
        assert!(
            r.starts_with(&format!("+OK {} octets\r\n", MSG.len())),
            "{r}"
        );
        assert!(r.contains("\r\n..dot\r\n..\r\nlast\r\n"), "{r}");
        assert!(r.ends_with("end\r\n.\r\n"));
        // Only the closing line is a lone dot.
        assert_eq!(r.matches("\r\n.\r\n").count(), 1);
        assert_eq!(
            run(|o| retr(b"RETR 3\r\n", MSG, 2, o)),
            "-ERR There's no message 3.\r\n"
        );
        assert_eq!(
            retr(b"TOP 1 1\r\n", MSG, 2, &mut Vec::new())
                .unwrap_err()
                .kind,
            FnErrorKind::Protocol
        );
    }

    #[test]
    fn top_returns_the_headers_and_the_requested_lines() {
        let t = run(|o| top(b"TOP 1 2\r\n", MSG, 2, o));
        assert_eq!(
            t,
            "+OK\r\nFrom: a.b@example.test\r\nSubject: Hi\r\n\r\nfirst\r\n..dot\r\n.\r\n"
        );
        let none = run(|o| top(b"TOP 1 0\r\n", MSG, 2, o));
        assert_eq!(
            none,
            "+OK\r\nFrom: a.b@example.test\r\nSubject: Hi\r\n\r\n.\r\n"
        );
        let all = run(|o| top(b"TOP 2 100\r\n", MSG, 2, o));
        assert!(all.ends_with("end\r\n.\r\n"));
        assert!(top(b"TOP 1 -1\r\n", MSG, 2, &mut Vec::new()).is_err());
        assert!(top(b"TOP 1\r\n", MSG, 2, &mut Vec::new()).is_err());
    }

    #[test]
    fn dele_accepts_messages_in_the_mailbox_only() {
        assert_eq!(
            run(|o| dele(b"DELE 2\r\n", 2, o)),
            "+OK Marked to be deleted.\r\n"
        );
        assert_eq!(
            run(|o| dele(b"DELE 0\r\n", 2, o)),
            "-ERR There's no message 0.\r\n"
        );
        assert!(dele(b"DELE x\r\n", 2, &mut Vec::new()).is_err());
        assert!(dele(b"DELE 1\r\n", -1, &mut Vec::new()).is_err());
    }
}
