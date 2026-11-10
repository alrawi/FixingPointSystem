// SPDX-License-Identifier: GPL-3.0-only
//! HTTP/2 frames as RFC 9113 section 4 and 6 define them.
//!
//! The encoders append whole frames to a buffer, and the parsers read frames
//! out of untrusted bytes. A parser checks every length before it indexes, so
//! no input makes it panic.

use std::fmt;

/// The client connection preface of RFC 9113 section 3.4.
pub const PREFACE: &[u8; 24] = b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n";

/// The length of the frame header in octets.
pub const HEADER_LEN: usize = 9;

/// The largest payload that the 24 bit length field can announce.
pub const MAX_PAYLOAD: usize = 0xFF_FFFF;

/// The largest stream id, which has 31 bits.
pub const MAX_STREAM_ID: u32 = 0x7FFF_FFFF;

/// Frame type codes of RFC 9113 section 6.
pub mod ty {
    /// DATA.
    pub const DATA: u8 = 0x0;
    /// HEADERS.
    pub const HEADERS: u8 = 0x1;
    /// PRIORITY.
    pub const PRIORITY: u8 = 0x2;
    /// RST_STREAM.
    pub const RST_STREAM: u8 = 0x3;
    /// SETTINGS.
    pub const SETTINGS: u8 = 0x4;
    /// PUSH_PROMISE.
    pub const PUSH_PROMISE: u8 = 0x5;
    /// PING.
    pub const PING: u8 = 0x6;
    /// GOAWAY.
    pub const GOAWAY: u8 = 0x7;
    /// WINDOW_UPDATE.
    pub const WINDOW_UPDATE: u8 = 0x8;
    /// CONTINUATION.
    pub const CONTINUATION: u8 = 0x9;
}

/// Frame flags of RFC 9113 section 6.
pub mod flag {
    /// END_STREAM on DATA and HEADERS.
    pub const END_STREAM: u8 = 0x1;
    /// ACK on SETTINGS and PING.
    pub const ACK: u8 = 0x1;
    /// END_HEADERS on HEADERS and CONTINUATION.
    pub const END_HEADERS: u8 = 0x4;
    /// PADDED on DATA and HEADERS.
    pub const PADDED: u8 = 0x8;
    /// PRIORITY on HEADERS.
    pub const PRIORITY: u8 = 0x20;
}

/// Why a frame could not be built or read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FrameError {
    /// The input ends before the frame does.
    Short {
        /// What was being read.
        what: &'static str,
        /// Octets that the frame needs.
        need: usize,
        /// Octets that the input holds.
        have: usize,
    },
    /// A field has a value that the frame type forbids.
    Invalid(String),
}

impl fmt::Display for FrameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FrameError::Short { what, need, have } => {
                write!(f, "{what} needs {need} octets, the input has {have}")
            }
            FrameError::Invalid(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for FrameError {}

fn invalid(message: impl Into<String>) -> FrameError {
    FrameError::Invalid(message.into())
}

/// The 9 octet header of a frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameHeader {
    /// Payload length in octets.
    pub length: u32,
    /// Frame type code.
    pub ty: u8,
    /// Frame flags.
    pub flags: u8,
    /// Stream id without the reserved bit.
    pub stream: u32,
}

impl FrameHeader {
    /// Reads the header at the start of `bytes`. The reserved bit of the
    /// stream id is ignored, as RFC 9113 section 4.1 requires.
    pub fn parse(bytes: &[u8]) -> Result<FrameHeader, FrameError> {
        let Some(h) = bytes.get(..HEADER_LEN) else {
            return Err(FrameError::Short {
                what: "a frame header",
                need: HEADER_LEN,
                have: bytes.len(),
            });
        };
        Ok(FrameHeader {
            length: u32::from_be_bytes([0, h[0], h[1], h[2]]),
            ty: h[3],
            flags: h[4],
            stream: u32::from_be_bytes([h[5], h[6], h[7], h[8]]) & MAX_STREAM_ID,
        })
    }

    /// Appends the 9 octets of the header.
    pub fn write(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.length.to_be_bytes()[1..]);
        out.push(self.ty);
        out.push(self.flags);
        out.extend_from_slice(&(self.stream & MAX_STREAM_ID).to_be_bytes());
    }

    /// Returns true when `flag` is set.
    pub fn has(&self, flag: u8) -> bool {
        self.flags & flag != 0
    }
}

