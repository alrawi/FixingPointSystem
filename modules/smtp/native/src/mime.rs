// SPDX-License-Identifier: GPL-3.0-only
//! Message rendering: base64, line breaks, MIME structure, and the transparent
//! form of the DATA command.
//!
//! The renderers follow RFC 5322 for the message, RFC 2045 and RFC 2046 for
//! MIME, RFC 2047 for non ASCII subjects, and RFC 5321 section 4.5.2 for dot
//! stuffing. Every text argument is checked before the first byte is written,
//! so a value with a line break fails the call with `invalid_argument` and
//! never turns into an extra header or command.

use fps_flow::{FnError, FnErrorKind};
use rand::RngCore;

use crate::catalog::MAX_LINE;
use crate::rng::Rng;

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn invalid(message: String) -> FnError {
    FnError::new(FnErrorKind::InvalidArgument, message)
}

/// Appends the base64 form of `data`. A `wrap` above zero ends each output line
/// after that many characters, with CRLF between the lines and none after the
/// last one.
pub(crate) fn base64(data: &[u8], wrap: usize, out: &mut Vec<u8>) {
    let mut col = 0;
    let mut put = |c: u8, out: &mut Vec<u8>| {
        if wrap > 0 && col == wrap {
            out.extend_from_slice(b"\r\n");
            col = 0;
        }
        out.push(c);
        col += 1;
    };
    for chunk in data.chunks(3) {
        let b = [
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        let idx = |shift: u32| B64[((n >> shift) & 63) as usize];
        put(idx(18), out);
        put(idx(12), out);
        put(if chunk.len() > 1 { idx(6) } else { b'=' }, out);
        put(if chunk.len() > 2 { idx(0) } else { b'=' }, out);
    }
}

/// Rewrites every line break, which may be LF, CR, or CRLF, as CRLF.
pub(crate) fn crlf(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() + data.len() / 32);
    let mut i = 0;
    while i < data.len() {
        match data[i] {
            b'\r' => {
                out.extend_from_slice(b"\r\n");
                if data.get(i + 1) == Some(&b'\n') {
                    i += 1;
                }
            }
            b'\n' => out.extend_from_slice(b"\r\n"),
            b => out.push(b),
        }
        i += 1;
    }
    out
}

/// Renders the `DATA` payload of a message: CRLF line breaks, a second dot in
/// front of every line that starts with a dot, a final line break, and the
/// line `.` that ends the data.
pub(crate) fn data(message: &[u8], out: &mut Vec<u8>) -> Result<(), FnError> {
    if message.is_empty() {
        return Err(invalid("message of 'data' must not be empty".into()));
    }
    if message.contains(&0) {
        return Err(invalid(
            "message of 'data' must not contain a NUL byte".into(),
        ));
    }
    let body = crlf(message);
    let mut at_line_start = true;
    for &b in &body {
        if at_line_start && b == b'.' {
            out.push(b'.');
        }
        out.push(b);
        at_line_start = b == b'\n';
    }
    if !body.ends_with(b"\r\n") {
        out.extend_from_slice(b"\r\n");
    }
    out.extend_from_slice(b".\r\n");
    Ok(())
}

/// Wraps ASCII text so that no line exceeds [`MAX_LINE`] bytes. A line breaks
/// at its last space within the limit, and a line without one breaks at the
/// limit. The input has CRLF line breaks, and so has the output.
fn wrap_ascii(text: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(text.len() + 16);
    let mut lines = text.split(|&b| b == b'\n').peekable();
    while let Some(raw) = lines.next() {
        let last = lines.peek().is_none();
        let mut line = raw.strip_suffix(b"\r").unwrap_or(raw);
        while line.len() > MAX_LINE {
            let cut = line[..=MAX_LINE]
                .iter()
                .rposition(|&b| b == b' ')
                .filter(|&p| p > 0)
                .unwrap_or(MAX_LINE);
            out.extend_from_slice(&line[..cut]);
            out.extend_from_slice(b"\r\n");
            line = &line[cut..];
            if line.first() == Some(&b' ') {
                line = &line[1..];
            }
        }
        out.extend_from_slice(line);
        if !last {
            out.extend_from_slice(b"\r\n");
        }
    }
    out
}

fn check_header(name: &str, value: &str) -> Result<(), FnError> {
    if value.bytes().any(|b| b == b'\r' || b == b'\n' || b == 0) {
        return Err(invalid(format!(
            "'{name}' of 'message' must not contain a line break or a NUL byte"
        )));
    }
    Ok(())
}

fn check_ascii(name: &str, value: &str) -> Result<(), FnError> {
    check_header(name, value)?;
    if !value.is_ascii() {
        return Err(invalid(format!("'{name}' of 'message' must be ASCII")));
    }
    Ok(())
}

