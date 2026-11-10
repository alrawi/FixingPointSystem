// SPDX-License-Identifier: GPL-3.0-only
//! HPACK header compression as RFC 7541 defines it.
//!
//! [`Encoder`] turns header fields into a header block and keeps the dynamic
//! table of one direction of a connection, and [`Decoder`] reads header blocks
//! and keeps the table of the other direction. Both take every decision about
//! a field from the arguments and the table, so the same calls always give the
//! same bytes.
//!
//! # How the encoder represents a field
//!
//! The encoder decides per field, in this order.
//!
//! 1. A field that equals an entry of the static or the dynamic table becomes
//!    an indexed field (RFC 7541 section 6.1), one octet for most fields.
//! 2. A field called `authorization`, `proxy-authorization`, `cookie`, or
//!    `set-cookie` becomes a literal field never indexed (section 6.2.3), with
//!    the name index of the table when the name is in it. Intermediaries must
//!    not index such a field either, so it keeps secrets out of any table.
//! 3. With [`IndexMode::Off`], or when the entry would not fit in the dynamic
//!    table at all, the field becomes a literal field without indexing
//!    (section 6.2.2).
//! 4. Any other field becomes a literal field with incremental indexing
//!    (section 6.2.1) and joins the dynamic table, as browsers and nghttp2 do.
//!
//! [`IndexMode::All`] skips rule 2 and indexes sensitive fields too.
//!
//! A literal takes the name from the tables when it can, and a string literal
//! is Huffman coded when [`HuffmanMode::Auto`] finds the coding strictly
//! shorter, always with [`HuffmanMode::Always`], and never with
//! [`HuffmanMode::Never`].

use std::collections::VecDeque;
use std::fmt;

use crate::huffman::{self, HuffmanError};
use crate::static_table::STATIC_TABLE;

/// The size of the dynamic table that RFC 9113 section 6.5.2 gives to a
/// connection before any SETTINGS frame changes it.
pub const DEFAULT_TABLE_SIZE: usize = 4096;

/// The largest dynamic table that [`Encoder`] and [`Decoder`] accept. A peer
/// may announce 2^32 - 1, and the module refuses to hold more than 1 MiB.
pub const MAX_TABLE_SIZE: usize = 1 << 20;

/// The longest string literal that the decoder reads, in octets.
pub const MAX_STRING: usize = 1 << 20;

/// The most that one decoded header list may hold, counted as RFC 9113
/// section 6.5.2 counts it: name and value length plus 32 for each field.
pub const MAX_LIST_SIZE: usize = 1 << 20;

/// Entries of the static table.
const STATIC_LEN: usize = STATIC_TABLE.len();

/// Names whose fields the encoder never indexes, because they carry secrets.
const SENSITIVE: [&[u8]; 4] = [
    b"authorization",
    b"proxy-authorization",
    b"cookie",
    b"set-cookie",
];

/// Whether the encoder Huffman codes a string literal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HuffmanMode {
    /// Code the string when that makes it strictly shorter.
    Auto,
    /// Code every string.
    Always,
    /// Write every string as it is.
    Never,
}

/// Whether the encoder adds fields to the dynamic table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IndexMode {
    /// Index the fields that the rules in the module documentation allow.
    Auto,
    /// Write literals without indexing, so the table does not grow.
    Off,
    /// Index every field that fits, also the ones that [`IndexMode::Auto`]
    /// keeps out of the table because they carry secrets. The RFC 7541
    /// Appendix C examples index a `set-cookie` field this way.
    All,
}

/// Why a header block cannot be decoded. RFC 7541 section 2.3.3 treats each
/// of them as a decoding error, which RFC 9113 maps to COMPRESSION_ERROR.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HpackError {
    /// The block ends inside a field.
    Truncated,
    /// An integer does not fit in 32 bits.
    IntegerOverflow,
    /// A field refers to an index that no table entry has, or to index 0.
    InvalidIndex(usize),
    /// A Huffman coded string is invalid.
    Huffman(HuffmanError),
    /// A dynamic table size update follows a header field.
    SizeUpdateNotFirst,
    /// A size update asks for more than the decoder allows.
    SizeUpdateTooLarge {
        /// The size in the update.
        requested: usize,
        /// The largest size that the decoder allows.
        allowed: usize,
    },
    /// A string literal or the header list exceeds the limit of the decoder.
    TooLarge,
}