/// One frame, borrowed from the input.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Frame<'a> {
    /// The header.
    pub header: FrameHeader,
    /// The payload, `header.length` octets.
    pub payload: &'a [u8],
}

/// Reads the frame at the start of `bytes` and returns it with the octets
/// that follow it.
pub fn split(bytes: &[u8]) -> Result<(Frame<'_>, &[u8]), FrameError> {
    let header = FrameHeader::parse(bytes)?;
    let end = HEADER_LEN + header.length as usize;
    let (Some(payload), Some(rest)) = (bytes.get(HEADER_LEN..end), bytes.get(end..)) else {
        return Err(FrameError::Short {
            what: "a frame payload",
            need: end,
            have: bytes.len(),
        });
    };
    Ok((Frame { header, payload }, rest))
}

/// Returns the name of a frame type, such as `HEADERS`, or `UNKNOWN_<n>`.
pub fn type_name(ty: u8) -> String {
    match ty {
        self::ty::DATA => "DATA".into(),
        self::ty::HEADERS => "HEADERS".into(),
        self::ty::PRIORITY => "PRIORITY".into(),
        self::ty::RST_STREAM => "RST_STREAM".into(),
        self::ty::SETTINGS => "SETTINGS".into(),
        self::ty::PUSH_PROMISE => "PUSH_PROMISE".into(),
        self::ty::PING => "PING".into(),
        self::ty::GOAWAY => "GOAWAY".into(),
        self::ty::WINDOW_UPDATE => "WINDOW_UPDATE".into(),
        self::ty::CONTINUATION => "CONTINUATION".into(),
        other => format!("UNKNOWN_{other}"),
    }
}

/// The error codes of RFC 9113 section 7 by name.
const ERROR_NAMES: [&str; 14] = [
    "NO_ERROR",
    "PROTOCOL_ERROR",
    "INTERNAL_ERROR",
    "FLOW_CONTROL_ERROR",
    "SETTINGS_TIMEOUT",
    "STREAM_CLOSED",
    "FRAME_SIZE_ERROR",
    "REFUSED_STREAM",
    "CANCEL",
    "COMPRESSION_ERROR",
    "CONNECT_ERROR",
    "ENHANCE_YOUR_CALM",
    "INADEQUATE_SECURITY",
    "HTTP_1_1_REQUIRED",
];

/// Returns the code of an error name such as `CANCEL`, in any letter case.
pub fn error_code(name: &str) -> Option<u32> {
    ERROR_NAMES
        .iter()
        .position(|n| n.eq_ignore_ascii_case(name))
        .map(|i| i as u32)
}

fn check_stream(what: &str, stream: i64, allow_zero: bool) -> Result<u32, FrameError> {
    let ok = if allow_zero {
        (0..=i64::from(MAX_STREAM_ID)).contains(&stream)
    } else {
        (1..=i64::from(MAX_STREAM_ID)).contains(&stream)
    };
    if !ok {
        let low = u8::from(!allow_zero);
        return Err(invalid(format!(
            "stream of {what} must be between {low} and {MAX_STREAM_ID}, not {stream}"
        )));
    }
    Ok(stream as u32)
}

fn put(
    out: &mut Vec<u8>,
    ty: u8,
    flags: u8,
    stream: u32,
    payload_len: usize,
) -> Result<(), FrameError> {
    if payload_len > MAX_PAYLOAD {
        return Err(invalid(format!(
            "a frame payload of {payload_len} octets exceeds the 24 bit length field"
        )));
    }
    FrameHeader {
        length: payload_len as u32,
        ty,
        flags,
        stream,
    }
    .write(out);
    Ok(())
}

/// The priority fields of a HEADERS frame (RFC 9113 section 6.2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Priority {
    /// The stream that this stream depends on.
    pub dependency: u32,
    /// The weight, 1 to 256.
    pub weight: u16,
    /// The exclusive flag.
    pub exclusive: bool,
}

/// Appends a frame of any type.
pub fn raw(
    out: &mut Vec<u8>,
    ty: u8,
    flags: u8,
    stream: i64,
    payload: &[u8],
) -> Result<(), FrameError> {
    let stream = check_stream("a frame", stream, true)?;
    put(out, ty, flags, stream, payload.len())?;
    out.extend_from_slice(payload);
    Ok(())
}

