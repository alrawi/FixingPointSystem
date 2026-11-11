// SPDX-License-Identifier: GPL-3.0-only
//! The template function catalog of the `hls` module.
//!
//! The entries are the `functions` list that `modules/hls/manifest.yaml`
//! carries, name for name, parameter for parameter, in the same order and with
//! the same types and defaults, as the contract section "Template function
//! catalog" requires. `cargo run -p fps-module-hls --example print_catalog`
//! prints the list in manifest syntax.

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

use DefaultValue::{Boolean as B, Int as I};
use ValueType::{Boolean, Bytes, Int, Text};

macro_rules! function {
    ($ident:ident, $name:literal, $summary:literal, [$($param:expr),* $(,)?]) => {
        #[doc = concat!("`hls.", $name, "`.")]
        pub const $ident: FunctionSpec = FunctionSpec {
            name: $name,
            summary: $summary,
            params: &[$($param),*],
        };
    };
}

function!(MASTER, "master",
    "Master playlist with one EXT-X-STREAM-INF per rung of the ladder, which is text such as '400000@640x360,800000@960x540', and the media playlist path v0/index.m3u8, v1/index.m3u8, and so on.",
    [req("ladder", Text)]);

function!(MEDIA, "media",
    "Media playlist of one rung. A VOD list names the segments from first on and ends with ENDLIST. A live list names count segments from first and has no end. Segments are seg00000.ts and so on.",
    [
        req("segments", Int),
        req("segment_ms", Int),
        opt("first", Int, I(0)),
        opt("count", Int, I(0)),
        opt("live", Boolean, B(false)),
    ]);

function!(SEGMENT, "segment",
    "MPEG-TS body of the segment that a request line names, such as GET /v1/seg00007.ts. Its size is the bit rate of that rung times segment_ms, in whole 188 octet packets.",
    [
        req("request", Bytes),
        req("ladder", Text),
        req("segment_ms", Int),
    ]);

function!(SEGMENT_SIZE, "segment_size",
    "Size in octets of a segment of the rung, as text. It is the bit rate times segment_ms divided by 8000, rounded to whole 188 octet packets, and at least three packets.",
    [
        req("ladder", Text),
        req("variant", Int),
        req("segment_ms", Int),
    ]);

function!(CONTENT_LENGTH, "content_length",
    "Value of the Content-Length header of an HTTP message, as text, so a flow can read a body of any size after the header block.",
    [req("message", Bytes)]);

function!(PLAYLIST_URI, "playlist_uri",
    "The index-th URI line of a playlist, counting from 0 and skipping tag and blank lines, as text. A negative index counts from the end, so -1 is the last URI.",
    [req("playlist", Bytes), req("index", Int)]);

function!(
    PLAYLIST_COUNT,
    "playlist_count",
    "Number of URI lines of a playlist, as text.",
    [req("playlist", Bytes)]
);

function!(RESOLVE, "resolve",
    "Request path of a playlist URI relative to the path of the playlist that names it. A URI that starts with a slash stays as it is.",
    [req("base", Bytes), req("uri", Bytes)]);

function!(RENDITION, "rendition",
    "Rung that a player picks for segment n, as text. The plan fixed:I stays on rung I, and ramp:K starts on rung 0 and moves up one rung every K segments until the top rung.",
    [
        req("ladder", Text),
        req("plan", Text),
        req("n", Int),
    ]);

function!(NEXT, "next",
    "Counter of this flow side, as text. The first call gives 0, and each call adds step to it, so a loop can number its segments. A step of 0 reads the counter and leaves it.",
    [opt("step", Int, I(1))]);

function!(PACE, "pace",
    "Milliseconds to sleep until the next segment of this flow side is due, as text. Each call moves the due time on by one segment duration, so a late wake shortens the next sleep.",
    [req("segment_ms", Int)]);

/// Every template function of the module, in the order of the manifest.
pub const FUNCTIONS: &[FunctionSpec] = &[
    MASTER,
    MEDIA,
    SEGMENT,
    SEGMENT_SIZE,
    CONTENT_LENGTH,
    PLAYLIST_URI,
    PLAYLIST_COUNT,
    RESOLVE,
    RENDITION,
    NEXT,
    PACE,
];

fn yaml_text(s: &str) -> String {
    let mut out = String::from("\"");
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            other => out.push(other),
        }
    }
    out.push('"');
    out
}

/// Renders [`FUNCTIONS`] as the `functions` list of a module manifest, in the
/// syntax of `modules/imap/manifest.yaml`. MOD-13 pastes the result into
/// `modules/smb2/manifest.yaml`, and the catalog test compares the two.
pub fn manifest_functions() -> String {
    let mut out = String::from("functions:\n");
    for f in FUNCTIONS {
        out.push_str(&format!("  - name: {}\n", f.name));
        out.push_str(&format!("    summary: {}\n", yaml_text(f.summary)));
        if f.params.is_empty() {
            out.push_str("    params: []\n");
            continue;
        }
        out.push_str("    params:\n");
        for p in f.params {
            let mut line = format!("      - {{ name: {}, type: {}", p.name, p.ty);
            match p.default {
                None => {}
                Some(DefaultValue::Text(t)) => {
                    line.push_str(&format!(", default: {}", yaml_text(t)))
                }
                Some(DefaultValue::Int(i)) => line.push_str(&format!(", default: {i}")),
                Some(DefaultValue::Boolean(b)) => line.push_str(&format!(", default: {b}")),
                Some(DefaultValue::Bytes(b)) => {
                    let hex: String = b.iter().map(|x| format!("{x:02x}")).collect();
                    line.push_str(&format!(", default: {}", yaml_text(&hex)));
                }
            }
            line.push_str(" }\n");
            out.push_str(&line);
        }
    }
    out
}
