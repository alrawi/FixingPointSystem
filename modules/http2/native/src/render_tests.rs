// SPDX-License-Identifier: GPL-3.0-only
//! Tests of the template functions through the `TemplateFn` trait.

use super::*;
use fps_flow::function::NamedArg;
use fps_flow::{ArgValue, CallArgs};

fn call(
    h: &mut Http2,
    function: &'static str,
    positional: &[ArgValue<'_>],
    named: &[NamedArg<'_>],
) -> Result<Vec<u8>, FnError> {
    let mut rng = rand_pcg::Pcg64Mcg::new(42);
    let mut out = Vec::new();
    let c = FnCall {
        module: "http2",
        function,
        args: CallArgs { positional, named },
    };
    h.render(&c, &mut rng, &mut out).map(|()| out)
}

fn bytes(b: &[u8]) -> ArgValue<'_> {
    ArgValue::Bytes(b)
}

fn named<'a>(name: &'a str, value: ArgValue<'a>) -> NamedArg<'a> {
    NamedArg { name, value }
}

fn int(v: i64) -> ArgValue<'static> {
    ArgValue::Int(v)
}

fn text(v: &str) -> ArgValue<'_> {
    ArgValue::Text(v)
}

/// Feeds concatenated frames to `headers_decode` one payload at a time, as a
/// flow does after each pair of `expect` actions, and returns what it rendered.
fn feed(server: &mut Http2, frames: &[u8]) -> Result<Vec<u8>, FnError> {
    let mut rest = frames;
    let mut out = Vec::new();
    while !rest.is_empty() {
        let (f, tail) = frame::split(rest).unwrap();
        out.extend(call(
            server,
            "headers_decode",
            &[bytes(f.payload)],
            &[
                named("flags", int(i64::from(f.header.flags))),
                named("type", int(i64::from(f.header.ty))),
            ],
        )?);
        rest = tail;
    }
    Ok(out)
}

fn server_decode(server: &mut Http2, frames: &[u8]) -> Vec<u8> {
    feed(server, frames).unwrap()
}

#[test]
fn rejects_other_modules_and_functions() {
    let mut rng = rand_pcg::Pcg64Mcg::new(1);
    let c = FnCall {
        module: "http1",
        function: "preface",
        args: CallArgs {
            positional: &[],
            named: &[],
        },
    };
    let e = Http2::default()
        .render(&c, &mut rng, &mut Vec::new())
        .unwrap_err();
    assert_eq!(e.kind, FnErrorKind::Unsupported);
    let e = call(&mut Http2::default(), "push_promise", &[], &[]).unwrap_err();
    assert_eq!(e.kind, FnErrorKind::Unsupported);
}

#[test]
fn the_preface_and_the_settings_frames() {
    let mut h = Http2::default();
    assert_eq!(call(&mut h, "preface", &[], &[]).unwrap(), frame::PREFACE);
    assert_eq!(
        call(&mut h, "settings", &[], &[]).unwrap(),
        [0, 0, 0, 4, 0, 0, 0, 0, 0]
    );
    let s = call(
        &mut h,
        "settings",
        &[],
        &[
            named("max_concurrent_streams", int(100)),
            named("enable_push", int(0)),
            named("initial_window_size", int(1 << 20)),
        ],
    )
    .unwrap();
    // Identifier order, and not the order of the arguments.
    assert_eq!(
        s,
        [
            0, 0, 18, 4, 0, 0, 0, 0, 0, //
            0, 2, 0, 0, 0, 0, //
            0, 3, 0, 0, 0, 100, //
            0, 4, 0, 16, 0, 0
        ]
    );
    assert_eq!(
        call(&mut h, "settings_ack", &[], &[]).unwrap(),
        [0, 0, 0, 4, 1, 0, 0, 0, 0]
    );
    for (name, v) in [
        ("enable_push", 2),
        ("max_frame_size", 100),
        ("initial_window_size", 1 << 31),
        ("header_table_size", -2),
    ] {
        let e = call(&mut h, "settings", &[], &[named(name, int(v))]).unwrap_err();
        assert_eq!(e.kind, FnErrorKind::InvalidArgument, "{name}={v}");
    }
}