/// Checks the frame size limit that a caller passes.
pub fn check_max_frame_size(max: i64) -> Result<usize, FrameError> {
    if !(1..=MAX_PAYLOAD as i64).contains(&max) {
        return Err(invalid(format!(
            "max_frame_size must be between 1 and {MAX_PAYLOAD}, not {max}"
        )));
    }
    Ok(max as usize)
}

/// Turns the `padding` argument into a pad length. -1 means no padding, and
/// 0 to 255 means a PADDED frame with that many zero octets.
pub fn check_padding(padding: i64) -> Result<Option<u8>, FrameError> {
    match padding {
        -1 => Ok(None),
        0..=255 => Ok(Some(padding as u8)),
        _ => Err(invalid(format!(
            "padding must be -1 or between 0 and 255, not {padding}"
        ))),
    }
}

/// Appends DATA frames that carry `data`, each with at most `max` octets of
/// payload including its padding fields.
pub fn data(
    out: &mut Vec<u8>,
    stream: i64,
    data: &[u8],
    end_stream: bool,
    padding: Option<u8>,
    max: usize,
) -> Result<(), FrameError> {
    let stream = check_stream("DATA", stream, false)?;
    let overhead = padding.map_or(0, |p| 1 + usize::from(p));
    let room = max
        .checked_sub(overhead)
        .filter(|r| *r > 0)
        .ok_or_else(|| {
            invalid(format!(
                "padding of {overhead} octets leaves no room for data in a frame of {max} octets"
            ))
        })?;
    let mut chunks = data.chunks(room).peekable();
    // Empty data still renders one frame, which may carry END_STREAM.
    if chunks.peek().is_none() {
        return data_frame(out, stream, &[], end_stream, padding);
    }
    while let Some(chunk) = chunks.next() {
        data_frame(
            out,
            stream,
            chunk,
            end_stream && chunks.peek().is_none(),
            padding,
        )?;
    }
    Ok(())
}

fn data_frame(
    out: &mut Vec<u8>,
    stream: u32,
    chunk: &[u8],
    end_stream: bool,
    padding: Option<u8>,
) -> Result<(), FrameError> {
    let mut flags = if end_stream { flag::END_STREAM } else { 0 };
    let pad_len = match padding {
        Some(p) => {
            flags |= flag::PADDED;
            1 + usize::from(p)
        }
        None => 0,
    };
    put(out, ty::DATA, flags, stream, chunk.len() + pad_len)?;
    if let Some(p) = padding {
        out.push(p);
    }
    out.extend_from_slice(chunk);
    out.resize(out.len() + padding.map_or(0, usize::from), 0);
    Ok(())
}

/// Appends a HEADERS frame that carries the HPACK `block`, and CONTINUATION
/// frames when the block does not fit in `max` octets.
pub fn headers(
    out: &mut Vec<u8>,
    stream: i64,
    block: &[u8],
    end_stream: bool,
    padding: Option<u8>,
    priority: Option<Priority>,
    max: usize,
) -> Result<(), FrameError> {
    let stream = check_stream("HEADERS", stream, false)?;
    let overhead =
        padding.map_or(0, |p| 1 + usize::from(p)) + if priority.is_some() { 5 } else { 0 };
    let room = max.checked_sub(overhead).filter(|r| *r > 0).ok_or_else(|| {
        invalid(format!(
            "padding and priority take {overhead} octets and leave no room for a header block fragment in a frame of {max} octets"
        ))
    })?;
    let first_len = block.len().min(room);
    let (first, mut rest) = block.split_at(first_len);
    let mut flags = if end_stream { flag::END_STREAM } else { 0 };
    if rest.is_empty() {
        flags |= flag::END_HEADERS;
    }
    if priority.is_some() {
        flags |= flag::PRIORITY;
    }
    if padding.is_some() {
        flags |= flag::PADDED;
    }
    put(out, ty::HEADERS, flags, stream, first.len() + overhead)?;
    if let Some(p) = padding {
        out.push(p);
    }
    if let Some(p) = priority {
        let dep = p.dependency | if p.exclusive { 0x8000_0000 } else { 0 };
        out.extend_from_slice(&dep.to_be_bytes());
        // The wire carries the weight minus one.
        out.push((p.weight.clamp(1, 256) - 1) as u8);
    }
    out.extend_from_slice(first);
    out.resize(out.len() + padding.map_or(0, usize::from), 0);
    while !rest.is_empty() {
        let n = rest.len().min(max);
        let (chunk, tail) = rest.split_at(n);
        rest = tail;
        let flags = if rest.is_empty() {
            flag::END_HEADERS
        } else {
            0
        };
        put(out, ty::CONTINUATION, flags, stream, chunk.len())?;
        out.extend_from_slice(chunk);
    }
    Ok(())
}

