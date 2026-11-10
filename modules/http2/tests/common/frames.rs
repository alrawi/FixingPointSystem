// SPDX-License-Identifier: GPL-3.0-only
//! A small reader for the bytes that one side of a session wrote.
//!
//! It splits a stream into frames with its own code, so the checks on the
//! scenarios do not depend on the frame parser of the crate under test, and it
//! offers the readings that the checks need: the SETTINGS pairs, the GOAWAY
//! fields, the WINDOW_UPDATE increments, and the length of DATA per stream.
#![allow(dead_code)] // The test files use different subsets.

use fps_module_http2::hpack::Decoder;

/// The connection preface that a client starts with.
pub const PREFACE: &[u8] = b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n";

/// One frame with its header fields and payload.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    pub ty: u8,
    pub flags: u8,
    pub stream: u32,
    pub payload: Vec<u8>,
}

pub const DATA: u8 = 0;
pub const HEADERS: u8 = 1;
pub const SETTINGS: u8 = 4;
pub const PUSH_PROMISE: u8 = 5;
pub const PING: u8 = 6;
pub const GOAWAY: u8 = 7;
pub const WINDOW_UPDATE: u8 = 8;

pub const END_STREAM: u8 = 0x1;
pub const ACK: u8 = 0x1;
pub const END_HEADERS: u8 = 0x4;

impl Frame {
    pub fn has(&self, flag: u8) -> bool {
        self.flags & flag != 0
    }

    /// The pairs of a SETTINGS payload.
    pub fn settings(&self) -> Vec<(u16, u32)> {
        assert_eq!(self.ty, SETTINGS);
        assert_eq!(self.payload.len() % 6, 0, "SETTINGS payload length");
        self.payload
            .chunks(6)
            .map(|c| {
                (
                    u16::from_be_bytes([c[0], c[1]]),
                    u32::from_be_bytes([c[2], c[3], c[4], c[5]]),
                )
            })
            .collect()
    }

    /// The increment of a WINDOW_UPDATE frame.
    pub fn increment(&self) -> u32 {
        assert_eq!(self.ty, WINDOW_UPDATE);
        assert_eq!(self.payload.len(), 4, "WINDOW_UPDATE payload length");
        u32::from_be_bytes([
            self.payload[0],
            self.payload[1],
            self.payload[2],
            self.payload[3],
        ]) & 0x7FFF_FFFF
    }

    /// The last stream id and the error code of a GOAWAY frame.
    pub fn goaway(&self) -> (u32, u32) {
        assert_eq!(self.ty, GOAWAY);
        assert!(self.payload.len() >= 8, "GOAWAY payload length");
        let p = &self.payload;
        (
            u32::from_be_bytes([p[0], p[1], p[2], p[3]]) & 0x7FFF_FFFF,
            u32::from_be_bytes([p[4], p[5], p[6], p[7]]),
        )
    }
}

/// Splits `bytes` into frames, and fails when a frame is cut off.
pub fn split(mut bytes: &[u8]) -> Vec<Frame> {
    let mut out = Vec::new();
    while !bytes.is_empty() {
        assert!(
            bytes.len() >= 9,
            "{} octets cannot hold a frame header",
            bytes.len()
        );
        let length = u32::from_be_bytes([0, bytes[0], bytes[1], bytes[2]]) as usize;
        assert!(bytes.len() >= 9 + length, "a frame is cut off");
        out.push(Frame {
            ty: bytes[3],
            flags: bytes[4],
            stream: u32::from_be_bytes([bytes[5], bytes[6], bytes[7], bytes[8]]) & 0x7FFF_FFFF,
            payload: bytes[9..9 + length].to_vec(),
        });
        bytes = &bytes[9 + length..];
    }
    out
}

/// Splits what a client wrote, which starts with the connection preface.
pub fn split_client(bytes: &[u8]) -> Vec<Frame> {
    assert!(
        bytes.starts_with(PREFACE),
        "the client starts with the preface"
    );
    split(&bytes[PREFACE.len()..])
}

/// Decodes the header blocks of the HEADERS frames of one direction, in
/// order, with one HPACK decoder, as a peer does. Each entry is the stream id
/// and the field list. Every block here fits one frame with END_HEADERS.
pub fn header_lists(frames: &[Frame]) -> Vec<(u32, Vec<(String, String)>)> {
    let mut decoder = Decoder::new();
    let mut out = Vec::new();
    for f in frames.iter().filter(|f| f.ty == HEADERS) {
        assert!(
            f.has(END_HEADERS),
            "a block of these scenarios fits one frame"
        );
        // No padding and no priority in these scenarios.
        assert_eq!(f.flags & 0x28, 0, "padded or prioritized HEADERS");
        let fields = decoder.decode(&f.payload).expect("the block decodes");
        out.push((
            f.stream,
            fields
                .into_iter()
                .map(|h| {
                    (
                        String::from_utf8(h.name).expect("name"),
                        String::from_utf8(h.value).expect("value"),
                    )
                })
                .collect(),
        ));
    }
    out
}

/// The value of the first field called `name`.
pub fn field<'a>(list: &'a [(String, String)], name: &str) -> Option<&'a str> {
    list.iter()
        .find(|(n, _)| n == name)
        .map(|(_, v)| v.as_str())
}

/// Writes the frames back to bytes, without a preface.
pub fn join(frames: &[Frame]) -> Vec<u8> {
    let mut out = Vec::new();
    for f in frames {
        let length = f.payload.len() as u32;
        out.extend_from_slice(&length.to_be_bytes()[1..]);
        out.push(f.ty);
        out.push(f.flags);
        out.extend_from_slice(&f.stream.to_be_bytes());
        out.extend_from_slice(&f.payload);
    }
    out
}

/// Drops the PING frames with ACK. A replay into a peer that never sent the
/// PING would otherwise carry an unsolicited acknowledgment, which RFC 9113
/// does not forbid but the `h2` crate treats as a protocol error.
pub fn without_ping_acks(frames: &[Frame]) -> Vec<Frame> {
    frames
        .iter()
        .filter(|f| !(f.ty == PING && f.has(ACK)))
        .cloned()
        .collect()
}
