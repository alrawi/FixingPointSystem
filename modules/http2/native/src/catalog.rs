// SPDX-License-Identifier: GPL-3.0-only
//! The template function catalog of the `http2` module.
//!
//! The entries are the `functions` list that `modules/http2/manifest.yaml`
//! carries, name for name, parameter for parameter, in the same order and with
//! the same types and defaults, as the contract section "Template function
//! catalog" requires. MOD-06 adds the manifest, and `cargo run -p
//! fps-module-http2 --example print_catalog` prints the list in manifest
//! syntax.

use fps_flow::ValueType;

/// The default of an optional parameter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DefaultValue {
    /// A text default.
    Text(&'static str),
    /// An integer default.
    Int(i64),
    /// A boolean default.
    Boolean(bool),
    /// A bytes default. The manifest writes it as hexadecimal digit pairs.
    Bytes(&'static [u8]),
}

/// One parameter of a template function.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ParamSpec {
    /// Parameter name.
    pub name: &'static str,
    /// Declared type.
    pub ty: ValueType,
    /// The default, or `None` for a required parameter.
    pub default: Option<DefaultValue>,
}

/// One template function.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FunctionSpec {
    /// Function name without the module prefix.
    pub name: &'static str,
    /// What the function renders, in one sentence, for the manifest.
    pub summary: &'static str,
    /// Parameters in positional order.
    pub params: &'static [ParamSpec],
}

const fn req(name: &'static str, ty: ValueType) -> ParamSpec {
    ParamSpec {
        name,
        ty,
        default: None,
    }
}

const fn opt(name: &'static str, ty: ValueType, default: DefaultValue) -> ParamSpec {
    ParamSpec {
        name,
        ty,
        default: Some(default),
    }
}

/// The largest frame payload that a peer accepts until it announces
/// `SETTINGS_MAX_FRAME_SIZE` (RFC 9113 section 4.2), and the default of every
/// `max_frame_size` parameter.
pub const DEFAULT_MAX_FRAME_SIZE: i64 = 16_384;

/// The largest dynamic table that the module keeps, in bytes. A peer may
/// announce up to 2^32 - 1, and the module refuses to hold more than 1 MiB.
pub const MAX_TABLE_SIZE: i64 = 1 << 20;

/// The dynamic table size of RFC 7541 section 4.2 before any SETTINGS frame.
pub const DEFAULT_TABLE_SIZE: usize = 4096;

/// `http2.preface`, the 24 octets that start every connection of the client.
pub const PREFACE: FunctionSpec = FunctionSpec {
    name: "preface",
    summary: "The client connection preface of RFC 9113 section 3.4, 24 octets that start with PRI * HTTP/2.0. The SETTINGS frame follows it.",
    params: &[],
};

/// `http2.settings`, a SETTINGS frame.
pub const SETTINGS: FunctionSpec = FunctionSpec {
    name: "settings",
    summary: "A SETTINGS frame with the listed parameters in identifier order. A value of -1 leaves its parameter out, and a call without values renders an empty SETTINGS frame. A header_table_size also sets the largest dynamic table that this side accepts from its peer.",
    params: &[
        opt("header_table_size", ValueType::Int, DefaultValue::Int(-1)),
        opt("enable_push", ValueType::Int, DefaultValue::Int(-1)),
        opt(
            "max_concurrent_streams",
            ValueType::Int,
            DefaultValue::Int(-1),
        ),
        opt("initial_window_size", ValueType::Int, DefaultValue::Int(-1)),
        opt("max_frame_size", ValueType::Int, DefaultValue::Int(-1)),
        opt(
            "max_header_list_size",
            ValueType::Int,
            DefaultValue::Int(-1),
        ),
    ],
};

/// `http2.settings_ack`, the acknowledgment of a SETTINGS frame.
pub const SETTINGS_ACK: FunctionSpec = FunctionSpec {
    name: "settings_ack",
    summary: "An empty SETTINGS frame with the ACK flag, which acknowledges the SETTINGS frame of the peer.",
    params: &[],
};