#[test]
fn settings_values_come_back_out_of_a_frame() {
    let mut h = Http2::default();
    let s = call(
        &mut h,
        "settings",
        &[],
        &[
            named("initial_window_size", int(1000)),
            named("max_concurrent_streams", int(7)),
        ],
    )
    .unwrap();
    let get = |h: &mut Http2, n: &str| {
        String::from_utf8(call(h, "settings_value", &[bytes(&s[9..]), text(n)], &[]).unwrap())
            .unwrap()
    };
    assert_eq!(get(&mut h, "initial_window_size"), "1000");
    assert_eq!(get(&mut h, "max_concurrent_streams"), "7");
    // Initial values of what the frame leaves out.
    assert_eq!(get(&mut h, "max_frame_size"), "16384");
    assert_eq!(get(&mut h, "header_table_size"), "4096");
    assert_eq!(get(&mut h, "max_header_list_size"), "-1");
    let e = call(
        &mut h,
        "settings_value",
        &[bytes(&s[9..]), text("colour")],
        &[],
    )
    .unwrap_err();
    assert_eq!(e.kind, FnErrorKind::InvalidArgument);
    let e = call(
        &mut h,
        "settings_value",
        &[bytes(&s[9..10]), text("enable_push")],
        &[],
    )
    .unwrap_err();
    assert_eq!(e.kind, FnErrorKind::Protocol);
}

const REQUEST: &[u8] = b":method: GET\n:scheme: https\n:path: /index.html\n:authority: www.example.com\nuser-agent: fps/0.1\naccept: */*\n";

#[test]
fn headers_decode_on_the_other_side_with_a_table_that_persists() {
    let (mut client, mut server) = (Http2::default(), Http2::default());
    let end = [named("end_stream", ArgValue::Boolean(true))];
    let first = call(&mut client, "headers", &[int(1), bytes(REQUEST)], &end).unwrap();
    let second = call(&mut client, "headers", &[int(3), bytes(REQUEST)], &end).unwrap();
    // The second request is nothing but indexed fields.
    assert!(
        second.len() < first.len() / 2,
        "{} {}",
        second.len(),
        first.len()
    );
    for frames in [&first, &second] {
        assert_eq!(server_decode(&mut server, frames), REQUEST);
    }
    let value = |server: &mut Http2, n: &str, extra: &[NamedArg<'_>]| {
        call(server, "header_value", &[bytes(REQUEST), text(n)], extra)
    };
    assert_eq!(value(&mut server, "user-agent", &[]).unwrap(), b"fps/0.1");
    assert_eq!(value(&mut server, ":path", &[]).unwrap(), b"/index.html");
    assert_eq!(
        value(&mut server, "etag", &[]).unwrap_err().kind,
        FnErrorKind::Protocol
    );
    let optional = [named("required", ArgValue::Boolean(false))];
    assert!(value(&mut server, "etag", &optional).unwrap().is_empty());
}

#[test]
fn a_table_size_update_goes_first_and_empties_the_table() {
    let (mut client, mut server) = (Http2::default(), Http2::default());
    let a = call(&mut client, "headers", &[int(1), bytes(b"x-one: 1\n")], &[]).unwrap();
    server_decode(&mut server, &a);
    let b = call(
        &mut client,
        "headers",
        &[int(3), bytes(b"x-one: 1\n")],
        &[named("table_size", int(0))],
    )
    .unwrap();
    // 0x20 is the size update to 0, then a literal, because the table is empty.
    assert_eq!(b[9], 0x20);
    assert_eq!(server_decode(&mut server, &b), b"x-one: 1\n");
    assert!(client.encoder.is_empty() && server.decoder.is_empty());
}

#[test]
fn a_large_header_list_continues_in_continuation_frames() {
    let (mut client, mut server) = (Http2::default(), Http2::default());
    let mut list = Vec::new();
    for i in 0..40 {
        list.extend_from_slice(format!("x-field-{i}: {}\n", "v".repeat(200)).as_bytes());
    }
    let f = call(
        &mut client,
        "headers",
        &[int(1), bytes(&list)],
        &[
            named("max_frame_size", int(1000)),
            named("huffman", text("never")),
        ],
    )
    .unwrap();
    let mut types = Vec::new();
    let mut rest = &f[..];
    while !rest.is_empty() {
        let (fr, r) = frame::split(rest).unwrap();
        assert!(fr.payload.len() <= 1000);
        types.push((fr.header.ty, fr.header.has(flag::END_HEADERS)));
        rest = r;
    }
    assert_eq!(types[0], (ty::HEADERS, false));
    assert!(types.len() > 5);
    assert!(types[1..].iter().all(|t| t.0 == ty::CONTINUATION));
    assert_eq!(types.iter().filter(|t| t.1).count(), 1);
    assert!(types.last().is_some_and(|t| t.1));
    assert_eq!(server_decode(&mut server, &f), list);
}

