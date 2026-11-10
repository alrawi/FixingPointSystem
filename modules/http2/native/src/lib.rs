// SPDX-License-Identifier: GPL-3.0-only
//! Native template functions of the `http2` module.
//!
//! Decision D10 lets first party modules implement their template functions in
//! Rust. This crate implements the `http2` functions behind the [`TemplateFn`]
//! trait of `fps-flow`, so the interpreter renders
//! `{{ http2.headers(1, vars.request, end_stream=true) }}` without a
//! WebAssembly runtime. The functions write the frames of RFC 9113 with the
//! header compression of RFC 7541 and parse the frames that the peer sent.
//! Every function is binary safe: arguments and results are octets, and a DATA
//! frame carries any bytes that a nested call such as `content.binary` returns.
//!
//! # State
//!
//! HPACK keeps a dynamic table on each side of a connection, so unlike the
//! functions of the other modules the [`Http2`] value has state. The agent
//! gives every flow side its own copy, which is the connection state of that
//! side: `http2.headers` encodes with the table of the headers that this side
//! sent before, and `http2.headers_decode` decodes with the table of the
//! headers that it received before. Both depend on the order of the calls, so
//! a scenario sends and decodes each header block exactly once, in order.
//!
//! # Reading frames in a flow
//!
//! A flow cannot branch on what it reads, but it can read a frame in two
//! `expect` actions, because the 9 octet frame header announces the payload
//! length. After the first `expect`, `last` holds the header, and the decoders
//! `frame_length`, `frame_stream`, and `frame_field` read it and give decimal
//! digits, which a `set` action binds as integers. After the second `expect`,
//! `last` holds the payload, which goes to the payload decoders
//! `headers_decode`, `data_payload`, `settings_value`, and `ping_ack` together
//! with the `flags` that the header announced. A server reads the connection
//! preface and then a HEADERS frame like this.
//!
//! ```yaml
//! - expect: { bytes: 24 }                                      # the preface
//! - expect: { bytes: 9 }                                       # frame header
//! - set:
//!     length: "{{ http2.frame_length(last) }}"
//!     stream: "{{ http2.frame_stream(last) }}"
//!     flags: "{{ http2.frame_field(last, 'flags') }}"
//! - expect: { bytes: "{{ vars.length }}" }                     # the payload
//! - set: { request: "{{ http2.headers_decode(last, flags=vars.flags) }}" }
//! - set: { path: "{{ http2.header_value(vars.request, ':path') }}" }
//! - send: "{{ http2.headers(vars.stream, vars.response, end_stream=true) }}"
//! ```
//!
//! A `set` action keeps text, and its value goes through UTF-8, so a frame
//! must never travel through a variable. The decoders therefore take `last`
//! directly, and the one state that crosses frames, a header block that
//! continues in CONTINUATION frames, lives in the [`Http2`] value:
//! `headers_decode` collects the fragments of a block and renders its header
//! list when a frame has END_HEADERS. A scenario knows how many frames to read,
//! because both of its sides come from one file and the sizes of the headers
//! and of the data are variables that both sides share.
//!
//! [`FUNCTIONS`] is the catalog that `modules/http2/manifest.yaml` lists under
//! `functions`. The catalog test compares the two, so the manifest stays the
//! single catalog that the validator and the controller read.
#![forbid(unsafe_code)]
#![warn(missing_docs, clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod args;
pub mod catalog;
pub mod frame;
pub mod hpack;
pub mod huffman;
mod huffman_table;
#[cfg(test)]
mod render_tests;
mod static_table;

use fps_flow::{FnCall, FnError, FnErrorKind, TemplateFn};
use rand::RngCore;

pub use catalog::{DefaultValue, FunctionSpec, ParamSpec, FUNCTIONS};

use frame::{flag, ty, FrameError};
use hpack::{HuffmanMode, IndexMode};

/// Name of the module that this crate implements.
pub const MODULE_NAME: &str = "http2";

/// The template functions of the `http2` module for one session side.
///
/// A value holds the HPACK tables of one side of one connection. Clone a fresh
/// value for every session, and never share one between two connections.
#[derive(Clone, Debug, Default)]
pub struct Http2 {
    encoder: hpack::Encoder,
    decoder: hpack::Decoder,
    /// The fragments of a header block that waits for its END_HEADERS frame.
    pending: Option<Vec<u8>>,
}

