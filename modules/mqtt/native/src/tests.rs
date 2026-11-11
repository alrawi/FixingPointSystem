// SPDX-License-Identifier: GPL-3.0-only
//! Unit tests of the packet encoders, the readers, and the template functions.

use fps_flow::function::NamedArg;
use fps_flow::{ArgValue, CallArgs};
use rand::{RngCore, SeedableRng};

use super::*;
use crate::read::{field, parse, Assembler};
use crate::wire::{
    ack, connack, connect, disconnect, get_varint, pingreq, pingresp, publish, put_varint,
    suback, subscribe, unsuback, unsubscribe, varint_len, Connack, Connect, Publish,
    Subscription, Will,
};

fn varint(v: u32) -> Vec<u8> {
    let mut out = Vec::new();
    put_varint(&mut out, v).unwrap();
    out
}

// ---------------------------------------------------------------------------
// Variable byte integers: the boundary values of the standard, section 1.5.5
// of MQTT 5.0 and 2.2.3 of MQTT 3.1.1.

#[test]
fn varint_boundaries_follow_the_standard() {
    let cases: &[(u32, &[u8])] = &[
        (0, &[0x00]),
        (127, &[0x7F]),
        (128, &[0x80, 0x01]),
        (16_383, &[0xFF, 0x7F]),
        (16_384, &[0x80, 0x80, 0x01]),
        (2_097_151, &[0xFF, 0xFF, 0x7F]),
        (2_097_152, &[0x80, 0x80, 0x80, 0x01]),
        (268_435_455, &[0xFF, 0xFF, 0xFF, 0x7F]),
    ];
    for (value, bytes) in cases {
        assert_eq!(varint(*value), *bytes, "{value}");
        assert_eq!(get_varint(bytes).unwrap(), (*value, bytes.len()), "{value}");
        assert_eq!(varint_len(*value), bytes.len());
    }
}

#[test]
fn varint_rejects_what_the_standard_forbids() {
    assert!(put_varint(&mut Vec::new(), 268_435_456).is_err());
    // Five bytes, and a value that does not use the least number of bytes.
    assert!(get_varint(&[0x80, 0x80, 0x80, 0x80, 0x01]).is_err());
    assert!(get_varint(&[0x80, 0x00]).is_err());
    assert!(get_varint(&[0xFF, 0xFF, 0xFF, 0x80]).is_err());
    // The integer ends before its last byte.
    assert!(get_varint(&[0x80]).is_err());
    assert!(get_varint(&[]).is_err());
}

#[test]
fn varint_round_trips() {
    let mut rng = rand_pcg::Pcg64Mcg::new(5);
    for _ in 0..2000 {
        let v = rng.next_u32() % (268_435_455 + 1);
        let shift = rng.next_u32() % 28;
        let v = v >> shift;
        let b = varint(v);
        assert_eq!(get_varint(&b).unwrap(), (v, b.len()));
    }
}

// ---------------------------------------------------------------------------
// Packets with known bytes.

fn enc<F: FnOnce(&mut Vec<u8>)>(f: F) -> Vec<u8> {
    let mut v = Vec::new();
    f(&mut v);
    v
}

fn base_connect(version: Version) -> Connect<'static> {
    Connect {
        version,
        client_id: "",
        keepalive: 60,
        clean: true,
        user: None,
        password: None,
        will: None,
        session_expiry: None,
        receive_max: None,
        topic_alias_max: None,
    }
}

#[test]
fn connect_matches_the_known_bytes() {
    let b = enc(|o| connect(o, &base_connect(Version::V311)).unwrap());
    assert_eq!(
        b,
        [0x10, 0x0C, 0x00, 0x04, b'M', b'Q', b'T', b'T', 0x04, 0x02, 0x00, 0x3C, 0x00, 0x00]
    );
    let b = enc(|o| connect(o, &base_connect(Version::V5)).unwrap());
    assert_eq!(
        b,
        [
            0x10, 0x0D, 0x00, 0x04, b'M', b'Q', b'T', b'T', 0x05, 0x02, 0x00, 0x3C, 0x00, 0x00,
            0x00
        ]
    );
}