#[test]
fn headers_with_padding_and_priority_decode() {
    let (mut client, mut server) = (Http2::default(), Http2::default());
    let f = call(
        &mut client,
        "headers",
        &[int(5), bytes(b":status: 200\nserver: fps\n")],
        &[
            named("padding", int(10)),
            named("priority_dependency", int(3)),
            named("priority_weight", int(200)),
            named("priority_exclusive", ArgValue::Boolean(true)),
        ],
    )
    .unwrap();
    let field = |name: &str| {
        String::from_utf8(
            call(
                &mut Http2::default(),
                "frame_field",
                &[bytes(&f), text(name)],
                &[],
            )
            .unwrap(),
        )
        .unwrap()
    };
    assert_eq!(field("type"), "HEADERS");
    assert_eq!(field("padded"), "true");
    assert_eq!(field("priority"), "true");
    assert_eq!(field("end_headers"), "true");
    assert_eq!(field("stream"), "5");
    assert_eq!(
        server_decode(&mut server, &f),
        b":status: 200\nserver: fps\n"
    );
}

#[test]
fn bad_header_lists_and_arguments_fail_the_call() {
    let mut h = Http2::default();
    for list in [
        &b"no colon here\n"[..],
        b"Upper: x\n",
        b": x\n",
        b"a b: x\n",
        b"x: a\0b\n",
    ] {
        let e = call(&mut h, "headers", &[int(1), bytes(list)], &[]).unwrap_err();
        assert_eq!(e.kind, FnErrorKind::InvalidArgument, "{list:?}");
    }
    for (stream, name, value) in [
        (0, "padding", int(0)),
        (1, "padding", int(300)),
        (1, "huffman", text("sometimes")),
        (1, "index", text("on")),
        (1, "table_size", int(1 << 30)),
        (1, "priority_dependency", int(-5)),
        (1, "max_frame_size", int(0)),
    ] {
        let e = call(
            &mut h,
            "headers",
            &[int(stream), bytes(b"a: b\n")],
            &[named(name, value)],
        )
        .unwrap_err();
        assert_eq!(e.kind, FnErrorKind::InvalidArgument, "{stream} {name}");
    }
    // A refused call leaves the table as it was.
    assert!(h.encoder.is_empty());
}

#[test]
fn data_frames_carry_binary_bytes_and_split_at_the_frame_size() {
    let body: Vec<u8> = (0..=255).cycle().take(1000).collect();
    let f = call(
        &mut Http2::default(),
        "data",
        &[int(1), bytes(&body)],
        &[
            named("max_frame_size", int(400)),
            named("end_stream", ArgValue::Boolean(true)),
            named("padding", int(7)),
        ],
    )
    .unwrap();
    let mut got = Vec::new();
    let mut rest = &f[..];
    let mut ends = 0;
    while !rest.is_empty() {
        let (fr, r) = frame::split(rest).unwrap();
        assert!(fr.payload.len() <= 400);
        ends += usize::from(fr.header.has(flag::END_STREAM));
        got.extend(
            call(
                &mut Http2::default(),
                "data_payload",
                &[bytes(fr.payload)],
                &[named("flags", int(i64::from(fr.header.flags)))],
            )
            .unwrap(),
        );
        rest = r;
    }
    assert_eq!(got, body);
    assert_eq!(ends, 1);
}

#[test]
fn control_frames() {
    let mut h = Http2::default();
    assert_eq!(
        call(&mut h, "window_update", &[int(0), int(1000)], &[]).unwrap(),
        [0, 0, 4, 8, 0, 0, 0, 0, 0, 0, 0, 3, 232]
    );
    assert_eq!(
        call(
            &mut h,
            "rst_stream",
            &[int(3)],
            &[named("error", text("cancel"))]
        )
        .unwrap(),
        [0, 0, 4, 3, 0, 0, 0, 0, 3, 0, 0, 0, 8]
    );
    assert_eq!(
        call(&mut h, "rst_stream", &[int(3), int(2)], &[]).unwrap()[12],
        2
    );
    assert_eq!(
        call(&mut h, "goaway", &[], &[]).unwrap(),
        [0, 0, 8, 7, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]
    );
    let g = call(
        &mut h,
        "goaway",
        &[int(7), text("ENHANCE_YOUR_CALM"), bytes(b"slow down")],
        &[],
    )
    .unwrap();
    assert_eq!(&g[9..17], [0, 0, 0, 7, 0, 0, 0, 11]);
    assert_eq!(&g[17..], b"slow down");
    for bad in [text("OOPS"), int(-1), ArgValue::Boolean(true)] {
        assert!(call(&mut h, "rst_stream", &[int(1), bad], &[]).is_err());
    }
    let raw = call(
        &mut h,
        "frame",
        &[int(0xfa), int(1)],
        &[named("payload", bytes(b"zz")), named("flags", int(0x5))],
    )
    .unwrap();
    assert_eq!(raw, [0, 0, 2, 0xfa, 5, 0, 0, 0, 1, b'z', b'z']);
    assert!(call(&mut h, "frame", &[int(256), int(1)], &[]).is_err());
}