fn invalid(message: impl Into<String>) -> FnError {
    FnError::new(FnErrorKind::InvalidArgument, message)
}

fn protocol(message: impl Into<String>) -> FnError {
    FnError::new(FnErrorKind::Protocol, message)
}

/// Maps a frame error that comes from the arguments of a call.
fn bad_arg(function: &str) -> impl Fn(FrameError) -> FnError + '_ {
    move |e| invalid(format!("'{function}': {e}"))
}

/// Maps a frame error that comes from bytes of the peer.
fn bad_frame(function: &str) -> impl Fn(FrameError) -> FnError + '_ {
    move |e| protocol(format!("'{function}': {e}"))
}

fn push(out: &mut Vec<u8>, text: &str) {
    out.extend_from_slice(text.as_bytes());
}

/// Reads an optional setting: -1 leaves it out, and any other value must lie
/// in `lo..=hi`.
fn setting(a: &args::Args<'_>, name: &str, lo: i64, hi: i64) -> Result<Option<u32>, FnError> {
    let v = a.int(name)?;
    if v == -1 {
        return Ok(None);
    }
    if !(lo..=hi).contains(&v) {
        return Err(invalid(format!(
            "'{name}' of 'settings' must be -1 or between {lo} and {hi}, not {v}"
        )));
    }
    Ok(Some(v as u32))
}

/// Reads an error code argument, a number or a name such as `CANCEL`.
fn error_arg(a: &args::Args<'_>, function: &str) -> Result<u32, FnError> {
    let text = a.any_text("error")?;
    if let Ok(n) = text.parse::<u32>() {
        return Ok(n);
    }
    frame::error_code(&text).ok_or_else(|| {
        invalid(format!(
            "'error' of '{function}' is not a code or a name: '{text}'"
        ))
    })
}

fn huffman_mode(a: &args::Args<'_>) -> Result<HuffmanMode, FnError> {
    match a.text("huffman")? {
        "auto" => Ok(HuffmanMode::Auto),
        "always" => Ok(HuffmanMode::Always),
        "never" => Ok(HuffmanMode::Never),
        other => Err(invalid(format!(
            "'huffman' of 'headers' must be auto, always, or never, not '{other}'"
        ))),
    }
}

fn index_mode(a: &args::Args<'_>) -> Result<IndexMode, FnError> {
    match a.text("index")? {
        "auto" => Ok(IndexMode::Auto),
        "off" => Ok(IndexMode::Off),
        "all" => Ok(IndexMode::All),
        other => Err(invalid(format!(
            "'index' of 'headers' must be auto, off, or all, not '{other}'"
        ))),
    }
}

/// One header: name and value.
type Header<'a> = (&'a [u8], &'a [u8]);

/// Splits a header list into fields. Each line is `name: value`, a name that
/// starts with a colon is a pseudo header, and blank lines are skipped.
fn parse_list(list: &[u8]) -> Result<Vec<Header<'_>>, FnError> {
    let mut fields = Vec::new();
    for (n, raw) in list.split(|b| *b == b'\n').enumerate() {
        let line = raw.strip_suffix(b"\r").unwrap_or(raw);
        if line.is_empty() {
            continue;
        }
        let n = n + 1;
        // A pseudo header name starts with a colon that is part of the name.
        let skip = usize::from(line[0] == b':');
        let Some(colon) = line[skip..]
            .iter()
            .position(|b| *b == b':')
            .map(|p| p + skip)
        else {
            return Err(invalid(format!("line {n} of 'headers' has no colon")));
        };
        let name = &line[..colon];
        let value = trim(&line[colon + 1..]);
        if name.len() == skip
            || !name[skip..]
                .iter()
                .all(|b| b.is_ascii_graphic() && !b.is_ascii_uppercase() && *b != b':')
        {
            return Err(invalid(format!(
                "line {n} of 'headers' has a name that is empty, has an uppercase letter, or has a character that is not allowed"
            )));
        }
        if value.iter().any(|b| matches!(b, b'\r' | b'\0')) {
            return Err(invalid(format!(
                "line {n} of 'headers' has a value with a CR or a NUL byte"
            )));
        }
        fields.push((name, value));
    }
    Ok(fields)
}