/// Appends a RST_STREAM frame.
pub fn rst_stream(out: &mut Vec<u8>, stream: i64, error: u32) -> Result<(), FrameError> {
    let stream = check_stream("RST_STREAM", stream, false)?;
    put(out, ty::RST_STREAM, 0, stream, 4)?;
    out.extend_from_slice(&error.to_be_bytes());
    Ok(())
}

/// Appends a WINDOW_UPDATE frame.
pub fn window_update(out: &mut Vec<u8>, stream: i64, increment: i64) -> Result<(), FrameError> {
    let stream = check_stream("WINDOW_UPDATE", stream, true)?;
    if !(1..=i64::from(MAX_STREAM_ID)).contains(&increment) {
        return Err(invalid(format!(
            "increment of WINDOW_UPDATE must be between 1 and {MAX_STREAM_ID}, not {increment}"
        )));
    }
    put(out, ty::WINDOW_UPDATE, 0, stream, 4)?;
    out.extend_from_slice(&(increment as u32).to_be_bytes());
    Ok(())
}

/// Appends a PING frame, with the ACK flag when `ack` is set.
pub fn ping(out: &mut Vec<u8>, data: [u8; 8], ack: bool) -> Result<(), FrameError> {
    put(out, ty::PING, if ack { flag::ACK } else { 0 }, 0, 8)?;
    out.extend_from_slice(&data);
    Ok(())
}

/// Appends a GOAWAY frame.
pub fn goaway(
    out: &mut Vec<u8>,
    last_stream: i64,
    error: u32,
    debug: &[u8],
) -> Result<(), FrameError> {
    let last = check_stream("GOAWAY last_stream", last_stream, true)?;
    put(out, ty::GOAWAY, 0, 0, 8 + debug.len())?;
    out.extend_from_slice(&last.to_be_bytes());
    out.extend_from_slice(&error.to_be_bytes());
    out.extend_from_slice(debug);
    Ok(())
}

/// The setting identifiers of RFC 9113 section 6.5.2 with their names.
pub const SETTING_NAMES: [(u16, &str); 6] = [
    (0x1, "header_table_size"),
    (0x2, "enable_push"),
    (0x3, "max_concurrent_streams"),
    (0x4, "initial_window_size"),
    (0x5, "max_frame_size"),
    (0x6, "max_header_list_size"),
];

/// Appends a SETTINGS frame with the listed identifier and value pairs.
pub fn settings(out: &mut Vec<u8>, pairs: &[(u16, u32)]) -> Result<(), FrameError> {
    put(out, ty::SETTINGS, 0, 0, pairs.len() * 6)?;
    for (id, value) in pairs {
        out.extend_from_slice(&id.to_be_bytes());
        out.extend_from_slice(&value.to_be_bytes());
    }
    Ok(())
}

/// Appends the empty SETTINGS frame with the ACK flag.
pub fn settings_ack(out: &mut Vec<u8>) -> Result<(), FrameError> {
    put(out, ty::SETTINGS, flag::ACK, 0, 0)
}

/// Reads the pairs of a SETTINGS payload.
pub fn settings_pairs(payload: &[u8]) -> Result<Vec<(u16, u32)>, FrameError> {
    if !payload.len().is_multiple_of(6) {
        return Err(invalid(format!(
            "a SETTINGS payload of {} octets is not a multiple of 6",
            payload.len()
        )));
    }
    let (pairs, _) = payload.as_chunks::<6>();
    Ok(pairs
        .iter()
        .map(|c| {
            (
                u16::from_be_bytes([c[0], c[1]]),
                u32::from_be_bytes([c[2], c[3], c[4], c[5]]),
            )
        })
        .collect())
}

/// Returns the payload of a padded frame without its pad length field and
/// padding, or the payload itself when the PADDED flag is clear.
pub fn strip_padding(flags: u8, payload: &[u8]) -> Result<&[u8], FrameError> {
    if flags & flag::PADDED == 0 {
        return Ok(payload);
    }
    let Some((&pad, body)) = payload.split_first() else {
        return Err(invalid("a PADDED frame has no pad length field"));
    };
    let pad = usize::from(pad);
    if pad > body.len() {
        return Err(invalid(format!(
            "pad length {pad} exceeds the {} octets that follow it",
            body.len()
        )));
    }
    Ok(&body[..body.len() - pad])
}