#[test]
fn ping_takes_its_octets_from_the_session_generator_and_is_answered() {
    let mut h = Http2::default();
    let a = call(&mut h, "ping", &[], &[]).unwrap();
    assert_eq!(
        a,
        call(&mut h, "ping", &[], &[]).unwrap(),
        "same generator state, same octets"
    );
    assert_eq!(&a[..9], [0, 0, 8, 6, 0, 0, 0, 0, 0]);
    let ack = call(&mut h, "ping_ack", &[bytes(&a[9..])], &[]).unwrap();
    assert_eq!(&ack[..9], [0, 0, 8, 6, 1, 0, 0, 0, 0]);
    assert_eq!(ack[9..], a[9..]);
    let fixed = call(&mut h, "ping", &[bytes(b"12345678")], &[]).unwrap();
    assert_eq!(&fixed[9..], b"12345678");
    assert_eq!(
        call(&mut h, "ping", &[bytes(b"123")], &[])
            .unwrap_err()
            .kind,
        FnErrorKind::InvalidArgument
    );
    assert_eq!(
        call(&mut h, "ping_ack", &[bytes(b"")], &[])
            .unwrap_err()
            .kind,
        FnErrorKind::Protocol
    );
    assert_eq!(
        call(&mut h, "ping_ack", &[bytes(&a[9..12])], &[])
            .unwrap_err()
            .kind,
        FnErrorKind::Protocol
    );
}

#[test]
fn frame_headers_read_back() {
    let mut h = Http2::default();
    let f = call(
        &mut h,
        "data",
        &[int(9), bytes(b"hello")],
        &[named("end_stream", ArgValue::Boolean(true))],
    )
    .unwrap();
    let get = |name| {
        call(
            &mut Http2::default(),
            "frame_field",
            &[bytes(&f[..9]), text(name)],
            &[],
        )
        .map(|v| String::from_utf8(v).unwrap())
    };
    assert_eq!(get("length").unwrap(), "5");
    assert_eq!(get("stream").unwrap(), "9");
    assert_eq!(get("type").unwrap(), "DATA");
    assert_eq!(get("type_code").unwrap(), "0");
    assert_eq!(get("flags").unwrap(), "1");
    assert_eq!(get("end_stream").unwrap(), "true");
    assert_eq!(get("padded").unwrap(), "false");
    assert!(get("colour").is_err());
    assert_eq!(
        call(&mut h, "frame_length", &[bytes(&f[..9])], &[]).unwrap(),
        b"5"
    );
    assert_eq!(
        call(&mut h, "frame_stream", &[bytes(&f[..9])], &[]).unwrap(),
        b"9"
    );
    assert_eq!(
        call(&mut h, "frame_length", &[bytes(&f[..8])], &[])
            .unwrap_err()
            .kind,
        FnErrorKind::Protocol
    );
}

#[test]
fn decoding_garbage_is_a_protocol_error_and_not_a_panic() {
    let mut h = Http2::default();
    let kind = |h: &mut Http2, frame: &[u8]| feed(h, frame).unwrap_err().kind;
    let mut hdr = Vec::new();
    frame::raw(&mut hdr, ty::HEADERS, flag::END_HEADERS, 1, &[0xff, 0xff]).unwrap();
    assert_eq!(kind(&mut h, &hdr), FnErrorKind::Protocol);
    // A DATA frame is no header block.
    let mut data = Vec::new();
    frame::raw(&mut data, ty::DATA, 0, 1, b"x").unwrap();
    assert_eq!(kind(&mut h, &data), FnErrorKind::Protocol);
    // A CONTINUATION frame without an open block.
    let mut cont = Vec::new();
    frame::raw(&mut cont, ty::CONTINUATION, flag::END_HEADERS, 1, &[0x82]).unwrap();
    assert_eq!(kind(&mut h, &cont), FnErrorKind::Protocol);
    // A value with a line break cannot go into the text form.
    let mut lf = Vec::new();
    let block = [0x00, 0x01, b'a', 0x03, b'b', b'\n', b'c'];
    frame::raw(&mut lf, ty::HEADERS, flag::END_HEADERS, 1, &block).unwrap();
    assert_eq!(kind(&mut h, &lf), FnErrorKind::Protocol);
    // A block that stays open renders nothing, and a second HEADERS frame
    // before its END_HEADERS is an error.
    let mut open = Vec::new();
    frame::raw(&mut open, ty::HEADERS, 0, 1, &[0x82]).unwrap();
    let mut h = Http2::default();
    assert!(feed(&mut h, &open).unwrap().is_empty());
    assert_eq!(kind(&mut h, &open), FnErrorKind::Protocol);
    // A bad flags or type argument.
    for (name, v) in [("flags", 256), ("type", -1)] {
        let e = call(
            &mut h,
            "headers_decode",
            &[bytes(b"")],
            &[named(name, int(v))],
        )
        .unwrap_err();
        assert_eq!(e.kind, FnErrorKind::InvalidArgument, "{name}");
    }
}