/// Strips spaces and tabs at both ends of a field value.
fn trim(value: &[u8]) -> &[u8] {
    let is_ows = |b: &u8| *b == b' ' || *b == b'\t';
    let start = value.iter().position(|b| !is_ows(b)).unwrap_or(value.len());
    let end = value
        .iter()
        .rposition(|b| !is_ows(b))
        .map_or(start, |p| p + 1);
    &value[start..end]
}

fn decimal(out: &mut Vec<u8>, n: impl std::fmt::Display) {
    push(out, &n.to_string());
}

/// Reads an argument that holds frame flags or a frame type, 0 to 255.
fn octet(a: &args::Args<'_>, function: &str, name: &str) -> Result<u8, FnError> {
    let v = a.int(name)?;
    u8::try_from(v).map_err(|_| {
        invalid(format!(
            "'{name}' of '{function}' must be between 0 and 255, not {v}"
        ))
    })
}

/// The initial value of a setting before the peer changes it, and -1 for
/// settings without a limit.
fn initial_setting(id: u16) -> i64 {
    match id {
        0x1 => 4096,
        0x2 => 1,
        0x4 => 65_535,
        0x5 => 16_384,
        _ => -1,
    }
}

impl Http2 {
    fn headers(&mut self, a: &args::Args<'_>, out: &mut Vec<u8>) -> Result<(), FnError> {
        let err = bad_arg("headers");
        let max = frame::check_max_frame_size(a.int("max_frame_size")?).map_err(&err)?;
        let padding = frame::check_padding(a.int("padding")?).map_err(&err)?;
        let priority = match a.int("priority_dependency")? {
            -1 => None,
            dep @ 0..=0x7FFF_FFFF => {
                let weight = a.int("priority_weight")?;
                if !(1..=256).contains(&weight) {
                    return Err(invalid(format!(
                        "'priority_weight' of 'headers' must be between 1 and 256, not {weight}"
                    )));
                }
                Some(frame::Priority {
                    dependency: dep as u32,
                    weight: weight as u16,
                    exclusive: a.boolean("priority_exclusive")?,
                })
            }
            other => {
                return Err(invalid(format!(
                    "'priority_dependency' of 'headers' must be -1 or a stream id, not {other}"
                )))
            }
        };
        let (huffman, index) = (huffman_mode(a)?, index_mode(a)?);
        let table_size = a.int("table_size")?;
        if table_size < -1 || table_size > hpack::MAX_TABLE_SIZE as i64 {
            return Err(invalid(format!(
                "'table_size' of 'headers' must be -1 or between 0 and {}, not {table_size}",
                hpack::MAX_TABLE_SIZE
            )));
        }
        let fields = parse_list(a.bytes("headers")?)?;
        let stream = a.int("stream")?;
        // Check the frame before the encoder changes its table, so a refused
        // call leaves the state as it was.
        frame::headers(&mut Vec::new(), stream, &[], false, padding, priority, max)
            .map_err(&err)?;
        let mut block = Vec::new();
        if table_size >= 0 {
            self.encoder.set_max_size(table_size as usize, &mut block);
        }
        for (name, value) in fields {
            self.encoder.encode(name, value, huffman, index, &mut block);
        }
        frame::headers(
            out,
            stream,
            &block,
            a.boolean("end_stream")?,
            padding,
            priority,
            max,
        )
        .map_err(&err)
    }

