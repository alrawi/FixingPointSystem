// SPDX-License-Identifier: GPL-3.0-only
//! The header compression examples of RFC 7541 Appendix C, which the RFC
//! gives as hexadecimal wire bytes with the header list and the dynamic table
//! size after each block.
//!
//! The decoder must turn every wire block into its header list and reach the
//! table size of the RFC. The encoder must write the same wire bytes for the
//! same lists with the same choices as the RFC makes, which are incremental
//! indexing for every new field and Huffman coding in C.4 and C.6.

use fps_module_http2::hpack::{Decoder, Encoder, HuffmanMode, IndexMode};

fn hex(s: &str) -> Vec<u8> {
    let s: String = s.split_whitespace().collect();
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

type List = &'static [(&'static str, &'static str)];

/// One block of an example: the wire bytes, the header list, and the size of
/// the dynamic table after the block.
struct Case {
    wire: &'static str,
    list: List,
    table: usize,
}

fn decode_list(d: &mut Decoder, wire: &[u8]) -> Vec<(String, String)> {
    d.decode(wire)
        .unwrap()
        .into_iter()
        .map(|f| {
            (
                String::from_utf8(f.name).unwrap(),
                String::from_utf8(f.value).unwrap(),
            )
        })
        .collect()
}

fn want(list: List) -> Vec<(String, String)> {
    list.iter()
        .map(|(n, v)| (n.to_string(), v.to_string()))
        .collect()
}

/// Runs one example, a sequence of blocks on one connection, in both
/// directions.
fn run(cases: &[Case], table_max: usize, huffman: HuffmanMode, index: IndexMode) {
    let mut d = Decoder::with_max_size(table_max);
    let mut e = Encoder::with_max_size(table_max);
    for (i, c) in cases.iter().enumerate() {
        let wire = hex(c.wire);
        assert_eq!(decode_list(&mut d, &wire), want(c.list), "decode block {i}");
        assert_eq!(d.size(), c.table, "decoder table size after block {i}");
        let mut out = Vec::new();
        for (n, v) in c.list {
            e.encode(n.as_bytes(), v.as_bytes(), huffman, index, &mut out);
        }
        assert_eq!(out, wire, "encode block {i}");
        assert_eq!(e.size(), c.table, "encoder table size after block {i}");
    }
}

#[test]
fn c2_1_literal_field_with_indexing() {
    run(
        &[Case {
            wire: "400a 6375 7374 6f6d 2d6b 6579 0d63 7573 746f 6d2d 6865 6164 6572",
            list: &[("custom-key", "custom-header")],
            table: 55,
        }],
        4096,
        HuffmanMode::Never,
        IndexMode::Auto,
    );
}

#[test]
fn c2_2_literal_field_without_indexing() {
    let wire = hex("040c 2f73 616d 706c 652f 7061 7468");
    let mut d = Decoder::new();
    assert_eq!(
        decode_list(&mut d, &wire),
        want(&[(":path", "/sample/path")])
    );
    assert_eq!(d.size(), 0, "the table stays empty");
    let mut e = Encoder::new();
    let mut out = Vec::new();
    e.encode(
        b":path",
        b"/sample/path",
        HuffmanMode::Never,
        IndexMode::Off,
        &mut out,
    );
    assert_eq!(out, wire);
    assert!(e.is_empty());
}

#[test]
fn c2_3_literal_field_never_indexed() {
    let wire = hex("1008 7061 7373 776f 7264 0673 6563 7265 74");
    let mut d = Decoder::new();
    let fields = d.decode(&wire).unwrap();
    assert_eq!(fields.len(), 1);
    assert_eq!(fields[0].name, b"password");
    assert_eq!(fields[0].value, b"secret");
    assert!(fields[0].sensitive, "the field is marked never indexed");
    assert_eq!(d.size(), 0);
    // The encoder writes the same bytes for a name on its sensitive list.
    let mut e = Encoder::new();
    let mut out = Vec::new();
    e.encode(
        b"x-secret",
        b"secret",
        HuffmanMode::Never,
        IndexMode::Auto,
        &mut out,
    );
    assert_eq!(out[0] & 0xc0, 0x40, "an ordinary new field is indexed");
    let mut out = Vec::new();
    e.encode(
        b"authorization",
        b"secret",
        HuffmanMode::Never,
        IndexMode::Auto,
        &mut out,
    );
    assert_eq!(out[0] & 0xf0, 0x10, "a sensitive field is never indexed");
}

#[test]
fn c2_4_indexed_field() {
    run(
        &[Case {
            wire: "82",
            list: &[(":method", "GET")],
            table: 0,
        }],
        4096,
        HuffmanMode::Never,
        IndexMode::Auto,
    );
}

const REQUESTS: [List; 3] = [
    &[
        (":method", "GET"),
        (":scheme", "http"),
        (":path", "/"),
        (":authority", "www.example.com"),
    ],
    &[
        (":method", "GET"),
        (":scheme", "http"),
        (":path", "/"),
        (":authority", "www.example.com"),
        ("cache-control", "no-cache"),
    ],
    &[
        (":method", "GET"),
        (":scheme", "https"),
        (":path", "/index.html"),
        (":authority", "www.example.com"),
        ("custom-key", "custom-value"),
    ],
];

#[test]
fn c3_requests_without_huffman_coding() {
    run(
        &[
            Case {
                wire: "8286 8441 0f77 7777 2e65 7861 6d70 6c65 2e63 6f6d",
                list: REQUESTS[0],
                table: 57,
            },
            Case {
                wire: "8286 84be 5808 6e6f 2d63 6163 6865",
                list: REQUESTS[1],
                table: 110,
            },
            Case {
                wire: "8287 85bf 400a 6375 7374 6f6d 2d6b 6579 0c63 7573 746f 6d2d 7661 6c75 65",
                list: REQUESTS[2],
                table: 164,
            },
        ],
        4096,
        HuffmanMode::Never,
        IndexMode::Auto,
    );
}

#[test]
fn c4_requests_with_huffman_coding() {
    // Automatic Huffman coding picks the coding for every string in the
    // example, because it is shorter for each of them.
    for mode in [HuffmanMode::Auto, HuffmanMode::Always] {
        run(
            &[
                Case {
                    wire: "8286 8441 8cf1 e3c2 e5f2 3a6b a0ab 90f4 ff",
                    list: REQUESTS[0],
                    table: 57,
                },
                Case {
                    wire: "8286 84be 5886 a8eb 1064 9cbf",
                    list: REQUESTS[1],
                    table: 110,
                },
                Case {
                    wire: "8287 85bf 4088 25a8 49e9 5ba9 7d7f 8925 a849 e95b b8e8 b4bf",
                    list: REQUESTS[2],
                    table: 164,
                },
            ],
            4096,
            mode,
            IndexMode::Auto,
        );
    }
}

const RESPONSES: [List; 3] = [
    &[
        (":status", "302"),
        ("cache-control", "private"),
        ("date", "Mon, 21 Oct 2013 20:13:21 GMT"),
        ("location", "https://www.example.com"),
    ],
    &[
        (":status", "307"),
        ("cache-control", "private"),
        ("date", "Mon, 21 Oct 2013 20:13:21 GMT"),
        ("location", "https://www.example.com"),
    ],
    &[
        (":status", "200"),
        ("cache-control", "private"),
        ("date", "Mon, 21 Oct 2013 20:13:22 GMT"),
        ("location", "https://www.example.com"),
        ("content-encoding", "gzip"),
        (
            "set-cookie",
            "foo=ASDJKHQKBZXOQWEOPIUAXQWEOIU; max-age=3600; version=1",
        ),
    ],
];

#[test]
fn c5_responses_without_huffman_coding_in_a_table_of_256_octets() {
    // The third response evicts entries, and indexes a set-cookie field,
    // which only IndexMode::All does.
    run(
        &[
            Case {
                wire: "4803 3330 3258 0770 7269 7661 7465 611d 4d6f 6e2c 2032 3120 4f63 7420 3230 3133 2032 303a 3133 3a32 3120 474d 546e 1768 7474 7073 3a2f 2f77 7777 2e65 7861 6d70 6c65 2e63 6f6d",
                list: RESPONSES[0],
                table: 222,
            },
            Case {
                wire: "4803 3330 37c1 c0bf",
                list: RESPONSES[1],
                table: 222,
            },
            Case {
                wire: "88c1 611d 4d6f 6e2c 2032 3120 4f63 7420 3230 3133 2032 303a 3133 3a32 3220 474d 54c0 5a04 677a 6970 7738 666f 6f3d 4153 444a 4b48 514b 425a 584f 5157 454f 5049 5541 5851 5745 4f49 553b 206d 6178 2d61 6765 3d33 3630 303b 2076 6572 7369 6f6e 3d31",
                list: RESPONSES[2],
                table: 215,
            },
        ],
        256,
        HuffmanMode::Never,
        IndexMode::All,
    );
}

/// Like [`run`], but compares what the encoder wrote with the header list and
/// not with the RFC bytes, for a policy that may choose another coding.
fn round_trip(cases: &[Case], table_max: usize, huffman: HuffmanMode, index: IndexMode) {
    let mut d = Decoder::with_max_size(table_max);
    let mut e = Encoder::with_max_size(table_max);
    for (i, c) in cases.iter().enumerate() {
        let mut out = Vec::new();
        for (n, v) in c.list {
            e.encode(n.as_bytes(), v.as_bytes(), huffman, index, &mut out);
        }
        assert_eq!(decode_list(&mut d, &out), want(c.list), "block {i}");
        assert_eq!(
            (e.size(), d.size()),
            (c.table, c.table),
            "tables after block {i}"
        );
    }
}

#[test]
fn c6_responses_with_huffman_coding_in_a_table_of_256_octets() {
    // The RFC codes "307" with Huffman although that takes as many octets as
    // the plain string, so the automatic mode, which codes only when that is
    // strictly shorter, differs from the RFC in that one string. The mode that
    // always codes matches the RFC bytes, and the automatic mode must still
    // decode to the same lists and reach the same table sizes.
    for mode in [HuffmanMode::Auto, HuffmanMode::Always] {
        let check = if mode == HuffmanMode::Auto {
            round_trip
        } else {
            run
        };
        check(
            &[
                Case {
                    wire: "4882 6402 5885 aec3 771a 4b61 96d0 7abe 9410 54d4 44a8 2005 9504 0b81 66e0 82a6 2d1b ff6e 919d 29ad 1718 63c7 8f0b 97c8 e9ae 82ae 43d3",
                    list: RESPONSES[0],
                    table: 222,
                },
                Case {
                    wire: "4883 640e ffc1 c0bf",
                    list: RESPONSES[1],
                    table: 222,
                },
                Case {
                    wire: "88c1 6196 d07a be94 1054 d444 a820 0595 040b 8166 e084 a62d 1bff c05a 839b d9ab 77ad 94e7 821d d7f2 e6c7 b335 dfdf cd5b 3960 d5af 2708 7f36 72c1 ab27 0fb5 291f 9587 3160 65c0 03ed 4ee5 b106 3d50 07",
                    list: RESPONSES[2],
                    table: 215,
                },
            ],
            256,
            mode,
            IndexMode::All,
        );
    }
}

#[test]
fn the_default_policy_keeps_set_cookie_out_of_the_table() {
    // Only the set-cookie field of C.5.3 differs from the RFC bytes, and the
    // difference is the representation, never indexed instead of indexed.
    let mut e = Encoder::with_max_size(256);
    let mut out = Vec::new();
    for l in RESPONSES {
        out.clear();
        for (n, v) in l {
            e.encode(
                n.as_bytes(),
                v.as_bytes(),
                HuffmanMode::Never,
                IndexMode::Auto,
                &mut out,
            );
        }
    }
    let rfc = hex("88c1 611d 4d6f 6e2c 2032 3120 4f63 7420 3230 3133 2032 303a 3133 3a32 3220 474d 54c0 5a04 677a 6970");
    assert_eq!(out[..rfc.len()], rfc[..]);
    let rest = &out[rfc.len()..];
    // 0001 1111 then 55 - 15 = 40 (0x28): never indexed with name index 55.
    assert_eq!(rest[0], 0x1f);
    assert_eq!(rest[1], 0x28);
    // The decoder reads that block and does not index the field.
    let mut d = Decoder::with_max_size(256);
    for wire in [
        "4803 3330 3258 0770 7269 7661 7465 611d 4d6f 6e2c 2032 3120 4f63 7420 3230 3133 2032 303a 3133 3a32 3120 474d 546e 1768 7474 7073 3a2f 2f77 7777 2e65 7861 6d70 6c65 2e63 6f6d",
        "4803 3330 37c1 c0bf",
    ] {
        d.decode(&hex(wire)).unwrap();
    }
    let fields = d.decode(&out).unwrap();
    assert!(fields
        .last()
        .is_some_and(|f| f.sensitive && f.name == b"set-cookie"));
    assert_eq!(d.size(), 222, "the table holds the same entries as before");
}