/// `http2.headers`, a HEADERS frame and the CONTINUATION frames after it.
pub const HEADERS: FunctionSpec = FunctionSpec {
    name: "headers",
    summary: "A HEADERS frame with the header list HPACK encoded, followed by CONTINUATION frames when the block exceeds max_frame_size. The list holds one 'name: value' per line, and names are lowercase. The dynamic table persists across calls of the same session side.",
    params: &[
        req("stream", ValueType::Int),
        req("headers", ValueType::Bytes),
        opt("end_stream", ValueType::Boolean, DefaultValue::Boolean(false)),
        opt("padding", ValueType::Int, DefaultValue::Int(-1)),
        opt(
            "priority_dependency",
            ValueType::Int,
            DefaultValue::Int(-1),
        ),
        opt("priority_weight", ValueType::Int, DefaultValue::Int(16)),
        opt(
            "priority_exclusive",
            ValueType::Boolean,
            DefaultValue::Boolean(false),
        ),
        opt(
            "max_frame_size",
            ValueType::Int,
            DefaultValue::Int(DEFAULT_MAX_FRAME_SIZE),
        ),
        opt("huffman", ValueType::Text, DefaultValue::Text("auto")),
        opt("index", ValueType::Text, DefaultValue::Text("auto")),
        opt("table_size", ValueType::Int, DefaultValue::Int(-1)),
    ],
};

/// `http2.data`, one or more DATA frames.
pub const DATA: FunctionSpec = FunctionSpec {
    name: "data",
    summary: "The data as DATA frames of at most max_frame_size octets each, with END_STREAM on the last one when end_stream is set. A padding of 0 or more adds that many zero octets to every frame, and -1 sends no padding.",
    params: &[
        req("stream", ValueType::Int),
        req("data", ValueType::Bytes),
        opt("end_stream", ValueType::Boolean, DefaultValue::Boolean(false)),
        opt("padding", ValueType::Int, DefaultValue::Int(-1)),
        opt(
            "max_frame_size",
            ValueType::Int,
            DefaultValue::Int(DEFAULT_MAX_FRAME_SIZE),
        ),
    ],
};

/// `http2.window_update`, a WINDOW_UPDATE frame.
pub const WINDOW_UPDATE: FunctionSpec = FunctionSpec {
    name: "window_update",
    summary: "A WINDOW_UPDATE frame that grants increment octets on a stream, or on the connection when stream is 0.",
    params: &[
        req("stream", ValueType::Int),
        req("increment", ValueType::Int),
    ],
};

/// `http2.ping`, a PING frame.
pub const PING: FunctionSpec = FunctionSpec {
    name: "ping",
    summary: "A PING frame with 8 opaque octets. Without data the octets come from the session generator.",
    params: &[opt("data", ValueType::Bytes, DefaultValue::Bytes(&[]))],
};

/// `http2.ping_ack`, the answer to a PING frame.
pub const PING_ACK: FunctionSpec = FunctionSpec {
    name: "ping_ack",
    summary: "A PING frame with the ACK flag and the opaque octets of the PING frame in frame, which holds the whole frame including its 9 octet header.",
    params: &[req("frame", ValueType::Bytes)],
};

/// `http2.goaway`, a GOAWAY frame.
pub const GOAWAY: FunctionSpec = FunctionSpec {
    name: "goaway",
    summary: "A GOAWAY frame with the last stream id, an error code (a number or a name such as NO_ERROR), and optional debug data.",
    params: &[
        opt("last_stream", ValueType::Int, DefaultValue::Int(0)),
        opt("error", ValueType::Any, DefaultValue::Int(0)),
        opt("debug", ValueType::Bytes, DefaultValue::Bytes(&[])),
    ],
};

/// `http2.rst_stream`, a RST_STREAM frame.
pub const RST_STREAM: FunctionSpec = FunctionSpec {
    name: "rst_stream",
    summary: "A RST_STREAM frame with an error code, a number or a name such as CANCEL.",
    params: &[
        req("stream", ValueType::Int),
        opt("error", ValueType::Any, DefaultValue::Int(0)),
    ],
};