    /// Takes the payload of a HEADERS or CONTINUATION frame, and renders the
    /// header list of the block when the frame ends it.
    fn headers_decode(
        &mut self,
        payload: &[u8],
        flags: u8,
        frame_type: u8,
        out: &mut Vec<u8>,
    ) -> Result<(), FnError> {
        let fragment = frame::block_fragment(frame_type, flags, payload)
            .map_err(bad_frame("headers_decode"))?;
        let mut block = match (frame_type, self.pending.take()) {
            (ty::HEADERS, None) => Vec::new(),
            (ty::CONTINUATION, Some(open)) => open,
            (ty::HEADERS, Some(_)) => return Err(protocol(
                "'headers_decode' got a HEADERS frame while a header block waits for END_HEADERS",
            )),
            _ => {
                return Err(protocol(
                    "'headers_decode' got a CONTINUATION frame without an open header block",
                ))
            }
        };
        if block.len() + fragment.len() > hpack::MAX_LIST_SIZE {
            return Err(protocol("'headers_decode': the header block is too large"));
        }
        block.extend_from_slice(fragment);
        if flags & flag::END_HEADERS == 0 {
            self.pending = Some(block);
            return Ok(());
        }
        let fields = self
            .decoder
            .decode(&block)
            .map_err(|e| protocol(format!("'headers_decode': {e}")))?;
        for f in fields {
            if f.value.iter().any(|b| matches!(b, b'\r' | b'\n' | b'\0')) {
                return Err(protocol(
                    "'headers_decode': a header value has a line break or a NUL byte",
                ));
            }
            out.extend_from_slice(&f.name);
            out.extend_from_slice(b": ");
            out.extend_from_slice(&f.value);
            out.push(b'\n');
        }
        Ok(())
    }
}

fn header_value(list: &[u8], name: &str, required: bool, out: &mut Vec<u8>) -> Result<(), FnError> {
    for line in list.split(|b| *b == b'\n') {
        let Some(colon) = line.iter().skip(1).position(|b| *b == b':').map(|p| p + 1) else {
            continue;
        };
        if line[..colon].eq_ignore_ascii_case(name.as_bytes()) {
            out.extend_from_slice(trim(&line[colon + 1..]));
            return Ok(());
        }
    }
    if required {
        return Err(protocol(format!(
            "'header_value': the header list has no field '{name}'"
        )));
    }
    Ok(())
}

fn frame_field(bytes: &[u8], field: &str, out: &mut Vec<u8>) -> Result<(), FnError> {
    let h = frame::FrameHeader::parse(bytes).map_err(bad_frame("frame_field"))?;
    match field {
        "length" => decimal(out, h.length),
        "stream" => decimal(out, h.stream),
        "flags" => decimal(out, h.flags),
        "type_code" => decimal(out, h.ty),
        "type" => push(out, &frame::type_name(h.ty)),
        "end_stream" => push(out, bool_text(h.has(flag::END_STREAM))),
        "end_headers" => push(out, bool_text(h.has(flag::END_HEADERS))),
        "ack" => push(out, bool_text(h.has(flag::ACK))),
        "padded" => push(out, bool_text(h.has(flag::PADDED))),
        "priority" => push(out, bool_text(h.has(flag::PRIORITY))),
        other => {
            return Err(invalid(format!(
                "'field' of 'frame_field' must be length, stream, flags, type_code, type, end_stream, end_headers, ack, padded, or priority, not '{other}'"
            )))
        }
    }
    Ok(())
}