#[test]
fn publish_and_acks_match_the_known_bytes() {
    let p = Publish {
        version: Version::V311,
        topic: "a/b",
        payload: b"hi",
        qos: 1,
        retain: false,
        dup: false,
        packet_id: 10,
        message_expiry: None,
        content_type: None,
        payload_format: None,
        response_topic: None,
        user_property: None,
    };
    assert_eq!(
        enc(|o| publish(o, &p).unwrap()),
        [0x32, 0x09, 0x00, 0x03, b'a', b'/', b'b', 0x00, 0x0A, b'h', b'i']
    );
    let q0 = Publish { qos: 0, retain: true, ..p };
    assert_eq!(
        enc(|o| publish(o, &q0).unwrap()),
        [0x31, 0x07, 0x00, 0x03, b'a', b'/', b'b', b'h', b'i']
    );
    assert_eq!(
        enc(|o| ack(o, Version::V311, AckKind::Puback, 10, 0, None).unwrap()),
        [0x40, 0x02, 0x00, 0x0A]
    );
    assert_eq!(
        enc(|o| ack(o, Version::V311, AckKind::Pubrec, 10, 0, None).unwrap()),
        [0x50, 0x02, 0x00, 0x0A]
    );
    // PUBREL carries the fixed flags 0b0010.
    assert_eq!(
        enc(|o| ack(o, Version::V311, AckKind::Pubrel, 10, 0, None).unwrap()),
        [0x62, 0x02, 0x00, 0x0A]
    );
    assert_eq!(
        enc(|o| ack(o, Version::V311, AckKind::Pubcomp, 10, 0, None).unwrap()),
        [0x70, 0x02, 0x00, 0x0A]
    );
    // MQTT 5.0 leaves out the success reason, and writes any other reason.
    assert_eq!(
        enc(|o| ack(o, Version::V5, AckKind::Puback, 10, 0, None).unwrap()),
        [0x40, 0x02, 0x00, 0x0A]
    );
    assert_eq!(
        enc(|o| ack(o, Version::V5, AckKind::Puback, 10, 16, None).unwrap()),
        [0x40, 0x03, 0x00, 0x0A, 0x10]
    );
}

#[test]
fn subscribe_ping_and_disconnect_match_the_known_bytes() {
    let sub = |q| Subscription {
        filter: "a/+",
        qos: q,
        no_local: false,
        retain_as_published: false,
        retain_handling: 0,
    };
    assert_eq!(
        enc(|o| subscribe(o, Version::V311, 1, &[sub(1)]).unwrap()),
        [0x82, 0x08, 0x00, 0x01, 0x00, 0x03, b'a', b'/', b'+', 0x01]
    );
    assert_eq!(
        enc(|o| subscribe(o, Version::V5, 1, &[sub(2)]).unwrap()),
        [0x82, 0x09, 0x00, 0x01, 0x00, 0x00, 0x03, b'a', b'/', b'+', 0x02]
    );
    assert_eq!(enc(|o| suback(o, Version::V311, 1, &[1]).unwrap()), [0x90, 0x03, 0x00, 0x01, 0x01]);
    assert_eq!(
        enc(|o| suback(o, Version::V5, 1, &[1, 128]).unwrap()),
        [0x90, 0x05, 0x00, 0x01, 0x00, 0x01, 0x80]
    );
    assert_eq!(enc(pingreq), [0xC0, 0x00]);
    assert_eq!(enc(pingresp), [0xD0, 0x00]);
    assert_eq!(enc(|o| disconnect(o, Version::V311, 0, None).unwrap()), [0xE0, 0x00]);
    assert_eq!(enc(|o| disconnect(o, Version::V5, 0, None).unwrap()), [0xE0, 0x00]);
    assert_eq!(enc(|o| disconnect(o, Version::V5, 4, None).unwrap()), [0xE0, 0x01, 0x04]);
    assert_eq!(
        enc(|o| unsuback(o, Version::V311, 7, &[0]).unwrap()),
        [0xB0, 0x02, 0x00, 0x07]
    );
    assert_eq!(
        enc(|o| unsuback(o, Version::V5, 7, &[0, 17]).unwrap()),
        [0xB0, 0x05, 0x00, 0x07, 0x00, 0x00, 0x11]
    );
    assert_eq!(
        enc(|o| unsubscribe(o, Version::V311, 7, &["a/b"]).unwrap()),
        [0xA2, 0x07, 0x00, 0x07, 0x00, 0x03, b'a', b'/', b'b']
    );
}

// ---------------------------------------------------------------------------
// Round trips through the reader.

fn rt(version: Version, packet: &[u8]) -> Packet {
    parse(version, packet).unwrap_or_else(|e| panic!("{e}: {packet:02x?}"))
}