/// `http2.frame`, a frame of any type.
pub const FRAME: FunctionSpec = FunctionSpec {
    name: "frame",
    summary: "A frame of any type with the given flags and payload, for frame types without a function of their own and for deliberately odd frames.",
    params: &[
        req("type", ValueType::Int),
        req("stream", ValueType::Int),
        opt("payload", ValueType::Bytes, DefaultValue::Bytes(&[])),
        opt("flags", ValueType::Int, DefaultValue::Int(0)),
    ],
};

/// `http2.frame_length`, the payload length that a frame header announces.
pub const FRAME_LENGTH: FunctionSpec = FunctionSpec {
    name: "frame_length",
    summary: "The payload length in a 9 octet frame header as decimal digits, for an expect action with bytes that reads the payload after the header.",
    params: &[req("header", ValueType::Bytes)],
};

/// `http2.frame_stream`, the stream id of a frame header.
pub const FRAME_STREAM: FunctionSpec = FunctionSpec {
    name: "frame_stream",
    summary: "The stream id in a 9 octet frame header as decimal digits, so the answer of a server uses the stream of the request.",
    params: &[req("header", ValueType::Bytes)],
};

/// `http2.frame_field`, one field of a frame header.
pub const FRAME_FIELD: FunctionSpec = FunctionSpec {
    name: "frame_field",
    summary: "One field of the 9 octet header at the start of frame: length, stream, flags, and type_code as decimal digits, type as the frame type name such as HEADERS, and end_stream, end_headers, ack, padded, and priority as true or false.",
    params: &[req("frame", ValueType::Bytes), req("field", ValueType::Text)],
};

/// `http2.data_payload`, the data of a DATA frame.
pub const DATA_PAYLOAD: FunctionSpec = FunctionSpec {
    name: "data_payload",
    summary: "The data octets of the DATA frame in frame, which holds header and payload, without the padding.",
    params: &[req("frame", ValueType::Bytes)],
};

/// `http2.headers_decode`, the header list of a HEADERS frame.
pub const HEADERS_DECODE: FunctionSpec = FunctionSpec {
    name: "headers_decode",
    summary: "The header list of the HEADERS frame in frames, which may be followed by its CONTINUATION frames, one 'name: value' per line. It updates the HPACK decoder of this side, so each block is decoded once, in the order it arrived.",
    params: &[req("frames", ValueType::Bytes)],
};

/// `http2.header_value`, one value of a decoded header list.
pub const HEADER_VALUE: FunctionSpec = FunctionSpec {
    name: "header_value",
    summary: "The value of the first field called name in a header list that headers_decode rendered. A missing field fails the call unless required is false, which renders an empty value.",
    params: &[
        req("headers", ValueType::Bytes),
        req("name", ValueType::Text),
        opt("required", ValueType::Boolean, DefaultValue::Boolean(true)),
    ],
};

/// `http2.settings_value`, one parameter of a SETTINGS frame.
pub const SETTINGS_VALUE: FunctionSpec = FunctionSpec {
    name: "settings_value",
    summary: "The value of a setting in the SETTINGS frame in frame as decimal digits. The name is one of header_table_size, enable_push, max_concurrent_streams, initial_window_size, max_frame_size, and max_header_list_size. A setting that the frame omits gives its initial value, and -1 stands for no limit.",
    params: &[req("frame", ValueType::Bytes), req("name", ValueType::Text)],
};

/// Every function of the module, in the order of the manifest.
pub const FUNCTIONS: &[FunctionSpec] = &[
    PREFACE,
    SETTINGS,
    SETTINGS_ACK,
    HEADERS,
    DATA,
    WINDOW_UPDATE,
    PING,
    PING_ACK,
    GOAWAY,
    RST_STREAM,
    FRAME,
    FRAME_LENGTH,
    FRAME_STREAM,
    FRAME_FIELD,
    DATA_PAYLOAD,
    HEADERS_DECODE,
    HEADER_VALUE,
    SETTINGS_VALUE,
];