impl TemplateFn for Http2 {
    fn render(
        &mut self,
        call: &FnCall<'_>,
        rng: &mut dyn RngCore,
        out: &mut Vec<u8>,
    ) -> Result<(), FnError> {
        if call.module != MODULE_NAME {
            return Err(FnError::new(
                FnErrorKind::Unsupported,
                format!("module '{}' is not http2", call.module),
            ));
        }
        match call.function {
            "preface" => {
                args::bind(&catalog::PREFACE, &call.args)?;
                out.extend_from_slice(frame::PREFACE);
            }
            "settings" => {
                let a = args::bind(&catalog::SETTINGS, &call.args)?;
                let values = [
                    (
                        0x1,
                        setting(&a, "header_table_size", 0, i64::from(u32::MAX))?,
                    ),
                    (0x2, setting(&a, "enable_push", 0, 1)?),
                    (
                        0x3,
                        setting(&a, "max_concurrent_streams", 0, i64::from(u32::MAX))?,
                    ),
                    (0x4, setting(&a, "initial_window_size", 0, 0x7FFF_FFFF)?),
                    (0x5, setting(&a, "max_frame_size", 16_384, 0xFF_FFFF)?),
                    (
                        0x6,
                        setting(&a, "max_header_list_size", 0, i64::from(u32::MAX))?,
                    ),
                ];
                let pairs: Vec<(u16, u32)> = values
                    .iter()
                    .filter_map(|(id, v)| v.map(|v| (*id, v)))
                    .collect();
                if let Some(size) = values[0].1 {
                    self.decoder.set_allowed_size(size as usize);
                }
                frame::settings(out, &pairs).map_err(bad_arg("settings"))?;
            }
            "settings_ack" => {
                args::bind(&catalog::SETTINGS_ACK, &call.args)?;
                frame::settings_ack(out).map_err(bad_arg("settings_ack"))?;
            }
            "headers" => {
                let a = args::bind(&catalog::HEADERS, &call.args)?;
                self.headers(&a, out)?;
            }
            "data" => {
                let a = args::bind(&catalog::DATA, &call.args)?;
                let err = bad_arg("data");
                frame::data(
                    out,
                    a.int("stream")?,
                    a.bytes("data")?,
                    a.boolean("end_stream")?,
                    frame::check_padding(a.int("padding")?).map_err(&err)?,
                    frame::check_max_frame_size(a.int("max_frame_size")?).map_err(&err)?,
                )
                .map_err(&err)?;
            }
            "window_update" => {
                let a = args::bind(&catalog::WINDOW_UPDATE, &call.args)?;
                frame::window_update(out, a.int("stream")?, a.int("increment")?)
                    .map_err(bad_arg("window_update"))?;
            }
            "ping" => {
                let a = args::bind(&catalog::PING, &call.args)?;
                let given = a.bytes("data")?;
                let data = if given.is_empty() {
                    rng.next_u64().to_be_bytes()
                } else {
                    <[u8; 8]>::try_from(given).map_err(|_| {
                        invalid(format!(
                            "'data' of 'ping' must be 8 octets, not {}",
                            given.len()
                        ))
                    })?
                };
                frame::ping(out, data, false).map_err(bad_arg("ping"))?;
            }
            "ping_ack" => {
                let a = args::bind(&catalog::PING_ACK, &call.args)?;
                let payload = a.bytes("payload")?;
                let data = <[u8; 8]>::try_from(payload).map_err(|_| {
                    protocol(format!(
                        "'ping_ack': a PING payload has 8 octets, not {}",
                        payload.len()
                    ))
                })?;
                frame::ping(out, data, true).map_err(bad_arg("ping_ack"))?;
            }
            "goaway" => {
                let a = args::bind(&catalog::GOAWAY, &call.args)?;
                frame::goaway(
                    out,
                    a.int("last_stream")?,
                    error_arg(&a, "goaway")?,
                    a.bytes("debug")?,
                )
                .map_err(bad_arg("goaway"))?;
            }
            "rst_stream" => {
                let a = args::bind(&catalog::RST_STREAM, &call.args)?;
                frame::rst_stream(out, a.int("stream")?, error_arg(&a, "rst_stream")?)
                    .map_err(bad_arg("rst_stream"))?;
            }
            "frame" => {
                let a = args::bind(&catalog::FRAME, &call.args)?;
                let byte = |name: &str| -> Result<u8, FnError> {
                    let v = a.int(name)?;
                    u8::try_from(v).map_err(|_| {
                        invalid(format!(
                            "'{name}' of 'frame' must be between 0 and 255, not {v}"
                        ))
                    })
                };
                frame::raw(
                    out,
                    byte("type")?,
                    byte("flags")?,
                    a.int("stream")?,
                    a.bytes("payload")?,
                )
                .map_err(bad_arg("frame"))?;
            }
            "frame_length" => {
                let a = args::bind(&catalog::FRAME_LENGTH, &call.args)?;
                let h = frame::FrameHeader::parse(a.bytes("header")?)
                    .map_err(bad_frame("frame_length"))?;
                decimal(out, h.length);
            }
            "frame_stream" => {
                let a = args::bind(&catalog::FRAME_STREAM, &call.args)?;
                let h = frame::FrameHeader::parse(a.bytes("header")?)
                    .map_err(bad_frame("frame_stream"))?;
                decimal(out, h.stream);
            }
            "frame_field" => {
                let a = args::bind(&catalog::FRAME_FIELD, &call.args)?;
                frame_field(a.bytes("header")?, a.text("field")?, out)?;
            }
            "data_payload" => {
                let a = args::bind(&catalog::DATA_PAYLOAD, &call.args)?;
                let flags = octet(&a, "data_payload", "flags")?;
                out.extend_from_slice(
                    frame::strip_padding(flags, a.bytes("payload")?)
                        .map_err(bad_frame("data_payload"))?,
                );
            }
            "headers_decode" => {
                let a = args::bind(&catalog::HEADERS_DECODE, &call.args)?;
                let frame_type = octet(&a, "headers_decode", "type")?;
                let flags = octet(&a, "headers_decode", "flags")?;
                self.headers_decode(a.bytes("payload")?, flags, frame_type, out)?;
            }
            "header_value" => {
                let a = args::bind(&catalog::HEADER_VALUE, &call.args)?;
                header_value(
                    a.bytes("headers")?,
                    a.text("name")?,
                    a.boolean("required")?,
                    out,
                )?;
            }
            "settings_value" => {
                let a = args::bind(&catalog::SETTINGS_VALUE, &call.args)?;
                let name = a.text("name")?;
                let Some((id, _)) = frame::SETTING_NAMES.iter().find(|(_, n)| *n == name) else {
                    return Err(invalid(format!(
                        "'name' of 'settings_value' is not a setting name: '{name}'"
                    )));
                };
                let pairs = frame::settings_pairs(a.bytes("payload")?)
                    .map_err(bad_frame("settings_value"))?;
                // A later value of the same identifier replaces an earlier one.
                match pairs.iter().rev().find(|(i, _)| i == id) {
                    Some((_, v)) => decimal(out, v),
                    None => decimal(out, initial_setting(*id)),
                }
            }
            "data_frames" => {
                let a = args::bind(&catalog::DATA_FRAMES, &call.args)?;
                let err = bad_arg("data_frames");
                let size = a.int("size")?;
                if !(0..=0xFFFF_FFFF).contains(&size) {
                    return Err(invalid(format!(
                        "'size' of 'data_frames' must be 0 or more, not {size}"
                    )));
                }
                let max = frame::check_max_frame_size(a.int("max_frame_size")?).map_err(&err)?;
                let padding = frame::check_padding(a.int("padding")?).map_err(&err)?;
                let overhead = padding.map_or(0, |p| 1 + usize::from(p));
                let room = max
                    .checked_sub(overhead)
                    .filter(|r| *r > 0)
                    .ok_or_else(|| invalid("'padding' of 'data_frames' leaves no room for data"))?;
                decimal(out, (size as usize).div_ceil(room).max(1));
            }
            "headers_frames" => {
                let a = args::bind(&catalog::HEADERS_FRAMES, &call.args)?;
                let err = bad_arg("headers_frames");
                let max = frame::check_max_frame_size(a.int("max_frame_size")?).map_err(&err)?;
                let padding = frame::check_padding(a.int("padding")?).map_err(&err)?;
                let priority = a.int("priority_dependency")? >= 0;
                let (huffman, index) = (huffman_mode(&a)?, index_mode(&a)?);
                let mut block = Vec::new();
                let mut fresh = hpack::Encoder::new();
                for (name, value) in parse_list(a.bytes("headers")?)? {
                    fresh.encode(name, value, huffman, index, &mut block);
                }
                let overhead =
                    padding.map_or(0, |p| 1 + usize::from(p)) + 5 * usize::from(priority);
                let room = max
                    .checked_sub(overhead)
                    .filter(|r| *r > 0)
                    .ok_or_else(|| {
                        invalid(
                            "padding and priority of 'headers_frames' leave no room for a fragment",
                        )
                    })?;
                let rest = block.len().saturating_sub(room);
                decimal(out, 1 + rest.div_ceil(max));
            }
            other => {
                return Err(FnError::new(
                    FnErrorKind::Unsupported,
                    format!("http2 has no function '{other}'"),
                ))
            }
        }
        Ok(())
    }
}

fn bool_text(b: bool) -> &'static str {
    if b {
        "true"
    } else {
        "false"
    }
}
