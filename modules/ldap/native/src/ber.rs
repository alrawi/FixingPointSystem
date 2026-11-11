// SPDX-License-Identifier: GPL-3.0-only
//! The BER subset that LDAP uses (RFC 4511 section 5.1).
//!
//! LDAP restricts BER to definite lengths and to tags below 31, so the codec
//! is small: a writer that builds one value at a time and a reader that walks
//! a buffer value by value. Both sides are written here, in this crate,
//! because no maintained permissively licensed crate offers a codec that
//! builds exactly the bytes of an LDAP client and checks a peer's bytes
//! without an ASN.1 schema.

use std::fmt;

/// `BOOLEAN`.
pub const BOOLEAN: u8 = 0x01;
/// `INTEGER`.
pub const INTEGER: u8 = 0x02;
/// `OCTET STRING`.
pub const OCTET_STRING: u8 = 0x04;
/// `ENUMERATED`.
pub const ENUMERATED: u8 = 0x0a;
/// `SEQUENCE`, constructed.
pub const SEQUENCE: u8 = 0x30;
/// `SET`, constructed.
pub const SET: u8 = 0x31;

/// The longest value that the reader accepts. The head of a message holds at
/// most two length octets, so no message is longer.
pub const MAX_LEN: usize = 0xFFFF;

/// A byte string that is not valid BER, or not valid for LDAP.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Malformed(pub String);

impl fmt::Display for Malformed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Malformed {}

/// Builds a [`Malformed`] error.
pub fn bad(message: impl Into<String>) -> Malformed {
    Malformed(message.into())
}

/// Appends the definite length octets of `len`, in the shortest form.
pub fn put_len(out: &mut Vec<u8>, len: usize) {
    if len < 0x80 {
        out.push(len as u8);
        return;
    }
    let bytes = (len as u64).to_be_bytes();
    let skip = bytes.iter().take_while(|b| **b == 0).count();
    out.push(0x80 | (8 - skip) as u8);
    out.extend_from_slice(&bytes[skip..]);
}

/// Returns the value with `tag`, the length, and `content`.
pub fn tlv(tag: u8, content: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(content.len() + 4);
    out.push(tag);
    put_len(&mut out, content.len());
    out.extend_from_slice(content);
    out
}

/// Returns the value with `tag` around the concatenation of `parts`.
pub fn constructed(tag: u8, parts: &[Vec<u8>]) -> Vec<u8> {
    let content: Vec<u8> = parts.iter().flatten().copied().collect();
    tlv(tag, &content)
}

/// The content octets of the integer `v`, in two's complement without a
/// redundant leading octet.
fn int_content(v: i64) -> Vec<u8> {
    let bytes = v.to_be_bytes();
    let mut start = 0;
    while start < 7 {
        let (a, b) = (bytes[start], bytes[start + 1]);
        if (a == 0x00 && b & 0x80 == 0) || (a == 0xff && b & 0x80 != 0) {
            start += 1;
        } else {
            break;
        }
    }
    bytes[start..].to_vec()
}

/// An integer with the universal tag `INTEGER`, or any other tag, such as
/// `ENUMERATED` for the result code.
pub fn integer(tag: u8, v: i64) -> Vec<u8> {
    tlv(tag, &int_content(v))
}

/// A `BOOLEAN`, with `0xff` for true as DER and BER agree.
pub fn boolean(b: bool) -> Vec<u8> {
    tlv(BOOLEAN, &[if b { 0xff } else { 0x00 }])
}

/// An `OCTET STRING` with the tag `tag`.
pub fn octets(tag: u8, bytes: &[u8]) -> Vec<u8> {
    tlv(tag, bytes)
}

/// One decoded value: the tag octet and the content.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tlv<'a> {
    /// The identifier octet.
    pub tag: u8,
    /// The content octets.
    pub content: &'a [u8],
}