impl fmt::Display for HpackError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HpackError::Truncated => f.write_str("header block ends inside a field"),
            HpackError::IntegerOverflow => {
                f.write_str("integer in the header block exceeds 32 bits")
            }
            HpackError::InvalidIndex(i) => write!(
                f,
                "header block refers to table index {i}, which does not exist"
            ),
            HpackError::Huffman(e) => write!(f, "{e}"),
            HpackError::SizeUpdateNotFirst => {
                f.write_str("dynamic table size update follows a header field")
            }
            HpackError::SizeUpdateTooLarge { requested, allowed } => write!(
                f,
                "dynamic table size update to {requested} exceeds the allowed {allowed}"
            ),
            HpackError::TooLarge => {
                f.write_str("header block exceeds the size limits of the decoder")
            }
        }
    }
}

impl std::error::Error for HpackError {}

impl From<HuffmanError> for HpackError {
    fn from(e: HuffmanError) -> Self {
        HpackError::Huffman(e)
    }
}

/// The dynamic table of RFC 7541 section 2.3.2, newest entry first.
#[derive(Clone, Debug)]
struct DynTable {
    entries: VecDeque<(Vec<u8>, Vec<u8>)>,
    size: usize,
    max: usize,
}

fn entry_size(name: &[u8], value: &[u8]) -> usize {
    name.len() + value.len() + 32
}

impl DynTable {
    fn new(max: usize) -> Self {
        DynTable {
            entries: VecDeque::new(),
            size: 0,
            max,
        }
    }

    fn evict_to(&mut self, limit: usize) {
        while self.size > limit {
            match self.entries.pop_back() {
                Some((n, v)) => self.size -= entry_size(&n, &v),
                None => {
                    self.size = 0;
                    break;
                }
            }
        }
    }

    fn insert(&mut self, name: Vec<u8>, value: Vec<u8>) {
        let size = entry_size(&name, &value);
        if size > self.max {
            // RFC 7541 section 4.4: an entry larger than the table empties it.
            self.entries.clear();
            self.size = 0;
            return;
        }
        self.evict_to(self.max - size);
        self.size += size;
        self.entries.push_front((name, value));
    }

    fn set_max(&mut self, max: usize) {
        self.max = max;
        self.evict_to(max);
    }

    /// Returns the entry at the 1 based index of the whole address space.
    fn get(&self, index: usize) -> Option<(&[u8], &[u8])> {
        if index == 0 {
            return None;
        }
        if index <= STATIC_LEN {
            let (n, v) = STATIC_TABLE[index - 1];
            return Some((n.as_bytes(), v.as_bytes()));
        }
        self.entries
            .get(index - STATIC_LEN - 1)
            .map(|(n, v)| (n.as_slice(), v.as_slice()))
    }
}

fn put_int(out: &mut Vec<u8>, pattern: u8, prefix: u8, mut value: usize) {
    let max = (1usize << prefix) - 1;
    if value < max {
        out.push(pattern | value as u8);
        return;
    }
    out.push(pattern | max as u8);
    value -= max;
    while value >= 128 {
        out.push((value & 0x7f) as u8 | 0x80);
        value >>= 7;
    }
    out.push(value as u8);
}

fn put_string(out: &mut Vec<u8>, data: &[u8], mode: HuffmanMode) {
    let coded = match mode {
        HuffmanMode::Never => false,
        HuffmanMode::Always => true,
        HuffmanMode::Auto => huffman::encoded_len(data) < data.len(),
    };
    if coded {
        put_int(out, 0x80, 7, huffman::encoded_len(data));
        huffman::encode(data, out);
    } else {
        put_int(out, 0x00, 7, data.len());
        out.extend_from_slice(data);
    }
}

/// The encoder side of HPACK for one direction of a connection.
#[derive(Clone, Debug)]
pub struct Encoder {
    table: DynTable,
}

impl Default for Encoder {
    fn default() -> Self {
        Encoder::new()
    }
}

impl Encoder {
    /// Returns an encoder with an empty table of [`DEFAULT_TABLE_SIZE`].
    pub fn new() -> Self {
        Encoder::with_max_size(DEFAULT_TABLE_SIZE)
    }