/// Returns the header line for an address list, folded at the commas.
fn address_header(name: &str, list: &str, out: &mut Vec<u8>) {
    out.extend_from_slice(name.as_bytes());
    out.extend_from_slice(b": ");
    let mut col = name.len() + 2;
    for (i, part) in list.split(',').map(str::trim).enumerate() {
        if i > 0 {
            if col + part.len() + 2 > MAX_LINE {
                out.extend_from_slice(b",\r\n\t");
                col = 8;
            } else {
                out.extend_from_slice(b", ");
                col += 2;
            }
        }
        out.extend_from_slice(part.as_bytes());
        col += part.len();
    }
    out.extend_from_slice(b"\r\n");
}

/// Renders a subject, as RFC 2047 encoded words when it is not ASCII.
fn subject_header(subject: &str, out: &mut Vec<u8>) {
    out.extend_from_slice(b"Subject: ");
    if subject.is_ascii() {
        out.extend_from_slice(subject.as_bytes());
    } else {
        let mut words: Vec<&str> = Vec::new();
        let mut start = 0;
        for (i, c) in subject.char_indices() {
            if i + c.len_utf8() - start > 36 {
                words.push(&subject[start..i]);
                start = i;
            }
        }
        words.push(&subject[start..]);
        for (n, w) in words.iter().enumerate() {
            if n > 0 {
                out.extend_from_slice(b"\r\n ");
            }
            out.extend_from_slice(b"=?UTF-8?B?");
            base64(w.as_bytes(), 0, out);
            out.extend_from_slice(b"?=");
        }
    }
    out.extend_from_slice(b"\r\n");
}

const MONTHS: [(&str, u32); 12] = [
    ("Jan", 31),
    ("Feb", 28),
    ("Mar", 31),
    ("Apr", 30),
    ("May", 31),
    ("Jun", 30),
    ("Jul", 31),
    ("Aug", 31),
    ("Sep", 30),
    ("Oct", 31),
    ("Nov", 30),
    ("Dec", 31),
];
const WEEKDAYS: [&str; 7] = ["Thu", "Fri", "Sat", "Sun", "Mon", "Tue", "Wed"];

/// Returns a date in 2026, which began on a Thursday, in the form of RFC 5322.
fn date(r: &mut Rng) -> String {
    let day_of_year = r.below(365);
    let mut rest = day_of_year as u32;
    let mut month = MONTHS[11];
    for m in &MONTHS {
        if rest < m.1 {
            month = *m;
            break;
        }
        rest -= m.1;
    }
    format!(
        "{}, {:02} {} 2026 {:02}:{:02}:{:02} +0000",
        WEEKDAYS[day_of_year % 7],
        rest + 1,
        month.0,
        r.below(24),
        r.below(60),
        r.below(60)
    )
}

/// The parts of a message.
pub(crate) struct Message<'a> {
    pub sender: &'a str,
    pub to: &'a str,
    pub cc: &'a str,
    pub subject: &'a str,
    pub body: &'a [u8],
    pub attachment: &'a [u8],
    pub filename: &'a str,
    pub content_type: &'a str,
    pub mailer: &'a str,
}

fn token_ok(s: &str) -> bool {
    !s.is_empty()
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"!#$&^_.+-".contains(&b))
}