#[test]
fn every_packet_round_trips_in_both_versions() {
    for v in [Version::V311, Version::V5] {
        let c = Connect {
            version: v,
            client_id: "mqttjs_0badf00d",
            keepalive: 30,
            clean: false,
            user: Some("sensor-0412"),
            password: Some(b"Tst-amber-falcon-27"),
            will: Some(Will {
                topic: "fps/status",
                payload: b"offline",
                qos: 1,
                retain: true,
            }),
            session_expiry: v.is_v5().then_some(300),
            receive_max: v.is_v5().then_some(20),
            topic_alias_max: v.is_v5().then_some(10),
        };
        let p = rt(v, &enc(|o| connect(o, &c).unwrap()));
        assert_eq!(p.kind, Kind::Connect);
        assert_eq!(p.version, v);
        assert_eq!(p.client_id, "mqttjs_0badf00d");
        assert_eq!(p.keepalive, 30);
        assert!(!p.clean);
        assert_eq!(p.user.as_deref(), Some("sensor-0412"));
        assert_eq!(p.password.as_deref(), Some(&b"Tst-amber-falcon-27"[..]));
        let w = p.will.clone().unwrap();
        assert_eq!((w.topic.as_str(), w.payload.as_slice(), w.qos, w.retain), ("fps/status", &b"offline"[..], 1, true));
        if v.is_v5() {
            assert_eq!(p.property("session_expiry_interval"), Some(&read::PropValue::U32(300)));
            assert_eq!(p.property("receive_maximum"), Some(&read::PropValue::U16(20)));
        }

        let a = Connack {
            version: v,
            session_present: true,
            code: 0,
            session_expiry: None,
            receive_max: v.is_v5().then_some(20),
            topic_alias_max: v.is_v5().then_some(10),
            server_keepalive: v.is_v5().then_some(45),
            assigned_client_id: v.is_v5().then_some("auto-1234"),
            max_qos: v.is_v5().then_some(1),
            retain_available: v.is_v5().then_some(true),
        };
        let p = rt(v, &enc(|o| connack(o, &a).unwrap()));
        assert!(p.session_present);
        assert_eq!(p.code, 0);
        if v.is_v5() {
            assert_eq!(field(&p, "server_keep_alive").unwrap(), b"45");
            assert_eq!(field(&p, "assigned_client_identifier").unwrap(), b"auto-1234");
        }

        for qos in 0..=2u8 {
            let pb = Publish {
                version: v,
                topic: "fps/site-3/telemetry",
                payload: &[0u8, 255, 7, 8],
                qos,
                retain: qos == 1,
                dup: qos == 2,
                packet_id: 513,
                message_expiry: v.is_v5().then_some(60),
                content_type: v.is_v5().then_some("application/json"),
                payload_format: v.is_v5().then_some(1),
                response_topic: v.is_v5().then_some("fps/reply"),
                user_property: v.is_v5().then_some(("origin", "fixture")),
            };
            if qos == 0 {
                let ok = Publish { dup: false, ..pb };
                let p = rt(v, &enc(|o| publish(o, &ok).unwrap()));
                assert_eq!(p.packet_id, None);
                assert_eq!(p.qos, 0);
                assert_eq!(p.payload, [0, 255, 7, 8]);
                continue;
            }
            let p = rt(v, &enc(|o| publish(o, &pb).unwrap()));
            assert_eq!((p.qos, p.packet_id, p.dup, p.retain), (qos, Some(513), qos == 2, qos == 1));
            assert_eq!(p.topic, "fps/site-3/telemetry");
            assert_eq!(p.payload, [0, 255, 7, 8]);
            if v.is_v5() {
                assert_eq!(field(&p, "content_type").unwrap(), b"application/json");
                assert_eq!(field(&p, "user_property").unwrap(), b"origin=fixture");
            }
        }

        for k in [AckKind::Puback, AckKind::Pubrec, AckKind::Pubrel, AckKind::Pubcomp] {
            for reason in [0u8, 146] {
                if !v.is_v5() && reason != 0 {
                    continue;
                }
                if !k.reasons().contains(&reason) {
                    assert!(enc_err(|o| ack(o, v, k, 9, reason, None)));
                    continue;
                }
                let p = rt(v, &enc(|o| ack(o, v, k, 9, reason, None).unwrap()));
                assert_eq!((p.kind, p.packet_id, p.code), (k.kind(), Some(9), reason));
            }
        }

        let subs = [
            Subscription { filter: "fps/+/telemetry", qos: 1, no_local: v.is_v5(), retain_as_published: false, retain_handling: u8::from(v.is_v5()) },
            Subscription { filter: "fps/#", qos: 2, no_local: false, retain_as_published: v.is_v5(), retain_handling: 0 },
        ];
        let p = rt(v, &enc(|o| subscribe(o, v, 3, &subs).unwrap()));
        assert_eq!(p.filters.len(), 2);
        assert_eq!(p.filters[0].filter, "fps/+/telemetry");
        assert_eq!(p.filters[1].qos(), 2);
        assert_eq!(field(&p, "requested_qos").unwrap(), b"1,2");

        let p = rt(v, &enc(|o| suback(o, v, 3, &[1, 2]).unwrap()));
        assert_eq!(p.codes, [1, 2]);
        let p = rt(v, &enc(|o| unsubscribe(o, v, 4, &["fps/#", "a/b"]).unwrap()));
        assert_eq!(field(&p, "filters").unwrap(), b"fps/#,a/b");
        let p = rt(v, &enc(|o| unsuback(o, v, 4, &[0, 0]).unwrap()));
        assert_eq!(p.packet_id, Some(4));
        assert_eq!(rt(v, &enc(pingreq)).kind, Kind::Pingreq);
        assert_eq!(rt(v, &enc(pingresp)).kind, Kind::Pingresp);
        assert_eq!(rt(v, &enc(|o| disconnect(o, v, 0, None).unwrap())).kind, Kind::Disconnect);
    }
}