impl<'a> Tlv<'a> {
    /// Returns a reader over the content, for a constructed value.
    pub fn reader(&self) -> Reader<'a> {
        Reader { buf: self.content }
    }

    /// Decodes the content as an integer or an enumerated value.
    pub fn int(&self) -> Result<i64, Malformed> {
        let c = self.content;
        if c.is_empty() || c.len() > 8 {
            return Err(bad(format!(
                "an integer has {} content octets, it needs 1 to 8",
                c.len()
            )));
        }
        let mut v: i64 = if c[0] & 0x80 != 0 { -1 } else { 0 };
        for b in c {
            v = (v << 8) | i64::from(*b);
        }
        Ok(v)
    }

    /// Decodes the content as a boolean.
    pub fn boolean(&self) -> Result<bool, Malformed> {
        match self.content {
            [b] => Ok(*b != 0),
            _ => Err(bad("a boolean needs exactly one content octet")),
        }
    }

    /// Decodes the content as UTF-8 text.
    pub fn text(&self) -> Result<&'a str, Malformed> {
        std::str::from_utf8(self.content).map_err(|_| bad("a string is not valid UTF-8"))
    }
}

/// Reads the head of the value at the start of `buf` and returns the tag, the
/// length of the head, and the length of the content. It does not check that
/// the content is present.
pub fn head(buf: &[u8]) -> Result<(u8, usize, usize), Malformed> {
    let (Some(&tag), Some(&first)) = (buf.first(), buf.get(1)) else {
        return Err(bad("the value is shorter than its head"));
    };
    if tag & 0x1f == 0x1f {
        return Err(bad("tags of 31 and above are not part of LDAP"));
    }
    if first < 0x80 {
        return Ok((tag, 2, usize::from(first)));
    }
    let n = usize::from(first & 0x7f);
    if n == 0 {
        return Err(bad("an indefinite length is not part of LDAP"));
    }
    if n > 4 {
        return Err(bad(format!("a length of {n} octets is too long")));
    }
    let Some(octets) = buf.get(2..2 + n) else {
        return Err(bad("the length octets are cut off"));
    };
    let len = octets
        .iter()
        .fold(0usize, |a, b| (a << 8) | usize::from(*b));
    Ok((tag, 2 + n, len))
}

/// A cursor over a buffer of consecutive values.
#[derive(Clone, Copy, Debug)]
pub struct Reader<'a> {
    buf: &'a [u8],
}

impl<'a> Reader<'a> {
    /// Starts reading `buf`.
    pub fn new(buf: &'a [u8]) -> Self {
        Reader { buf }
    }