    /// Returns an encoder with an empty table of `size` octets, for a peer
    /// that announced that size before the first block, so the encoder writes
    /// no size update. The size is capped at [`MAX_TABLE_SIZE`].
    pub fn with_max_size(size: usize) -> Self {
        Encoder {
            table: DynTable::new(size.min(MAX_TABLE_SIZE)),
        }
    }

    /// Returns the current maximum size of the dynamic table.
    pub fn max_size(&self) -> usize {
        self.table.max
    }

    /// Returns the number of octets that the dynamic table holds, counted as
    /// RFC 7541 section 4.1 counts them.
    pub fn size(&self) -> usize {
        self.table.size
    }

    /// Returns the number of entries of the dynamic table.
    pub fn len(&self) -> usize {
        self.table.entries.len()
    }

    /// Returns true when the dynamic table is empty.
    pub fn is_empty(&self) -> bool {
        self.table.entries.is_empty()
    }

    /// Changes the maximum size of the dynamic table and writes the dynamic
    /// table size update (RFC 7541 section 6.3) that tells the decoder. Call it
    /// before the first field of a block. The size must not exceed
    /// [`MAX_TABLE_SIZE`].
    pub fn set_max_size(&mut self, size: usize, out: &mut Vec<u8>) {
        let size = size.min(MAX_TABLE_SIZE);
        put_int(out, 0x20, 5, size);
        self.table.set_max(size);
    }

    fn find(&self, name: &[u8], value: &[u8]) -> (Option<usize>, Option<usize>) {
        let mut name_only = None;
        for (i, (n, v)) in STATIC_TABLE.iter().enumerate() {
            if n.as_bytes() == name {
                if v.as_bytes() == value {
                    return (Some(i + 1), Some(i + 1));
                }
                name_only.get_or_insert(i + 1);
            }
        }
        for (i, (n, v)) in self.table.entries.iter().enumerate() {
            if n == name {
                if v == value {
                    return (Some(STATIC_LEN + 1 + i), Some(STATIC_LEN + 1 + i));
                }
                name_only.get_or_insert(STATIC_LEN + 1 + i);
            }
        }
        (None, name_only)
    }

    /// Appends the representation of one header field.
    ///
    /// The name must be lowercase, as RFC 9113 section 8.2.1 requires, but the
    /// encoder does not check that.
    pub fn encode(
        &mut self,
        name: &[u8],
        value: &[u8],
        huffman: HuffmanMode,
        index: IndexMode,
        out: &mut Vec<u8>,
    ) {
        let (exact, name_index) = self.find(name, value);
        if let Some(i) = exact {
            put_int(out, 0x80, 7, i);
            return;
        }
        let sensitive = SENSITIVE.contains(&name) && index != IndexMode::All;
        let fits = entry_size(name, value) <= self.table.max;
        let (pattern, prefix) = if sensitive {
            (0x10, 4)
        } else if index == IndexMode::Off || !fits {
            (0x00, 4)
        } else {
            (0x40, 6)
        };
        put_int(out, pattern, prefix, name_index.unwrap_or(0));
        if name_index.is_none() {
            put_string(out, name, huffman);
        }
        put_string(out, value, huffman);
        if pattern == 0x40 {
            self.table.insert(name.to_vec(), value.to_vec());
        }
    }
}

/// One decoded header field.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Field {
    /// The field name.
    pub name: Vec<u8>,
    /// The field value.
    pub value: Vec<u8>,
    /// True when the block carried the field as never indexed.
    pub sensitive: bool,
}

/// The decoder side of HPACK for one direction of a connection.
#[derive(Clone, Debug)]
pub struct Decoder {
    table: DynTable,
    allowed: usize,
}

impl Default for Decoder {
    fn default() -> Self {
        Decoder::new()
    }
}