fn enc_err<F: FnOnce(&mut Vec<u8>) -> wire::Result<()>>(f: F) -> bool {
    f(&mut Vec::new()).is_err()
}

#[test]
fn encoders_refuse_what_the_standard_forbids() {
    let base = |v| Publish {
        version: v,
        topic: "a/b",
        payload: b"",
        qos: 1,
        retain: false,
        dup: false,
        packet_id: 1,
        message_expiry: None,
        content_type: None,
        payload_format: None,
        response_topic: None,
        user_property: None,
    };
    // A packet identifier of 0, a topic name with a wildcard, QoS 3, DUP at QoS 0.
    assert!(enc_err(|o| publish(o, &Publish { packet_id: 0, ..base(Version::V311) })));
    assert!(enc_err(|o| publish(o, &Publish { topic: "a/+", ..base(Version::V311) })));
    assert!(enc_err(|o| publish(o, &Publish { topic: "", ..base(Version::V311) })));
    assert!(enc_err(|o| publish(o, &Publish { qos: 3, ..base(Version::V311) })));
    assert!(enc_err(|o| publish(o, &Publish { qos: 0, dup: true, ..base(Version::V311) })));
    // A property in a session of MQTT 3.1.1.
    assert!(enc_err(|o| publish(o, &Publish { message_expiry: Some(5), ..base(Version::V311) })));
    // A topic name with the null character.
    assert!(enc_err(|o| publish(o, &Publish { topic: "a\0b", ..base(Version::V5) })));
    // Filters.
    let sub = |f| [Subscription { filter: f, qos: 0, no_local: false, retain_as_published: false, retain_handling: 0 }];
    for bad in ["", "a/#/b", "a#", "a+", "a/b+/c", "#/a"] {
        assert!(enc_err(|o| subscribe(o, Version::V311, 1, &sub(bad))), "{bad}");
    }
    for good in ["#", "+", "+/+", "a/#", "/", "a//b", "$SYS/#"] {
        assert!(!enc_err(|o| subscribe(o, Version::V311, 1, &sub(good))), "{good}");
    }
    // Subscription options of MQTT 5.0 in a session of 3.1.1.
    let nl = [Subscription { filter: "a", qos: 0, no_local: true, retain_as_published: false, retain_handling: 0 }];
    assert!(enc_err(|o| subscribe(o, Version::V311, 1, &nl)));
    // Codes.
    assert!(enc_err(|o| suback(o, Version::V311, 1, &[3])));
    assert!(enc_err(|o| suback(o, Version::V311, 1, &[0x83])));
    assert!(enc_err(|o| connack(o, &Connack { version: Version::V311, session_present: false, code: 6, session_expiry: None, receive_max: None, topic_alias_max: None, server_keepalive: None, assigned_client_id: None, max_qos: None, retain_available: None })));
    assert!(enc_err(|o| disconnect(o, Version::V311, 4, None)));
    assert!(enc_err(|o| ack(o, Version::V311, AckKind::Puback, 1, 16, None)));
    // A 3.1.1 client with an empty identifier needs a clean session.
    assert!(enc_err(|o| connect(o, &Connect { clean: false, ..base_connect(Version::V311) })));
    assert!(!enc_err(|o| connect(o, &Connect { clean: false, ..base_connect(Version::V5) })));
}

