// SPDX-License-Identifier: GPL-3.0-only
//! The data types of RFC 4251 section 5: `byte`, `boolean`, `uint32`,
//! `string`, `mpint`, and `name-list`.
//!
//! The reader never panics and never reads past its slice, so bytes from the
//! peer can only fail the calling session with a protocol error.

use fps_flow::{FnError, FnErrorKind};

/// Builds a protocol error, which fails the calling session.
pub(crate) fn protocol(message: impl Into<String>) -> FnError {
    FnError::new(FnErrorKind::Protocol, message)
}

/// Appends a `uint32`.
pub(crate) fn put_u32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_be_bytes());
}

/// Appends a `string`: a `uint32` length and the bytes.
pub(crate) fn put_string(out: &mut Vec<u8>, s: &[u8]) {
    // Every string of this module is far below 4 GiB, and `try_from` keeps
    // the cast honest.
    put_u32(out, u32::try_from(s.len()).unwrap_or(u32::MAX));
    out.extend_from_slice(s);
}

/// Appends a `boolean`.
pub(crate) fn put_bool(out: &mut Vec<u8>, v: bool) {
    out.push(u8::from(v));
}

/// Appends an `mpint` for the unsigned big endian integer `be`: leading zero
/// bytes drop, and a zero byte goes in front when the top bit is set, so the
/// number stays positive.
pub(crate) fn put_mpint(out: &mut Vec<u8>, be: &[u8]) {
    let start = be.iter().position(|b| *b != 0).unwrap_or(be.len());
    let digits = &be[start..];
    match digits.first() {
        None => put_u32(out, 0),
        Some(first) if first & 0x80 != 0 => {
            put_u32(out, u32::try_from(digits.len() + 1).unwrap_or(u32::MAX));
            out.push(0);
            out.extend_from_slice(digits);
        }
        Some(_) => put_string(out, digits),
    }
}

/// Reads the fields of one message.
pub(crate) struct Reader<'a> {
    buf: &'a [u8],
}

impl<'a> Reader<'a> {
    pub(crate) fn new(buf: &'a [u8]) -> Self {
        Reader { buf }
    }

    fn take(&mut self, n: usize, what: &str) -> Result<&'a [u8], FnError> {
        if self.buf.len() < n {
            return Err(protocol(format!(
                "message ends inside {what}: {n} bytes wanted, {} left",
                self.buf.len()
            )));
        }
        let (head, tail) = self.buf.split_at(n);
        self.buf = tail;
        Ok(head)
    }

    pub(crate) fn u8(&mut self, what: &str) -> Result<u8, FnError> {
        Ok(self.take(1, what)?[0])
    }

    pub(crate) fn boolean(&mut self, what: &str) -> Result<bool, FnError> {
        Ok(self.u8(what)? != 0)
    }

    pub(crate) fn u32(&mut self, what: &str) -> Result<u32, FnError> {
        let b = self.take(4, what)?;
        Ok(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }

    pub(crate) fn string(&mut self, what: &str) -> Result<&'a [u8], FnError> {
        let n = self.u32(what)? as usize;
        self.take(n, what)
    }

    /// Reads a `name-list` and returns its names.
    pub(crate) fn names(&mut self, what: &str) -> Result<Vec<&'a str>, FnError> {
        let raw = self.string(what)?;
        let text = std::str::from_utf8(raw)
            .map_err(|_| protocol(format!("{what} is not ASCII text")))?;
        Ok(if text.is_empty() {
            Vec::new()
        } else {
            text.split(',').collect()
        })
    }

    /// Skips `n` bytes.
    pub(crate) fn skip(&mut self, n: usize, what: &str) -> Result<(), FnError> {
        self.take(n, what).map(|_| ())
    }

    /// The bytes that no read has used yet.
    pub(crate) fn rest(&self) -> &'a [u8] {
        self.buf
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mpint_follows_the_rfc_4251_examples() {
        let mut out = Vec::new();
        put_mpint(&mut out, &[]);
        assert_eq!(out, [0, 0, 0, 0]);
        out.clear();
        put_mpint(&mut out, &[0, 0, 0x09, 0xa3, 0x78, 0xf9, 0xb2, 0xe3, 0x32, 0xa7]);
        assert_eq!(
            out,
            [0, 0, 0, 8, 0x09, 0xa3, 0x78, 0xf9, 0xb2, 0xe3, 0x32, 0xa7]
        );
        out.clear();
        put_mpint(&mut out, &[0x80]);
        assert_eq!(out, [0, 0, 0, 2, 0, 0x80]);
        out.clear();
        put_mpint(&mut out, &[0x7f, 0x01]);
        assert_eq!(out, [0, 0, 0, 2, 0x7f, 0x01]);
    }

    #[test]
    fn reader_stops_at_the_end_of_its_slice() {
        let mut out = Vec::new();
        put_string(&mut out, b"abc");
        put_u32(&mut out, 7);
        let mut r = Reader::new(&out);
        assert_eq!(r.string("a").unwrap(), b"abc");
        assert_eq!(r.u32("b").unwrap(), 7);
        assert!(r.u8("c").is_err());
        let mut short = Reader::new(&[0, 0, 0, 9, 1]);
        assert!(short.string("s").is_err());
    }

    #[test]
    fn names_split_on_commas() {
        let mut out = Vec::new();
        put_string(&mut out, b"a,b,c");
        put_string(&mut out, b"");
        let mut r = Reader::new(&out);
        assert_eq!(r.names("x").unwrap(), ["a", "b", "c"]);
        assert!(r.names("y").unwrap().is_empty());
    }
}