impl Message<'_> {
    fn check(&self) -> Result<(), FnError> {
        if self.sender.trim().is_empty() || self.to.trim().is_empty() {
            return Err(invalid(
                "'sender' and 'to' of 'message' must not be empty".into(),
            ));
        }
        check_ascii("sender", self.sender)?;
        check_ascii("to", self.to)?;
        check_ascii("cc", self.cc)?;
        check_header("subject", self.subject)?;
        check_ascii("mailer", self.mailer)?;
        check_ascii("filename", self.filename)?;
        if self.filename.is_empty() || self.filename.bytes().any(|b| b == b'"' || b == b'\\') {
            return Err(invalid(
                "'filename' of 'message' must be a plain name without quotes or backslashes".into(),
            ));
        }
        let ok = self
            .content_type
            .split_once('/')
            .is_some_and(|(t, s)| token_ok(t) && token_ok(s));
        if !ok {
            return Err(invalid(format!(
                "'content_type' of 'message' must look like type/subtype, not '{}'",
                self.content_type
            )));
        }
        if self.body.contains(&0) {
            return Err(invalid(
                "'body' of 'message' must not contain a NUL byte".into(),
            ));
        }
        Ok(())
    }

    /// Appends the message to `out` and draws the boundary, the date, and the
    /// message id from `rng`.
    pub(crate) fn render(&self, rng: &mut dyn RngCore, out: &mut Vec<u8>) -> Result<(), FnError> {
        self.check()?;
        let text = std::str::from_utf8(self.body)
            .map_err(|_| invalid("'body' of 'message' must be UTF-8 text".into()))?;
        let mut r = Rng::new(rng.next_u64());
        let boundary = format!("=_fps_{}", r.hex(20));
        let date = date(&mut r);
        let host = self
            .sender
            .trim_end_matches('>')
            .rsplit('@')
            .next()
            .unwrap_or("example.test");
        let id = r.hex(24);

        out.extend_from_slice(format!("From: {}\r\n", self.sender).as_bytes());
        address_header("To", self.to, out);
        if !self.cc.trim().is_empty() {
            address_header("Cc", self.cc, out);
        }
        subject_header(self.subject, out);
        out.extend_from_slice(format!("Date: {date}\r\n").as_bytes());
        out.extend_from_slice(format!("Message-ID: <{id}@{host}>\r\n").as_bytes());
        out.extend_from_slice(b"MIME-Version: 1.0\r\n");
        out.extend_from_slice(format!("X-Mailer: {}\r\n", self.mailer).as_bytes());

        let normal = crlf(text.as_bytes());
        let (encoding, encoded) = if text.is_ascii() {
            ("7bit", wrap_ascii(&normal))
        } else {
            let mut b = Vec::new();
            base64(&normal, MAX_LINE, &mut b);
            ("base64", b)
        };
        let text_headers = format!(
            "Content-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: {encoding}\r\n\r\n"
        );
        let end_line = |out: &mut Vec<u8>, body: &[u8]| {
            out.extend_from_slice(body);
            if !body.ends_with(b"\r\n") {
                out.extend_from_slice(b"\r\n");
            }
        };

        if self.attachment.is_empty() {
            out.extend_from_slice(text_headers.as_bytes());
            end_line(out, &encoded);
            return Ok(());
        }
        out.extend_from_slice(
            format!("Content-Type: multipart/mixed; boundary=\"{boundary}\"\r\n\r\n").as_bytes(),
        );
        out.extend_from_slice(b"This is a multi-part message in MIME format.\r\n");
        out.extend_from_slice(format!("--{boundary}\r\n{text_headers}").as_bytes());
        end_line(out, &encoded);
        out.extend_from_slice(
            format!(
                "--{boundary}\r\nContent-Type: {ct}; name=\"{name}\"\r\nContent-Disposition: attachment; filename=\"{name}\"\r\nContent-Transfer-Encoding: base64\r\n\r\n",
                ct = self.content_type,
                name = self.filename
            )
            .as_bytes(),
        );
        base64(self.attachment, MAX_LINE, out);
        out.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;

    fn b64(data: &[u8], wrap: usize) -> String {
        let mut out = Vec::new();
        base64(data, wrap, &mut out);
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn base64_matches_rfc_4648_vectors() {
        for (input, want) in [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
            ("fooba", "Zm9vYmE="),
            ("foobar", "Zm9vYmFy"),
        ] {
            assert_eq!(b64(input.as_bytes(), 0), want);
        }
    }

    #[test]
    fn base64_wraps_without_a_trailing_break() {
        let s = b64(&[7u8; 100], 76);
        let lines: Vec<&str> = s.split("\r\n").collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].len(), 76);
        assert!(!s.ends_with('\n'));
    }

    #[test]
    fn crlf_normalises_every_kind_of_line_break() {
        assert_eq!(crlf(b"a\nb\r\nc\rd"), b"a\r\nb\r\nc\r\nd");
        assert_eq!(crlf(b"\r\n\r\n"), b"\r\n\r\n");
    }

    #[test]
    fn data_stuffs_dots_and_ends_with_the_dot_line() {
        let mut out = Vec::new();
        data(b"a\n.b\n.\n..c", &mut out).unwrap();
        assert_eq!(out, b"a\r\n..b\r\n..\r\n...c\r\n.\r\n");
        // The end marker never appears inside the data.
        let first = out.windows(5).position(|w| w == b"\r\n.\r\n").unwrap();
        assert_eq!(first, out.len() - 5);
    }

    #[test]
    fn data_rejects_empty_and_nul() {
        assert!(data(b"", &mut Vec::new()).is_err());
        assert!(data(b"a\0b", &mut Vec::new()).is_err());
    }

    #[test]
    fn wrapping_keeps_every_line_short_and_loses_no_word() {
        let text = "word ".repeat(100);
        let w = wrap_ascii(text.as_bytes());
        for l in w.split(|&b| b == b'\n') {
            assert!(l.len() <= MAX_LINE + 1, "{}", l.len());
        }
        let joined: Vec<&str> = std::str::from_utf8(&w)
            .unwrap()
            .split_whitespace()
            .collect();
        assert_eq!(joined.len(), 100);
        let solid = wrap_ascii(&[b'x'; 200]);
        assert_eq!(solid.iter().filter(|&&b| b == b'\n').count(), 2);
    }

    fn msg<'a>(body: &'a [u8], attachment: &'a [u8]) -> Message<'a> {
        Message {
            sender: "a.b@example.test",
            to: "c.d@example.test, e.f@example.test",
            cc: "",
            subject: "Quarterly report",
            body,
            attachment,
            filename: "report.pdf",
            content_type: "application/pdf",
            mailer: "FixingPointSystem Mail 0.1",
        }
    }

    fn render(m: &Message<'_>) -> Result<String, FnError> {
        let mut rng = rand_pcg::Pcg64Mcg::seed_from_u64(1);
        let mut out = Vec::new();
        m.render(&mut rng, &mut out)?;
        Ok(String::from_utf8(out).unwrap())
    }

    #[test]
    fn a_message_with_an_attachment_is_multipart() {
        let s = render(&msg(b"Hello\nworld\n", &[1, 2, 3, 4])).unwrap();
        assert!(s.starts_with("From: a.b@example.test\r\nTo: c.d@example.test, e.f@example.test\r\nSubject: Quarterly report\r\nDate: "));
        assert!(s.contains("Content-Type: multipart/mixed; boundary=\"=_fps_"));
        assert!(s.contains("Content-Disposition: attachment; filename=\"report.pdf\"\r\n"));
        assert!(s.contains("\r\n\r\nAQIDBA==\r\n--=_fps_"));
        assert!(s.ends_with("--\r\n"));
        assert!(!s.contains("\n\n") && !s.replace("\r\n", "").contains('\n'));
    }

    #[test]
    fn a_message_without_an_attachment_is_a_single_text_part() {
        let s = render(&msg(b"Hello\n", b"")).unwrap();
        assert!(s.contains("Content-Type: text/plain; charset=utf-8\r\n"));
        assert!(!s.contains("multipart"));
        assert!(s.ends_with("\r\n\r\nHello\r\n"));
    }

    #[test]
    fn non_ascii_text_and_subjects_are_encoded() {
        let mut m = msg("Grüße aus Köln\n".as_bytes(), b"");
        m.subject = "Übersicht für das vierte Quartal, Zusammenfassung";
        let s = render(&m).unwrap();
        assert!(s.is_ascii());
        assert!(s.contains("Content-Transfer-Encoding: base64"));
        assert!(s.contains("Subject: =?UTF-8?B?"));
        assert!(s.contains("\r\n =?UTF-8?B?"));
    }

    #[test]
    fn many_recipients_fold_below_the_line_limit() {
        let to: Vec<String> = (0..60).map(|i| format!("user{i}@example.test")).collect();
        let mut m = msg(b"x\n", b"");
        let joined = to.join(", ");
        m.to = &joined;
        let s = render(&m).unwrap();
        assert!(
            s.lines().all(|l| l.len() <= 80),
            "a header line is too long"
        );
        assert!(s.contains(",\r\n\tuser"));
    }

    #[test]
    fn dates_fall_in_2026_with_the_right_weekday() {
        for seed in 0..400 {
            let d = date(&mut Rng::new(seed));
            assert!(d.contains(" 2026 "), "{d}");
        }
        // The first day of the year is a Thursday, and the generator
        // reaches every month.
        let mut seen = std::collections::HashSet::new();
        for seed in 0..400 {
            let d = date(&mut Rng::new(seed));
            seen.insert(d.split(' ').nth(2).unwrap().to_string());
        }
        assert_eq!(seen.len(), 12);
    }

    #[test]
    fn unsafe_header_values_are_refused() {
        let cases: [fn(&mut Message<'_>); 7] = [
            |m: &mut Message<'_>| m.subject = "a\r\nBcc: x@example.test",
            |m: &mut Message<'_>| m.sender = "a@example.test\nBcc: x",
            |m: &mut Message<'_>| m.to = "",
            |m: &mut Message<'_>| m.filename = "a\".pdf",
            |m: &mut Message<'_>| m.content_type = "application",
            |m: &mut Message<'_>| m.content_type = "a/b\r\nX: y",
            |m: &mut Message<'_>| m.mailer = "m\r\n",
        ];
        for f in cases {
            let mut m = msg(b"x\n", b"");
            f(&mut m);
            assert_eq!(
                render(&m).unwrap_err().kind,
                FnErrorKind::InvalidArgument,
                "{}",
                m.subject
            );
        }
        assert!(render(&msg(&[0xff, 0xfe], b"")).is_err());
        assert!(render(&msg(b"a\0b", b"")).is_err());
    }

    #[test]
    fn the_same_generator_state_gives_the_same_message() {
        assert_eq!(
            render(&msg(b"x\n", &[1])).unwrap(),
            render(&msg(b"x\n", &[1])).unwrap()
        );
    }
}