fn get_int(buf: &[u8], pos: &mut usize, prefix: u8) -> Result<usize, HpackError> {
    let max = (1u32 << prefix) - 1;
    let first = *buf.get(*pos).ok_or(HpackError::Truncated)?;
    *pos += 1;
    let first = u32::from(first) & max;
    if first < max {
        return Ok(first as usize);
    }
    let mut value = u64::from(max);
    let mut shift = 0u32;
    loop {
        let b = *buf.get(*pos).ok_or(HpackError::Truncated)?;
        *pos += 1;
        value += u64::from(b & 0x7f) << shift;
        if value > u64::from(u32::MAX) {
            return Err(HpackError::IntegerOverflow);
        }
        if b & 0x80 == 0 {
            return Ok(value as usize);
        }
        shift += 7;
        if shift > 28 {
            return Err(HpackError::IntegerOverflow);
        }
    }
}

fn get_string(buf: &[u8], pos: &mut usize) -> Result<Vec<u8>, HpackError> {
    let coded = buf.get(*pos).ok_or(HpackError::Truncated)? & 0x80 != 0;
    let len = get_int(buf, pos, 7)?;
    if len > MAX_STRING {
        return Err(HpackError::TooLarge);
    }
    let end = pos.checked_add(len).ok_or(HpackError::Truncated)?;
    let raw = buf.get(*pos..end).ok_or(HpackError::Truncated)?;
    *pos = end;
    if coded {
        let mut out = Vec::with_capacity(len + len / 2);
        huffman::decode(raw, &mut out)?;
        Ok(out)
    } else {
        Ok(raw.to_vec())
    }
}

impl Decoder {
    /// Returns a decoder with an empty table of [`DEFAULT_TABLE_SIZE`] that
    /// accepts size updates up to the same size.
    pub fn new() -> Self {
        Decoder::with_max_size(DEFAULT_TABLE_SIZE)
    }

    /// Returns a decoder with an empty table of `size` octets that accepts
    /// size updates up to the same size. The size is capped at
    /// [`MAX_TABLE_SIZE`].
    pub fn with_max_size(size: usize) -> Self {
        let size = size.min(MAX_TABLE_SIZE);
        Decoder {
            table: DynTable::new(size),
            allowed: size,
        }
    }

    /// Sets the largest table size that a size update may ask for, which is
    /// the `SETTINGS_HEADER_TABLE_SIZE` that this side announced. The value is
    /// capped at [`MAX_TABLE_SIZE`].
    pub fn set_allowed_size(&mut self, size: usize) {
        self.allowed = size.min(MAX_TABLE_SIZE);
    }

    /// Returns the current maximum size of the dynamic table.
    pub fn max_size(&self) -> usize {
        self.table.max
    }

    /// Returns the number of octets that the dynamic table holds, counted as
    /// RFC 7541 section 4.1 counts them.
    pub fn size(&self) -> usize {
        self.table.size
    }

    /// Returns the number of entries of the dynamic table.
    pub fn len(&self) -> usize {
        self.table.entries.len()
    }

    /// Returns true when the dynamic table is empty.
    pub fn is_empty(&self) -> bool {
        self.table.entries.is_empty()
    }

    fn name_at(&self, index: usize) -> Result<Vec<u8>, HpackError> {
        self.table
            .get(index)
            .map(|(n, _)| n.to_vec())
            .ok_or(HpackError::InvalidIndex(index))
    }