    /// True when no byte is left.
    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }

    /// The tag of the next value, without consuming it.
    pub fn peek_tag(&self) -> Option<u8> {
        self.buf.first().copied()
    }

    /// Reads the next value of any tag.
    pub fn next_any(&mut self) -> Result<Tlv<'a>, Malformed> {
        let (tag, head_len, len) = head(self.buf)?;
        if len > MAX_LEN {
            return Err(bad(format!("a value of {len} octets is too long")));
        }
        let Some(content) = self.buf.get(head_len..head_len + len) else {
            return Err(bad(format!(
                "the value claims {len} content octets and the buffer holds {}",
                self.buf.len().saturating_sub(head_len)
            )));
        };
        self.buf = &self.buf[head_len + len..];
        Ok(Tlv { tag, content })
    }

    /// Reads the next value and requires the tag `tag`.
    pub fn next(&mut self, tag: u8, what: &str) -> Result<Tlv<'a>, Malformed> {
        let t = self.next_any()?;
        if t.tag != tag {
            return Err(bad(format!(
                "{what} has the tag 0x{:02x}, expected 0x{tag:02x}",
                t.tag
            )));
        }
        Ok(t)
    }

    /// Requires that the reader has consumed everything.
    pub fn finish(&self, what: &str) -> Result<(), Malformed> {
        if self.buf.is_empty() {
            Ok(())
        } else {
            Err(bad(format!(
                "{what} has {} octets left over",
                self.buf.len()
            )))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lengths_use_the_shortest_form() {
        for (len, want) in [
            (0usize, vec![0x00]),
            (127, vec![0x7f]),
            (128, vec![0x81, 0x80]),
            (255, vec![0x81, 0xff]),
            (256, vec![0x82, 0x01, 0x00]),
            (65_535, vec![0x82, 0xff, 0xff]),
            (65_536, vec![0x83, 0x01, 0x00, 0x00]),
        ] {
            let mut out = Vec::new();
            put_len(&mut out, len);
            assert_eq!(out, want, "{len}");
        }
    }

    #[test]
    fn integers_follow_the_x690_examples() {
        // The two's complement forms of X.690 section 8.3.
        for (v, want) in [
            (0i64, vec![0x00]),
            (1, vec![0x01]),
            (127, vec![0x7f]),
            (128, vec![0x00, 0x80]),
            (255, vec![0x00, 0xff]),
            (256, vec![0x01, 0x00]),
            (-1, vec![0xff]),
            (-128, vec![0x80]),
            (-129, vec![0xff, 0x7f]),
            (2_147_483_647, vec![0x7f, 0xff, 0xff, 0xff]),
            (i64::MIN, vec![0x80, 0, 0, 0, 0, 0, 0, 0]),
            (
                i64::MAX,
                vec![0x7f, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff],
            ),
        ] {
            let enc = integer(INTEGER, v);
            assert_eq!(&enc[2..], &want[..], "{v}");
            assert_eq!(enc[1] as usize, want.len());
            let mut r = Reader::new(&enc);
            assert_eq!(r.next(INTEGER, "x").unwrap().int().unwrap(), v);
        }
    }

    #[test]
    fn a_value_round_trips_through_the_reader() {
        let inner = vec![integer(INTEGER, 3), octets(OCTET_STRING, b"cn=admin")];
        let seq = constructed(SEQUENCE, &inner);
        let mut r = Reader::new(&seq);
        let t = r.next(SEQUENCE, "seq").unwrap();
        r.finish("buffer").unwrap();
        let mut c = t.reader();
        assert_eq!(c.next(INTEGER, "v").unwrap().int().unwrap(), 3);
        assert_eq!(
            c.next(OCTET_STRING, "n").unwrap().text().unwrap(),
            "cn=admin"
        );
        c.finish("seq").unwrap();
    }

    #[test]
    fn long_values_use_the_long_form_and_read_back() {
        let body = vec![b'x'; 300];
        let enc = octets(OCTET_STRING, &body);
        assert_eq!(&enc[..4], &[0x04, 0x82, 0x01, 0x2c]);
        let mut r = Reader::new(&enc);
        assert_eq!(r.next_any().unwrap().content.len(), 300);
    }

    #[test]
    fn the_reader_rejects_what_ldap_forbids() {
        for (bytes, why) in [
            (&[][..], "empty"),
            (&[0x30][..], "no length"),
            (&[0x30, 0x80, 0x00, 0x00][..], "indefinite length"),
            (&[0x1f, 0x01, 0x00][..], "high tag number"),
            (&[0x04, 0x05, 0x61][..], "cut off content"),
            (&[0x04, 0x85, 0, 0, 0, 0, 1, 0x61][..], "five length octets"),
            (&[0x04, 0x82, 0x01][..], "cut off length"),
            (&[0x04, 0x83, 0x01, 0x00, 0x00][..], "value over the limit"),
        ] {
            assert!(Reader::new(bytes).next_any().is_err(), "{why}");
        }
    }

    #[test]
    fn a_wrong_tag_and_trailing_bytes_are_named() {
        let enc = integer(INTEGER, 1);
        let err = Reader::new(&enc)
            .next(OCTET_STRING, "the name")
            .unwrap_err();
        assert!(
            err.0.contains("the name") && err.0.contains("0x02"),
            "{err}"
        );
        let mut two = enc.clone();
        two.extend_from_slice(&enc);
        let mut r = Reader::new(&two);
        r.next(INTEGER, "a").unwrap();
        assert!(r.finish("message").unwrap_err().0.contains("left over"));
    }

    #[test]
    fn integers_and_booleans_check_their_content_length() {
        assert!(Tlv {
            tag: INTEGER,
            content: &[]
        }
        .int()
        .is_err());
        assert!(Tlv {
            tag: INTEGER,
            content: &[0; 9]
        }
        .int()
        .is_err());
        assert!(Tlv {
            tag: BOOLEAN,
            content: &[]
        }
        .boolean()
        .is_err());
        assert!(Tlv {
            tag: BOOLEAN,
            content: &[0xff]
        }
        .boolean()
        .unwrap());
        assert!(!Tlv {
            tag: BOOLEAN,
            content: &[0]
        }
        .boolean()
        .unwrap());
        assert!(Tlv {
            tag: OCTET_STRING,
            content: &[0xff, 0xfe]
        }
        .text()
        .is_err());
    }
}