#[test]
fn a_table_size_in_settings_limits_what_the_decoder_accepts() {
    let mut server = Http2::default();
    call(
        &mut server,
        "settings",
        &[],
        &[named("header_table_size", int(0))],
    )
    .unwrap();
    let mut client = Http2::default();
    let f = call(
        &mut client,
        "headers",
        &[int(1), bytes(b"a: b\n")],
        &[named("table_size", int(100))],
    )
    .unwrap();
    assert_eq!(
        feed(&mut server, &f).unwrap_err().kind,
        FnErrorKind::Protocol
    );
}

fn count(function: &'static str, positional: &[ArgValue<'_>], named: &[NamedArg<'_>]) -> usize {
    let out = call(&mut Http2::default(), function, positional, named).unwrap();
    String::from_utf8(out).unwrap().parse().unwrap()
}

#[test]
fn data_frames_counts_the_frames_that_data_renders() {
    for (size, max, padding) in [
        (0, 16384, -1),
        (1, 16384, -1),
        (100, 100, -1),
        (101, 100, -1),
        (40000, 100, -1),
        (1000, 100, 9),
        (5000, 16384, 255),
    ] {
        let n = count(
            "data_frames",
            &[int(size)],
            &[
                named("max_frame_size", int(max)),
                named("padding", int(padding)),
            ],
        );
        let body = vec![7u8; size as usize];
        let f = call(
            &mut Http2::default(),
            "data",
            &[int(1), bytes(&body)],
            &[
                named("max_frame_size", int(max)),
                named("padding", int(padding)),
            ],
        )
        .unwrap();
        let mut rest = &f[..];
        let mut frames = 0;
        while !rest.is_empty() {
            rest = frame::split(rest).unwrap().1;
            frames += 1;
        }
        assert_eq!(n, frames, "size {size} max {max} padding {padding}");
    }
}

#[test]
fn data_frames_rejects_bad_arguments() {
    let bad = |size, max, padding| {
        call(
            &mut Http2::default(),
            "data_frames",
            &[int(size)],
            &[
                named("max_frame_size", int(max)),
                named("padding", int(padding)),
            ],
        )
        .unwrap_err()
        .kind
    };
    assert_eq!(bad(-1, 16384, -1), FnErrorKind::InvalidArgument);
    assert_eq!(bad(1, 0, -1), FnErrorKind::InvalidArgument);
    assert_eq!(bad(1, 16384, 300), FnErrorKind::InvalidArgument);
    assert_eq!(bad(1, 100, 99), FnErrorKind::InvalidArgument);
}

#[test]
fn headers_frames_counts_the_frames_that_headers_renders() {
    let list = "\
:method: POST
:path: /upload
:authority: www.example.test
user-agent: fps-http2-test/1.0
x-request-id: 0123456789abcdef0123456789abcdef
";
    for (max, padding, dep, huffman) in [
        (16384, -1, -1, "auto"),
        (100, -1, -1, "never"),
        (50, -1, -1, "never"),
        (60, 5, 0, "never"),
        (40, -1, 3, "always"),
    ] {
        let n = count(
            "headers_frames",
            &[bytes(list.as_bytes())],
            &[
                named("max_frame_size", int(max)),
                named("huffman", text(huffman)),
                named("padding", int(padding)),
                named("priority_dependency", int(dep)),
            ],
        );
        let f = call(
            &mut Http2::default(),
            "headers",
            &[int(1), bytes(list.as_bytes())],
            &[
                named("max_frame_size", int(max)),
                named("huffman", text(huffman)),
                named("index", text("off")),
                named("padding", int(padding)),
                named("priority_dependency", int(dep)),
            ],
        )
        .unwrap();
        let mut rest = &f[..];
        let mut frames = 0;
        while !rest.is_empty() {
            rest = frame::split(rest).unwrap().1;
            frames += 1;
        }
        assert_eq!(n, frames, "max {max} padding {padding} dep {dep} {huffman}");
    }
}