/// Returns the header block fragment of a HEADERS or CONTINUATION frame of
/// type `frame_type` with `flags`, which is the payload without the padding and
/// the priority fields.
pub fn block_fragment(frame_type: u8, flags: u8, payload: &[u8]) -> Result<&[u8], FrameError> {
    let body = strip_padding(flags, payload)?;
    if frame_type == ty::HEADERS && flags & flag::PRIORITY != 0 {
        return body.get(5..).ok_or_else(|| {
            invalid("a HEADERS frame with PRIORITY has fewer than 5 octets of priority fields")
        });
    }
    Ok(body)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(f: impl FnOnce(&mut Vec<u8>) -> Result<(), FrameError>) -> Vec<u8> {
        let mut out = Vec::new();
        f(&mut out).unwrap();
        out
    }

    #[test]
    fn the_preface_is_the_24_octets_of_the_rfc() {
        assert_eq!(PREFACE.len(), 24);
        assert!(PREFACE.starts_with(b"PRI * HTTP/2.0\r\n"));
    }

    #[test]
    fn header_round_trip() {
        let h = FrameHeader {
            length: 0x01_02_03,
            ty: ty::HEADERS,
            flags: 0x25,
            stream: 0x7FFF_FFFF,
        };
        let mut out = Vec::new();
        h.write(&mut out);
        assert_eq!(out, [1, 2, 3, 1, 0x25, 0x7F, 0xFF, 0xFF, 0xFF]);
        assert_eq!(FrameHeader::parse(&out).unwrap(), h);
        // The reserved bit is dropped on read.
        out[5] |= 0x80;
        assert_eq!(FrameHeader::parse(&out).unwrap(), h);
    }

    #[test]
    fn short_input_is_an_error_and_not_a_panic() {
        for n in 0..HEADER_LEN {
            assert!(FrameHeader::parse(&vec![0; n]).is_err());
        }
        let mut f = one(|o| ping(o, [1; 8], false));
        f.pop();
        assert!(matches!(split(&f), Err(FrameError::Short { .. })));
    }

    #[test]
    fn settings_frames() {
        let f = one(|o| settings(o, &[(3, 100), (4, 65535)]));
        assert_eq!(
            f,
            [0, 0, 12, 4, 0, 0, 0, 0, 0, 0, 3, 0, 0, 0, 100, 0, 4, 0, 0, 255, 255]
        );
        let (frame, rest) = split(&f).unwrap();
        assert!(rest.is_empty());
        assert_eq!(
            settings_pairs(frame.payload).unwrap(),
            [(3, 100), (4, 65535)]
        );
        assert_eq!(
            one(settings_ack),
            [0, 0, 0, 4, 1, 0, 0, 0, 0],
            "RFC 9113 section 6.5.3"
        );
        assert!(settings_pairs(&[0; 5]).is_err());
    }

    #[test]
    fn data_splits_at_the_frame_size_and_marks_the_last_frame() {
        let f = one(|o| data(o, 1, &[7; 25], true, None, 10));
        let (a, rest) = split(&f).unwrap();
        let (b, rest) = split(rest).unwrap();
        let (c, rest) = split(rest).unwrap();
        assert!(rest.is_empty());
        assert_eq!(
            [a.header.length, b.header.length, c.header.length],
            [10, 10, 5]
        );
        assert_eq!(
            [a.header.flags, b.header.flags, c.header.flags],
            [0, 0, flag::END_STREAM]
        );
    }

    #[test]
    fn empty_data_renders_one_frame() {
        let f = one(|o| data(o, 3, &[], true, None, 16384));
        assert_eq!(f, [0, 0, 0, 0, 1, 0, 0, 0, 3]);
    }

    #[test]
    fn padding_adds_zero_octets_and_fits_the_frame_size() {
        let f = one(|o| data(o, 1, b"abcdef", false, Some(3), 8));
        let (a, rest) = split(&f).unwrap();
        let (b, _) = split(rest).unwrap();
        assert_eq!(a.header.flags, flag::PADDED);
        assert_eq!(a.payload, [3, b'a', b'b', b'c', b'd', 0, 0, 0]);
        assert_eq!(b.payload, [3, b'e', b'f', 0, 0, 0]);
        assert_eq!(strip_padding(a.header.flags, a.payload).unwrap(), b"abcd");
        let mut out = Vec::new();
        assert!(data(&mut out, 1, b"x", false, Some(255), 100).is_err());
    }

    #[test]
    fn headers_continue_in_continuation_frames() {
        let block: Vec<u8> = (0..25).collect();
        let f = one(|o| headers(o, 5, &block, true, None, None, 10));
        let (a, rest) = split(&f).unwrap();
        let (b, rest) = split(rest).unwrap();
        let (c, rest) = split(rest).unwrap();
        assert!(rest.is_empty());
        assert_eq!(
            (a.header.ty, a.header.flags),
            (ty::HEADERS, flag::END_STREAM)
        );
        assert_eq!((b.header.ty, b.header.flags), (ty::CONTINUATION, 0));
        assert_eq!(
            (c.header.ty, c.header.flags),
            (ty::CONTINUATION, flag::END_HEADERS)
        );
        let joined: Vec<u8> = [a.payload, b.payload, c.payload].concat();
        assert_eq!(joined, block);
    }

    #[test]
    fn headers_with_priority_and_padding() {
        let f = one(|o| {
            headers(
                o,
                1,
                b"xyz",
                false,
                Some(2),
                Some(Priority {
                    dependency: 7,
                    weight: 256,
                    exclusive: true,
                }),
                100,
            )
        });
        let (a, _) = split(&f).unwrap();
        assert_eq!(
            a.header.flags,
            flag::END_HEADERS | flag::PRIORITY | flag::PADDED
        );
        assert_eq!(a.payload, [2, 0x80, 0, 0, 7, 255, b'x', b'y', b'z', 0, 0]);
        assert_eq!(
            block_fragment(a.header.ty, a.header.flags, a.payload).unwrap(),
            b"xyz"
        );
    }

    #[test]
    fn control_frames_have_the_layouts_of_the_rfc() {
        assert_eq!(
            one(|o| rst_stream(o, 1, 8)),
            [0, 0, 4, 3, 0, 0, 0, 0, 1, 0, 0, 0, 8]
        );
        assert_eq!(
            one(|o| window_update(o, 0, 65535)),
            [0, 0, 4, 8, 0, 0, 0, 0, 0, 0, 0, 255, 255]
        );
        assert_eq!(
            one(|o| ping(o, *b"12345678", true)),
            [0, 0, 8, 6, 1, 0, 0, 0, 0, 49, 50, 51, 52, 53, 54, 55, 56]
        );
        assert_eq!(
            one(|o| goaway(o, 5, 0, b"bye")),
            [0, 0, 11, 7, 0, 0, 0, 0, 0, 0, 0, 0, 5, 0, 0, 0, 0, 98, 121, 101]
        );
    }

    #[test]
    fn values_out_of_range_are_refused() {
        let mut out = Vec::new();
        assert!(window_update(&mut out, 1, 0).is_err());
        assert!(window_update(&mut out, 1, 1 << 31).is_err());
        assert!(rst_stream(&mut out, 0, 0).is_err());
        assert!(data(&mut out, 0, b"", false, None, 10).is_err());
        assert!(headers(&mut out, -1, b"", false, None, None, 10).is_err());
        assert!(raw(&mut out, 0, 0, 1 << 31, b"").is_err());
        assert!(check_padding(256).is_err());
        assert!(check_max_frame_size(0).is_err());
        assert!(out.is_empty(), "a refused frame writes nothing");
    }

    #[test]
    fn stripping_padding_checks_the_pad_length() {
        let h = FrameHeader {
            length: 0,
            ty: ty::DATA,
            flags: flag::PADDED,
            stream: 1,
        };
        assert!(strip_padding(h.flags, &[]).is_err());
        assert!(strip_padding(h.flags, &[5, 1, 2]).is_err());
        assert_eq!(strip_padding(h.flags, &[2, 9, 0, 0]).unwrap(), [9]);
    }

    #[test]
    fn error_names_map_to_codes() {
        assert_eq!(error_code("no_error"), Some(0));
        assert_eq!(error_code("CANCEL"), Some(8));
        assert_eq!(error_code("HTTP_1_1_REQUIRED"), Some(13));
        assert_eq!(error_code("OOPS"), None);
        assert_eq!(type_name(1), "HEADERS");
        assert_eq!(type_name(77), "UNKNOWN_77");
    }
}