    /// Decodes one complete header block into its fields. After an error the
    /// table is in an undefined state, and the connection is lost, as RFC 7541
    /// section 2.3.3 says.
    pub fn decode(&mut self, block: &[u8]) -> Result<Vec<Field>, HpackError> {
        let mut fields = Vec::new();
        let mut pos = 0usize;
        let mut list_size = 0usize;
        while let Some(&first) = block.get(pos) {
            if first & 0x80 != 0 {
                let index = get_int(block, &mut pos, 7)?;
                let (name, value) = self
                    .table
                    .get(index)
                    .map(|(n, v)| (n.to_vec(), v.to_vec()))
                    .ok_or(HpackError::InvalidIndex(index))?;
                list_size += entry_size(&name, &value);
                fields.push(Field {
                    name,
                    value,
                    sensitive: false,
                });
            } else if first & 0xe0 == 0x20 {
                if !fields.is_empty() {
                    return Err(HpackError::SizeUpdateNotFirst);
                }
                let size = get_int(block, &mut pos, 5)?;
                if size > self.allowed {
                    return Err(HpackError::SizeUpdateTooLarge {
                        requested: size,
                        allowed: self.allowed,
                    });
                }
                self.table.set_max(size);
            } else {
                let incremental = first & 0xc0 == 0x40;
                let sensitive = first & 0xf0 == 0x10;
                let index = get_int(block, &mut pos, if incremental { 6 } else { 4 })?;
                let name = if index == 0 {
                    get_string(block, &mut pos)?
                } else {
                    self.name_at(index)?
                };
                let value = get_string(block, &mut pos)?;
                list_size += entry_size(&name, &value);
                if incremental {
                    self.table.insert(name.clone(), value.clone());
                }
                fields.push(Field {
                    name,
                    value,
                    sensitive,
                });
            }
            if list_size > MAX_LIST_SIZE {
                return Err(HpackError::TooLarge);
            }
        }
        Ok(fields)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn enc(fields: &[(&str, &str)], huffman: HuffmanMode, e: &mut Encoder) -> Vec<u8> {
        let mut out = Vec::new();
        for (n, v) in fields {
            e.encode(
                n.as_bytes(),
                v.as_bytes(),
                huffman,
                IndexMode::Auto,
                &mut out,
            );
        }
        out
    }

    #[test]
    fn integers_follow_rfc_7541_appendix_c1() {
        // C.1.1: 10 in a 5 bit prefix, C.1.2: 1337, C.1.3: 42 at an octet boundary.
        let mut out = Vec::new();
        put_int(&mut out, 0, 5, 10);
        assert_eq!(out, [0x0a]);
        out.clear();
        put_int(&mut out, 0, 5, 1337);
        assert_eq!(out, [0x1f, 0x9a, 0x0a]);
        out.clear();
        put_int(&mut out, 0, 8, 42);
        assert_eq!(out, [0x2a]);
        for (bytes, prefix, want) in [
            (&[0x0a][..], 5, 10),
            (&[0x1f, 0x9a, 0x0a], 5, 1337),
            (&[0x2a], 8, 42),
        ] {
            let mut pos = 0;
            assert_eq!(get_int(bytes, &mut pos, prefix).unwrap(), want);
            assert_eq!(pos, bytes.len());
        }
    }

    #[test]
    fn integer_decoding_rejects_overflow_and_truncation() {
        let mut pos = 0;
        assert_eq!(
            get_int(&[0x7f, 0xff, 0xff, 0xff, 0xff, 0x7f], &mut pos, 7),
            Err(HpackError::IntegerOverflow)
        );
        pos = 0;
        assert_eq!(
            get_int(&[0x7f, 0xff], &mut pos, 7),
            Err(HpackError::Truncated)
        );
        pos = 0;
        assert_eq!(
            get_int(&[0x7f, 0x80, 0x80, 0x80, 0x80, 0x80, 0x01], &mut pos, 7),
            Err(HpackError::IntegerOverflow)
        );
    }

    #[test]
    fn the_static_table_has_the_entries_that_the_rfc_numbers() {
        let t = DynTable::new(0);
        assert_eq!(t.get(1), Some((&b":authority"[..], &b""[..])));
        assert_eq!(t.get(2), Some((&b":method"[..], &b"GET"[..])));
        assert_eq!(t.get(8), Some((&b":status"[..], &b"200"[..])));
        assert_eq!(
            t.get(16),
            Some((&b"accept-encoding"[..], &b"gzip, deflate"[..]))
        );
        assert_eq!(t.get(61), Some((&b"www-authenticate"[..], &b""[..])));
        assert_eq!(t.get(0), None);
        assert_eq!(t.get(62), None);
    }

    #[test]
    fn the_table_evicts_the_oldest_entries_first() {
        let mut t = DynTable::new(100);
        t.insert(b"a".to_vec(), b"1".to_vec()); // 34
        t.insert(b"b".to_vec(), b"2".to_vec()); // 34
        t.insert(b"c".to_vec(), b"3".to_vec()); // 34, total 102 evicts a
        assert_eq!(t.entries.len(), 2);
        assert_eq!(t.get(62), Some((&b"c"[..], &b"3"[..])));
        assert_eq!(t.get(63), Some((&b"b"[..], &b"2"[..])));
        // An entry that cannot fit empties the table.
        t.insert(vec![b'x'; 100], Vec::new());
        assert!(t.entries.is_empty());
        assert_eq!(t.size, 0);
    }

    #[test]
    fn sensitive_fields_are_never_indexed() {
        let mut e = Encoder::new();
        let out = enc(
            &[("authorization", "Bearer abc")],
            HuffmanMode::Never,
            &mut e,
        );
        // 0001 prefix with name index 23 (authorization), then a raw value.
        assert_eq!(out[0], 0x10 | 15);
        assert_eq!(out[1], 23 - 15);
        assert!(e.is_empty());
    }

    #[test]
    fn auto_huffman_codes_only_when_that_is_shorter() {
        let mut out = Vec::new();
        put_string(&mut out, b"www.example.com", HuffmanMode::Auto);
        assert_eq!(out[0], 0x80 | 12);
        out.clear();
        // Octets above 0x7f have codes longer than 8 bits.
        put_string(&mut out, &[0xff, 0xfe, 0xfd], HuffmanMode::Auto);
        assert_eq!(out[0], 3);
        out.clear();
        put_string(&mut out, b"", HuffmanMode::Auto);
        assert_eq!(out, [0]);
    }

    #[test]
    fn size_updates_resize_and_evict() {
        let mut e = Encoder::new();
        enc(&[("x-a", "1"), ("x-b", "2")], HuffmanMode::Never, &mut e);
        assert_eq!(e.len(), 2);
        let mut out = Vec::new();
        e.set_max_size(0, &mut out);
        assert_eq!(out, [0x20]);
        assert!(e.is_empty());
        out.clear();
        e.set_max_size(4096, &mut out);
        assert_eq!(out, [0x3f, 0xe1, 0x1f]);
    }

    #[test]
    fn the_decoder_follows_what_the_encoder_wrote() {
        let mut e = Encoder::new();
        let mut d = Decoder::new();
        let lists: [&[(&str, &str)]; 3] = [
            &[(":method", "GET"), (":path", "/a"), ("x-token", "abc")],
            &[(":method", "GET"), (":path", "/b"), ("x-token", "abc")],
            &[("x-token", "abc"), ("cookie", "k=v")],
        ];
        for list in lists {
            let block = enc(list, HuffmanMode::Auto, &mut e);
            let got = d.decode(&block).unwrap();
            let got: Vec<(String, String)> = got
                .into_iter()
                .map(|f| {
                    (
                        String::from_utf8(f.name).unwrap(),
                        String::from_utf8(f.value).unwrap(),
                    )
                })
                .collect();
            let want: Vec<(String, String)> = list
                .iter()
                .map(|(n, v)| (n.to_string(), v.to_string()))
                .collect();
            assert_eq!(got, want);
        }
        assert_eq!(e.len(), d.len());
    }

    #[test]
    fn the_decoder_refuses_malformed_blocks() {
        let mut d = Decoder::new();
        assert_eq!(d.decode(&[0x80]), Err(HpackError::InvalidIndex(0)));
        assert_eq!(d.decode(&[0xff, 0x00]), Err(HpackError::InvalidIndex(127)));
        assert_eq!(d.decode(&[0x40]), Err(HpackError::Truncated));
        assert_eq!(d.decode(&[0x40, 0x05, b'a']), Err(HpackError::Truncated));
        // A size update after a field.
        assert_eq!(d.decode(&[0x82, 0x20]), Err(HpackError::SizeUpdateNotFirst));
        // A size update above the allowed size.
        assert_eq!(
            d.decode(&[0x3f, 0xe2, 0x1f]),
            Err(HpackError::SizeUpdateTooLarge {
                requested: 4097,
                allowed: 4096
            })
        );
        // A literal with a Huffman string that holds the end of string symbol.
        assert!(matches!(
            d.decode(&[0x00, 0x84, 0xff, 0xff, 0xff, 0xff, 0x00]),
            Err(HpackError::Huffman(HuffmanError::Eos))
        ));
    }

    #[test]
    fn a_string_longer_than_the_limit_is_refused_without_allocating_it() {
        let mut d = Decoder::new();
        // A literal whose name announces 2 MiB.
        let block = [0x40, 0x7f, 0x81, 0xff, 0x7f];
        assert_eq!(d.decode(&block), Err(HpackError::TooLarge));
    }
}