#[test]
fn the_reader_refuses_what_the_standard_forbids() {
    let v = Version::V311;
    // Wrong fixed flags: PUBREL with flags 0, SUBSCRIBE with flags 0, PINGREQ with flags 2.
    assert!(parse(v, &[0x60, 0x02, 0x00, 0x01]).is_err());
    assert!(parse(v, &[0x80, 0x06, 0x00, 0x01, 0x00, 0x01, b'a', 0x00]).is_err());
    assert!(parse(v, &[0xC2, 0x00]).is_err());
    // A Remaining Length that does not match the bytes.
    assert!(parse(v, &[0x40, 0x03, 0x00, 0x01]).is_err());
    assert!(parse(v, &[0x40, 0x01, 0x00]).is_err());
    // PUBLISH with QoS 3, with QoS 0 and DUP, and without bytes for the identifier.
    assert!(parse(v, &[0x36, 0x05, 0x00, 0x01, b'a', 0x00, 0x01]).is_err());
    assert!(parse(v, &[0x38, 0x03, 0x00, 0x01, b'a']).is_err());
    assert!(parse(v, &[0x32, 0x03, 0x00, 0x01, b'a']).is_err());
    // A packet type that the module does not read, and AUTH.
    assert!(parse(v, &[0x00, 0x00]).is_err());
    assert!(parse(Version::V5, &[0xF0, 0x00]).is_err());
    // A CONNECT with another protocol name or level.
    let mut c = enc(|o| connect(o, &base_connect(Version::V311)).unwrap());
    c[8] = 3;
    assert!(parse(v, &c).is_err());
    let mut c = enc(|o| connect(o, &base_connect(Version::V311)).unwrap());
    c[4] = b'X';
    assert!(parse(v, &c).is_err());
    // A string that is not UTF-8.
    assert!(parse(v, &[0x30, 0x04, 0x00, 0x02, 0xFF, 0xFE]).is_err());
    // A property that occurs twice.
    assert!(parse(
        Version::V5,
        &[0x20, 0x0A, 0x00, 0x00, 0x07, 0x21, 0x00, 0x05, 0x21, 0x00, 0x06]
    )
    .is_err());
    // An unknown property.
    assert!(parse(Version::V5, &[0x20, 0x05, 0x00, 0x00, 0x02, 0x7E, 0x01]).is_err());
}

#[test]
fn the_reader_survives_random_bytes() {
    let mut rng = rand_pcg::Pcg64Mcg::new(77);
    let seeds: Vec<Vec<u8>> = vec![
        enc(|o| connect(o, &base_connect(Version::V5)).unwrap()),
        enc(|o| subscribe(o, Version::V5, 3, &[Subscription { filter: "a/#", qos: 1, no_local: false, retain_as_published: false, retain_handling: 0 }]).unwrap()),
    ];
    for _ in 0..20_000 {
        let mut b = if rng.next_u32() % 3 == 0 {
            let n = (rng.next_u32() % 40) as usize;
            let mut v = vec![0u8; n];
            rng.fill_bytes(&mut v);
            v
        } else {
            seeds[(rng.next_u32() % 2) as usize].clone()
        };
        for _ in 0..(rng.next_u32() % 4) {
            if b.is_empty() {
                break;
            }
            let i = (rng.next_u32() as usize) % b.len();
            b[i] ^= 1 << (rng.next_u32() % 8);
        }
        for v in [Version::V311, Version::V5] {
            let _ = parse(v, &b);
        }
    }
}

// ---------------------------------------------------------------------------
// The assembler and the template functions.

fn call_with(
    m: &mut Mqtt,
    rng: &mut dyn RngCore,
    function: &'static str,
    positional: &[ArgValue<'_>],
    named: &[NamedArg<'_>],
) -> Result<Vec<u8>, FnError> {
    let mut out = Vec::new();
    let c = FnCall {
        module: MODULE_NAME,
        function,
        args: CallArgs { positional, named },
    };
    m.render(&c, rng, &mut out).map(|()| out)
}

fn call(
    m: &mut Mqtt,
    function: &'static str,
    positional: &[ArgValue<'_>],
    named: &[NamedArg<'_>],
) -> Result<Vec<u8>, FnError> {
    call_with(m, &mut rand_pcg::Pcg64Mcg::new(42), function, positional, named)
}

fn text(v: Vec<u8>) -> String {
    String::from_utf8(v).unwrap()
}

fn named<'a>(name: &'a str, value: ArgValue<'a>) -> NamedArg<'a> {
    NamedArg { name, value }
}

/// Reads `packet` the way a flow does: one byte, then six reads of the size
/// that `feed` returns. Returns the name that `recv` gives.
fn flow_read(m: &mut Mqtt, packet: &[u8], expect: &'static str) -> Result<String, FnError> {
    let mut at = 1;
    let mut last: Vec<u8> = packet[..1].to_vec();
    for _ in 0..6 {
        let n: usize = text(call(m, "feed", &[ArgValue::Bytes(&last)], &[])?).parse().unwrap();
        last = packet[at..at + n].to_vec();
        at += n;
    }
    assert_eq!(at, packet.len(), "the six reads cover the packet");
    call(m, "recv", &[ArgValue::Text(expect)], &[]).map(text)
}

#[test]
fn the_assembler_reads_every_remaining_length_in_six_reads() {
    for body in [0usize, 1, 127, 128, 16_383, 16_384, 70_000] {
        let payload = vec![0x5A; body];
        let packet = enc(|o| {
            publish(
                o,
                &Publish {
                    version: Version::V311,
                    topic: "t",
                    payload: &payload,
                    qos: 0,
                    retain: false,
                    dup: false,
                    packet_id: 0,
                    message_expiry: None,
                    content_type: None,
                    payload_format: None,
                    response_topic: None,
                    user_property: None,
                },
            )
            .unwrap()
        });
        let mut m = Mqtt::default();
        assert_eq!(flow_read(&mut m, &packet, "publish:0").unwrap(), "publish");
        assert_eq!(text(call(&mut m, "field", &[ArgValue::Text("payload_length")], &[]).unwrap()), body.to_string());
    }
}

#[test]
fn the_assembler_rejects_misuse() {
    let mut a = Assembler::default();
    assert!(a.take().is_err());
    assert!(a.feed(&[]).is_err());
    assert_eq!(a.feed(&[0x30]).unwrap(), 1);
    assert!(a.feed(&[0, 0]).is_err());
    let mut a = Assembler::default();
    a.feed(&[0x30]).unwrap();
    for _ in 0..3 {
        assert_eq!(a.feed(&[0x80]).unwrap(), 1);
    }
    // A fourth continuation byte makes the length longer than four bytes.
    assert!(a.feed(&[0x80]).is_err());
    let mut a = Assembler::default();
    a.feed(&[0x30]).unwrap();
    assert_eq!(a.feed(&[0x02]).unwrap(), 2);
    assert!(a.feed(&[1]).is_err());
    // Bytes after a complete packet.
    let mut a = Assembler::default();
    a.feed(&[0xC0]).unwrap();
    assert_eq!(a.feed(&[0]).unwrap(), 0);
    assert_eq!(a.feed(&[]).unwrap(), 0);
    assert!(a.feed(&[1]).is_err());
    assert_eq!(a.take().unwrap(), [0xC0, 0x00]);
    // A fresh packet after take.
    assert_eq!(a.feed(&[0xD0]).unwrap(), 1);
}

#[test]
fn a_session_runs_between_two_values_at_every_qos() {
    for version in [4i64, 5] {
        let mut c = Mqtt::default();
        let mut b = Mqtt::default();
        let hello = call(
            &mut c,
            "connect",
            &[ArgValue::Text("mqttjs_00000001")],
            &[named("version", ArgValue::Int(version)), named("keepalive", ArgValue::Int(30))],
        )
        .unwrap();
        assert_eq!(flow_read(&mut b, &hello, "connect").unwrap(), "connect");
        assert_eq!(text(call(&mut b, "field", &[ArgValue::Text("version")], &[]).unwrap()), version.to_string());
        assert_eq!(text(call(&mut b, "field", &[ArgValue::Text("keepalive")], &[]).unwrap()), "30");
        let ack = call(&mut b, "connack", &[], &[]).unwrap();
        assert_eq!(flow_read(&mut c, &ack, "connack").unwrap(), "connack");

        // QoS 1: the identifier of the PUBACK comes from the PUBLISH that the broker read.
        let p1 = call(&mut c, "publish", &[ArgValue::Text("a/b"), ArgValue::Bytes(b"x")], &[named("qos", ArgValue::Int(1))]).unwrap();
        assert_eq!(flow_read(&mut b, &p1, "publish:1").unwrap(), "publish");
        let id = text(call(&mut b, "field", &[ArgValue::Text("packet_id")], &[]).unwrap());
        assert_eq!(id, "1");
        let pa = call(&mut b, "puback", &[], &[]).unwrap();
        assert_eq!(flow_read(&mut c, &pa, "puback").unwrap(), "puback");
        assert_eq!(text(call(&mut c, "field", &[ArgValue::Text("packet_id")], &[]).unwrap()), "1");

        // QoS 2: PUBLISH, PUBREC, PUBREL, PUBCOMP with one identifier.
        let p2 = call(&mut c, "publish", &[ArgValue::Text("a/b"), ArgValue::Bytes(b"y")], &[named("qos", ArgValue::Int(2))]).unwrap();
        assert_eq!(flow_read(&mut b, &p2, "publish:2").unwrap(), "publish");
        let rec = call(&mut b, "pubrec", &[], &[]).unwrap();
        assert_eq!(flow_read(&mut c, &rec, "pubrec").unwrap(), "pubrec");
        let rel = call(&mut c, "pubrel", &[], &[]).unwrap();
        assert_eq!(flow_read(&mut b, &rel, "pubrel").unwrap(), "pubrel");
        let comp = call(&mut b, "pubcomp", &[], &[]).unwrap();
        assert_eq!(flow_read(&mut c, &comp, "pubcomp").unwrap(), "pubcomp");
        assert_eq!(text(call(&mut c, "field", &[ArgValue::Text("packet_id")], &[]).unwrap()), "2");

        // SUBSCRIBE with two filters at QoS 2, granted at most 1.
        let sub = call(&mut c, "subscribe", &[ArgValue::Text("a/#,b/+")], &[named("qos", ArgValue::Int(2))]).unwrap();
        assert_eq!(flow_read(&mut b, &sub, "subscribe").unwrap(), "subscribe");
        let granted = call(&mut b, "granted", &[ArgValue::Int(1)], &[]).unwrap();
        assert_eq!(granted, b"1,1");
        let sa = call(&mut b, "suback", &[ArgValue::Bytes(&granted)], &[]).unwrap();
        assert_eq!(flow_read(&mut c, &sa, "suback").unwrap(), "suback");
        assert_eq!(text(call(&mut c, "field", &[ArgValue::Text("codes")], &[]).unwrap()), "1,1");

        let un = call(&mut c, "unsubscribe", &[ArgValue::Text("a/#,b/+")], &[]).unwrap();
        assert_eq!(flow_read(&mut b, &un, "unsubscribe").unwrap(), "unsubscribe");
        let ua = call(&mut b, "unsuback", &[], &[named("count", ArgValue::Int(2))]).unwrap();
        assert_eq!(flow_read(&mut c, &ua, "unsuback").unwrap(), "unsuback");

        let ping = call(&mut c, "pingreq", &[], &[]).unwrap();
        assert_eq!(flow_read(&mut b, &ping, "pingreq").unwrap(), "pingreq");
        let pong = call(&mut b, "pingresp", &[], &[]).unwrap();
        assert_eq!(flow_read(&mut c, &pong, "pingresp").unwrap(), "pingresp");
        let bye = call(&mut c, "disconnect", &[], &[]).unwrap();
        assert_eq!(flow_read(&mut b, &bye, "disconnect").unwrap(), "disconnect");
    }
}

#[test]
fn recv_fails_the_session_on_a_wrong_kind_or_qos() {
    let mut c = Mqtt::default();
    let mut b = Mqtt::default();
    let p = call(&mut c, "publish", &[ArgValue::Text("a"), ArgValue::Bytes(b"")], &[named("qos", ArgValue::Int(1))]).unwrap();
    let e = flow_read(&mut b, &p, "puback").unwrap_err();
    assert_eq!(e.kind, FnErrorKind::Protocol);
    let e = flow_read(&mut b, &p, "publish:0").unwrap_err();
    assert_eq!(e.kind, FnErrorKind::Protocol);
    // recv before the reads finished a packet.
    let e = call(&mut b, "recv", &[], &[]).unwrap_err();
    assert_eq!(e.kind, FnErrorKind::Protocol);
    // field before recv.
    assert_eq!(call(&mut Mqtt::default(), "field", &[ArgValue::Text("type")], &[]).unwrap_err().kind, FnErrorKind::InvalidArgument);
    // An acknowledgment without a packet to answer.
    assert_eq!(call(&mut Mqtt::default(), "puback", &[], &[]).unwrap_err().kind, FnErrorKind::InvalidArgument);
}

#[test]
fn packet_identifiers_count_up_and_wrap() {
    let mut m = Mqtt::default();
    let id = |m: &mut Mqtt| {
        let p = call(m, "subscribe", &[ArgValue::Text("a")], &[]).unwrap();
        u16::from_be_bytes([p[2], p[3]])
    };
    assert_eq!(id(&mut m), 1);
    assert_eq!(id(&mut m), 2);
    m.next_id = u16::MAX;
    assert_eq!(id(&mut m), 65_535);
    assert_eq!(id(&mut m), 1);
    // QoS 0 takes no identifier and leaves the counter alone.
    call(&mut m, "publish", &[ArgValue::Text("a"), ArgValue::Bytes(b"")], &[]).unwrap();
    assert_eq!(id(&mut m), 2);
    // An explicit identifier is checked and leaves the counter alone.
    let p = call(&mut m, "subscribe", &[ArgValue::Text("a")], &[named("packet_id", ArgValue::Int(900))]).unwrap();
    assert_eq!(u16::from_be_bytes([p[2], p[3]]), 900);
    for bad in [0, 65_536, -2] {
        assert!(call(&mut m, "subscribe", &[ArgValue::Text("a")], &[named("packet_id", ArgValue::Int(bad))]).is_err());
    }
}

#[test]
fn a_session_of_3_1_1_refuses_the_properties_of_5_0() {
    let mut m = Mqtt::default();
    call(&mut m, "connect", &[ArgValue::Text("c")], &[]).unwrap();
    let e = call(
        &mut m,
        "publish",
        &[ArgValue::Text("a"), ArgValue::Bytes(b"")],
        &[named("content_type", ArgValue::Text("text/plain"))],
    )
    .unwrap_err();
    assert_eq!(e.kind, FnErrorKind::InvalidArgument);
    assert!(call(&mut m, "connect", &[ArgValue::Text("c")], &[named("session_expiry", ArgValue::Int(5))]).is_err());
    // A broker of 3.1.1 leaves the defaults of the properties out and rejects a given one.
    let mut b = Mqtt::default();
    assert_eq!(call(&mut b, "connack", &[], &[]).unwrap(), [0x20, 0x02, 0x00, 0x00]);
    assert!(call(&mut b, "connack", &[], &[named("receive_max", ArgValue::Int(7))]).is_err());
    // A broker of 5.0 sends them.
    let mut b = Mqtt::default();
    b.version = Version::V5;
    let p = call(&mut b, "connack", &[], &[]).unwrap();
    let parsed = parse(Version::V5, &p).unwrap();
    assert_eq!(field(&parsed, "receive_maximum").unwrap(), b"20");
    assert_eq!(field(&parsed, "topic_alias_maximum").unwrap(), b"10");
}

#[test]
fn bad_arguments_fail_the_call_and_not_the_process() {
    let mut m = Mqtt::default();
    let bad: &[(&'static str, Vec<ArgValue<'_>>)] = &[
        ("connect", vec![ArgValue::Int(1)]),
        ("publish", vec![ArgValue::Text("a/+"), ArgValue::Bytes(b"")]),
        ("subscribe", vec![ArgValue::Text("a,,b")]),
        ("suback", vec![ArgValue::Text("x")]),
        ("varint", vec![ArgValue::Int(268_435_456)]),
        ("varint", vec![ArgValue::Int(-1)]),
        ("varint_value", vec![ArgValue::Bytes(&[0x80, 0x00])]),
        ("pace", vec![ArgValue::Int(0)]),
        ("recv", vec![ArgValue::Text("teapot")]),
        ("recv", vec![ArgValue::Text("connect:1")]),
        ("client_id", vec![ArgValue::Text("a-prefix-that-is-far-too-long")]),
        ("reading", vec![ArgValue::Text("a\"b")]),
        ("feed", vec![ArgValue::Bytes(&[1, 2])]),
    ];
    for (f, args) in bad {
        let e = call(&mut m, f, args, &[]).unwrap_err();
        assert!(
            matches!(e.kind, FnErrorKind::InvalidArgument | FnErrorKind::Protocol),
            "{f}: {e}"
        );
    }
    let e = call(&mut m, "teapot", &[], &[]).unwrap_err();
    assert_eq!(e.kind, FnErrorKind::Unsupported);
}

#[test]
fn varint_functions_agree_with_the_wire() {
    let mut m = Mqtt::default();
    for v in [0i64, 1, 127, 128, 16_383, 16_384, 2_097_151, 2_097_152, 268_435_455] {
        let b = call(&mut m, "varint", &[ArgValue::Int(v)], &[]).unwrap();
        assert_eq!(text(call(&mut m, "varint_value", &[ArgValue::Bytes(&b)], &[]).unwrap()), v.to_string());
    }
}

#[test]
fn client_ids_and_readings_come_from_the_generator() {
    let mut m = Mqtt::default();
    let mut rng = rand_pcg::Pcg64Mcg::seed_from_u64(1);
    let a = text(call_with(&mut m, &mut rng, "client_id", &[], &[]).unwrap());
    assert!(a.starts_with("mqttjs_") && a.len() == 15, "{a}");
    let b = text(call_with(&mut m, &mut rng, "client_id", &[], &[]).unwrap());
    assert_ne!(a, b);
    let r1 = text(call_with(&mut m, &mut rng, "reading", &[], &[]).unwrap());
    let r2 = text(call_with(&mut m, &mut rng, "reading", &[], &[]).unwrap());
    assert!(r1.contains("\"seq\":1,") && r2.contains("\"seq\":2,"), "{r1} {r2}");
    assert!(r1.starts_with("{\"device\":\"sensor-0412\"") && r1.ends_with('}'));
    // A new side starts its own sequence.
    let r = text(call_with(&mut Mqtt::default(), &mut rng, "reading", &[ArgValue::Text("gw-7")], &[]).unwrap());
    assert!(r.contains("\"device\":\"gw-7\"") && r.contains("\"seq\":1,"));
}

#[tokio::test(start_paused = true)]
async fn pace_keeps_the_schedule_when_a_sleep_wakes_late() {
    let mut m = Mqtt::default();
    let pace = |m: &mut Mqtt| {
        text(call(m, "pace", &[ArgValue::Int(2)], &[]).unwrap())
            .parse::<u64>()
            .unwrap()
    };
    assert_eq!(pace(&mut m), 2000);
    // The sleep wakes 1.5 ms late, so the next sleep is 1.5 ms shorter.
    tokio::time::advance(Duration::from_micros(2_001_500)).await;
    assert_eq!(pace(&mut m), 1999);
    // A late wake of more than a whole interval leaves no sleep at all.
    tokio::time::advance(Duration::from_secs(5)).await;
    assert_eq!(pace(&mut m), 0);
}
