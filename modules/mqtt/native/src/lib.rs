// SPDX-License-Identifier: GPL-3.0-only
//! Native template functions of the `mqtt` module.
//!
//! Decision D10 lets first party modules implement their template functions in
//! Rust. This crate implements the `mqtt` functions behind the [`TemplateFn`]
//! trait of `fps-flow`, so the interpreter renders
//! `{{ mqtt.publish(vars.topic, mqtt.reading(), qos=1) }}` without a
//! WebAssembly runtime. The functions write the control packets of MQTT 3.1.1
//! and 5.0 that a client and a broker exchange, which are CONNECT, CONNACK,
//! PUBLISH, the four acknowledgments of the QoS 1 and QoS 2 flows, SUBSCRIBE,
//! SUBACK, UNSUBSCRIBE, UNSUBACK, PINGREQ, PINGRESP, and DISCONNECT, and they
//! read the packets of the peer.
//!
//! # State
//!
//! One [`Mqtt`] value serves one session side. It holds the protocol version,
//! which `connect` sets on the client and `recv` sets on the broker when it
//! reads the CONNECT, the next packet identifier, the sequence number of the
//! sensor readings, the packet that the reader collects, the packet that
//! `recv` read last, and the schedule of `pace`. A packet identifier of -1
//! therefore means two things. A packet that opens an exchange (PUBLISH of QoS
//! above 0, SUBSCRIBE, UNSUBSCRIBE) takes the next identifier of this side,
//! and a packet that answers one takes the identifier of the packet that this
//! side read last.
//!
//! # Reading packets in a flow
//!
//! A packet has a variable length, so a flow reads it in steps. One `expect`
//! of one byte reads the packet type, and six `expect` actions of
//! `mqtt.feed(last)` bytes follow in a loop. Each hands the bytes of the read
//! before to the reader, which returns the size of the next read, one byte for
//! each byte of the Remaining Length (at most four), then the body, then 0 for
//! the reads that remain. A `set` action with `mqtt.recv('connack')` then
//! checks the packet against the standard and stores its fields, which
//! `mqtt.field` reads.
//!
//! ```yaml
//! - expect: { bytes: 1, timeout_ms: 5000 }
//! - loop:
//!     count: 6
//!     do:
//!       - expect: { bytes: "{{ mqtt.feed(last) }}", timeout_ms: 5000 }
//! - set: { got: "{{ mqtt.recv('puback') }}" }
//! ```
//!
//! # Keep alive
//!
//! `pace(keepalive)` returns the milliseconds until the next slot of the
//! schedule, as `sip_rtp.pace` does for RTP, so the pings of a long session
//! keep their schedule when a sleep wakes late.
//!
//! # Limits
//!
//! * The module writes the versions 3.1.1 and 5.0, and it reads no MQTT 3.1
//!   (protocol name `MQIsdp`).
//! * It writes no AUTH packet, no enhanced authentication, no subscription
//!   identifier, no topic alias, and no will properties.
//! * A SUBSCRIBE carries one set of options for all its filters.
//! * The values of the sensor readings are synthetic.
//!
//! [`FUNCTIONS`] is the catalog that `modules/mqtt/manifest.yaml` lists under
//! `functions`. The catalog test compares the two, so the manifest stays the
//! single catalog that the validator and the controller read.
#![forbid(unsafe_code)]
#![warn(missing_docs, clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod args;
pub mod catalog;
pub mod read;
#[cfg(test)]
mod tests;
pub mod wire;

use std::time::Duration;

use fps_flow::{FnCall, FnError, FnErrorKind, TemplateFn};
use rand::RngCore;

pub use catalog::{DefaultValue, FunctionSpec, ParamSpec, FUNCTIONS};

use args::Args;
use read::{Assembler, Packet};
use wire::{AckKind, Kind, Version, WireError};

/// Name of the module that this crate implements.
pub const MODULE_NAME: &str = "mqtt";

/// The template functions of the `mqtt` module for one session side.
#[derive(Clone, Debug, Default)]
pub struct Mqtt {
    version: Version,
    next_id: u16,
    seq: u64,
    assembler: Assembler,
    last: Option<Packet>,
    /// When the next keep alive slot is due, which `pace` advances.
    due: Option<tokio::time::Instant>,
}

fn invalid(message: impl Into<String>) -> FnError {
    FnError::new(FnErrorKind::InvalidArgument, message)
}

fn protocol(message: impl Into<String>) -> FnError {
    FnError::new(FnErrorKind::Protocol, message)
}

/// Maps a wire error that comes from the arguments of a call.
fn bad_arg(function: &str) -> impl Fn(WireError) -> FnError + '_ {
    move |e| invalid(format!("'{function}': {e}"))
}

/// Maps a wire error that comes from bytes of the peer.
fn bad_msg(function: &str) -> impl Fn(WireError) -> FnError + '_ {
    move |e| protocol(format!("'{function}': {e}"))
}

fn push(out: &mut Vec<u8>, text: &str) {
    out.extend_from_slice(text.as_bytes());
}

/// Reads an integer argument in `lo..=hi`.
fn ranged(a: &Args<'_>, function: &str, name: &str, lo: i64, hi: i64) -> Result<i64, FnError> {
    let v = a.int(name)?;
    if !(lo..=hi).contains(&v) {
        return Err(invalid(format!(
            "'{name}' of '{function}' must be between {lo} and {hi}, not {v}"
        )));
    }
    Ok(v)
}

/// Reads an integer argument that is -1 or in `lo..=hi`, and returns `None`
/// for -1.
fn optional(
    a: &Args<'_>,
    function: &str,
    name: &str,
    lo: i64,
    hi: i64,
) -> Result<Option<i64>, FnError> {
    let v = a.int(name)?;
    if v == catalog::AUTO {
        return Ok(None);
    }
    if !(lo..=hi).contains(&v) {
        return Err(invalid(format!(
            "'{name}' of '{function}' must be -1 or between {lo} and {hi}, not {v}"
        )));
    }
    Ok(Some(v))
}

/// Reads a text argument and returns `None` when it is empty.
fn nonempty<'a>(a: &Args<'a>, name: &str) -> Result<Option<&'a str>, FnError> {
    let t = a.text(name)?;
    Ok(if t.is_empty() { None } else { Some(t) })
}

/// Splits a comma separated list that must not hold an empty entry.
fn list<'a>(function: &str, name: &str, text: &'a str) -> Result<Vec<&'a str>, FnError> {
    let items: Vec<&str> = text.split(',').collect();
    if items.iter().any(|i| i.is_empty()) {
        return Err(invalid(format!(
            "'{name}' of '{function}' must hold comma separated entries without an empty one, not '{text}'"
        )));
    }
    Ok(items)
}

fn codes(function: &str, name: &str, text: &str) -> Result<Vec<u8>, FnError> {
    list(function, name, text)?
        .into_iter()
        .map(|c| {
            c.trim().parse::<u8>().map_err(|_| {
                invalid(format!(
                    "'{name}' of '{function}' lists '{c}', which is not a code from 0 to 255"
                ))
            })
        })
        .collect()
}

/// Reads a bytes argument that must be text, because a nested call returns bytes.
fn utf8<'a>(function: &str, name: &str, b: &'a [u8]) -> Result<&'a str, FnError> {
    std::str::from_utf8(b).map_err(|_| {
        invalid(format!(
            "'{name}' of '{function}' must be text, and its bytes are not UTF-8"
        ))
    })
}

fn id_arg(function: &str, v: i64) -> Result<u16, FnError> {
    u16::try_from(v)
        .ok()
        .filter(|id| *id > 0)
        .ok_or_else(|| {
            invalid(format!(
                "'packet_id' of '{function}' must be -1 or between 1 and 65535, not {v}"
            ))
        })
}

impl Mqtt {
    /// The next packet identifier of this side, from 1 to 65535 and around.
    fn allocate_id(&mut self) -> u16 {
        let id = self.next_id.max(1);
        self.next_id = if id == u16::MAX { 1 } else { id + 1 };
        id
    }

    /// The identifier of a packet that opens an exchange.
    fn new_id(&mut self, a: &Args<'_>, function: &str) -> Result<u16, FnError> {
        match a.int("packet_id")? {
            catalog::AUTO => Ok(self.allocate_id()),
            v => id_arg(function, v),
        }
    }

    /// The identifier of a packet that answers the packet read last.
    fn answered_id(&self, a: &Args<'_>, function: &str) -> Result<u16, FnError> {
        match a.int("packet_id")? {
            catalog::AUTO => self
                .last
                .as_ref()
                .and_then(|p| p.packet_id)
                .ok_or_else(|| {
                    invalid(format!(
                        "'{function}' answers the packet that recv read last, and that packet has no identifier"
                    ))
                }),
            v => id_arg(function, v),
        }
    }

    fn ack(
        &self,
        function: &str,
        kind: AckKind,
        call: &FnCall<'_>,
        out: &mut Vec<u8>,
    ) -> Result<(), FnError> {
        let spec = catalog::FUNCTIONS
            .iter()
            .find(|f| f.name == function)
            .ok_or_else(|| FnError::new(FnErrorKind::Internal, "an acknowledgment without a spec"))?;
        let a = args::bind(spec, &call.args)?;
        let id = self.answered_id(&a, function)?;
        let reason = ranged(&a, function, "reason", 0, 255)? as u8;
        wire::ack(out, self.version, kind, id, reason, None).map_err(bad_arg(function))
    }

    fn connect(&mut self, call: &FnCall<'_>, out: &mut Vec<u8>) -> Result<(), FnError> {
        let a = args::bind(&catalog::CONNECT_SPEC, &call.args)?;
        let f = "connect";
        let version = match ranged(&a, f, "version", 4, 5)? {
            5 => Version::V5,
            _ => Version::V311,
        };
        let password = nonempty(&a, "password")?;
        let will_topic = nonempty(&a, "will_topic")?;
        let will = will_topic.map(|topic| wire::Will {
            topic,
            payload: a.bytes("will_payload").unwrap_or_default(),
            qos: 0,
            retain: false,
        });
        let will = match will {
            Some(mut w) => {
                w.qos = ranged(&a, f, "will_qos", 0, 2)? as u8;
                w.retain = a.boolean("will_retain")?;
                Some(w)
            }
            None => None,
        };
        wire::connect(
            out,
            &wire::Connect {
                version,
                client_id: utf8(f, "client_id", a.bytes("client_id")?)?,
                keepalive: ranged(&a, f, "keepalive", 0, 65_535)? as u16,
                clean: a.boolean("clean")?,
                user: nonempty(&a, "user")?,
                password: password.map(str::as_bytes),
                will,
                session_expiry: optional(&a, f, "session_expiry", 0, i64::from(u32::MAX))?
                    .map(|v| v as u32),
                receive_max: optional(&a, f, "receive_max", 1, 65_535)?.map(|v| v as u16),
                topic_alias_max: optional(&a, f, "topic_alias_max", 0, 65_535)?.map(|v| v as u16),
            },
        )
        .map_err(bad_arg(f))?;
        self.version = version;
        Ok(())
    }

    fn connack(&self, call: &FnCall<'_>, out: &mut Vec<u8>) -> Result<(), FnError> {
        let a = args::bind(&catalog::CONNACK_SPEC, &call.args)?;
        let f = "connack";
        let v5 = self.version.is_v5();
        // The defaults describe a broker of MQTT 5.0. A session of 3.1.1 has no
        // properties, so the defaults stay out of its packet.
        let prop = |name: &str, lo: i64, hi: i64| -> Result<Option<i64>, FnError> {
            let v = optional(&a, f, name, lo, hi)?;
            Ok(if v5 { v } else { None })
        };
        let given_in_311 = [
            "receive_max",
            "topic_alias_max",
            "server_keepalive",
            "max_qos",
            "session_expiry",
        ]
        .into_iter()
        .any(|n| {
            let default = catalog::CONNACK_SPEC
                .params
                .iter()
                .find(|p| p.name == n)
                .and_then(|p| p.default);
            matches!((a.int(n), default), (Ok(v), Some(DefaultValue::Int(d))) if v != d)
        });
        if !v5 && (given_in_311 || !a.text("assigned_client_id")?.is_empty()) {
            return Err(invalid(
                "'connack' sets a property, which needs MQTT 5.0, and the session uses 3.1.1",
            ));
        }
        wire::connack(
            out,
            &wire::Connack {
                version: self.version,
                session_present: a.boolean("session_present")?,
                code: ranged(&a, f, "code", 0, 255)? as u8,
                session_expiry: prop("session_expiry", 0, i64::from(u32::MAX))?.map(|v| v as u32),
                receive_max: prop("receive_max", 1, 65_535)?.map(|v| v as u16),
                topic_alias_max: prop("topic_alias_max", 0, 65_535)?.map(|v| v as u16),
                server_keepalive: prop("server_keepalive", 0, 65_535)?.map(|v| v as u16),
                assigned_client_id: if v5 { nonempty(&a, "assigned_client_id")? } else { None },
                max_qos: prop("max_qos", 0, 1)?.map(|v| v as u8),
                retain_available: None,
            },
        )
        .map_err(bad_arg(f))
    }

    fn publish(&mut self, call: &FnCall<'_>, out: &mut Vec<u8>) -> Result<(), FnError> {
        let a = args::bind(&catalog::PUBLISH_SPEC, &call.args)?;
        let f = "publish";
        let qos = ranged(&a, f, "qos", 0, 2)? as u8;
        // QoS 0 carries no identifier, so a given one is not an error and the
        // counter does not advance.
        let packet_id = if qos > 0 { self.new_id(&a, f)? } else { 0 };
        let user_property = match nonempty(&a, "user_property")? {
            Some(t) => Some(t.split_once('=').ok_or_else(|| {
                invalid(format!(
                    "'user_property' of 'publish' must read name=value, not '{t}'"
                ))
            })?),
            None => None,
        };
        wire::publish(
            out,
            &wire::Publish {
                version: self.version,
                topic: a.text("topic")?,
                payload: a.bytes("payload")?,
                qos,
                retain: a.boolean("retain")?,
                dup: a.boolean("dup")?,
                packet_id,
                message_expiry: optional(&a, f, "message_expiry", 0, i64::from(u32::MAX))?
                    .map(|v| v as u32),
                content_type: nonempty(&a, "content_type")?,
                payload_format: optional(&a, f, "payload_format", 0, 1)?.map(|v| v as u8),
                response_topic: nonempty(&a, "response_topic")?,
                user_property,
            },
        )
        .map_err(bad_arg(f))
    }

    fn subscribe(&mut self, call: &FnCall<'_>, out: &mut Vec<u8>) -> Result<(), FnError> {
        let a = args::bind(&catalog::SUBSCRIBE_SPEC, &call.args)?;
        let f = "subscribe";
        let qos = ranged(&a, f, "qos", 0, 2)? as u8;
        let handling = ranged(&a, f, "retain_handling", 0, 2)? as u8;
        let (no_local, rap) = (a.boolean("no_local")?, a.boolean("retain_as_published")?);
        let filters = list(f, "filters", a.text("filters")?)?;
        let subs: Vec<wire::Subscription<'_>> = filters
            .iter()
            .map(|filter| wire::Subscription {
                filter,
                qos,
                no_local,
                retain_as_published: rap,
                retain_handling: handling,
            })
            .collect();
        let id = self.new_id(&a, f)?;
        wire::subscribe(out, self.version, id, &subs).map_err(bad_arg(f))
    }

    fn recv(&mut self, call: &FnCall<'_>, out: &mut Vec<u8>) -> Result<(), FnError> {
        let a = args::bind(&catalog::RECV_SPEC, &call.args)?;
        let expect = a.text("expect")?;
        let (name, qos) = match expect.split_once(':') {
            Some((n, q)) => (n, Some(q)),
            None => (expect, None),
        };
        let want = match name {
            "any" => None,
            n => Some(Kind::from_name(n).ok_or_else(|| {
                invalid(format!("'expect' of 'recv' names '{n}', which is not a packet kind"))
            })?),
        };
        let want_qos = match qos {
            None => None,
            Some(q) if want == Some(Kind::Publish) => Some(
                q.parse::<u8>()
                    .ok()
                    .filter(|q| *q <= 2)
                    .ok_or_else(|| invalid(format!("'expect' of 'recv' names the QoS '{q}'")))?,
            ),
            Some(_) => return Err(invalid("'expect' of 'recv' takes a QoS only with publish")),
        };
        let packet = self.assembler.take().map_err(bad_msg("recv"))?;
        let parsed = read::parse(self.version, &packet).map_err(bad_msg("recv"))?;
        if let Some(k) = want {
            if parsed.kind != k {
                return Err(protocol(format!(
                    "'recv' expected {}, and the peer sent {}",
                    k.name(),
                    parsed.kind.name()
                )));
            }
        }
        if let Some(q) = want_qos {
            if parsed.qos != q {
                return Err(protocol(format!(
                    "'recv' expected a PUBLISH of QoS {q}, and the peer sent QoS {}",
                    parsed.qos
                )));
            }
        }
        if parsed.kind == Kind::Connect {
            self.version = parsed.version;
        }
        push(out, parsed.kind.name());
        self.last = Some(parsed);
        Ok(())
    }

    fn pace(&mut self, call: &FnCall<'_>, out: &mut Vec<u8>) -> Result<(), FnError> {
        let a = args::bind(&catalog::PACE_SPEC, &call.args)?;
        let keepalive = ranged(&a, "pace", "keepalive", 1, 65_535)? as u64;
        let step = Duration::from_secs(keepalive);
        let now = tokio::time::Instant::now();
        // The first call comes right after the CONNACK. Every later call sleeps
        // until the next slot of the schedule, so the sleeps do not add their
        // rounding up to a drift, and a late wake shortens the next sleep.
        let due = self.due.map_or(now + step, |d| d + step);
        self.due = Some(due);
        let wait = due.saturating_duration_since(now);
        push(out, &((wait.as_micros() + 500) / 1000).to_string());
        Ok(())
    }

    fn random_hex(rng: &mut dyn RngCore) -> String {
        format!("{:08x}", rng.next_u32())
    }

    fn reading(&mut self, call: &FnCall<'_>, rng: &mut dyn RngCore, out: &mut Vec<u8>) -> Result<(), FnError> {
        let a = args::bind(&catalog::READING_SPEC, &call.args)?;
        let device = a.text("device")?;
        let ok = !device.is_empty()
            && device.len() <= 64
            && device
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
        if !ok {
            return Err(invalid(format!(
                "'device' of 'reading' must be 1 to 64 letters, digits, '-' or '_', not '{device}'"
            )));
        }
        self.seq += 1;
        let (t, h, b, r) = (
            rng.next_u32() % 160,
            rng.next_u32() % 600,
            2800 + rng.next_u32() % 400,
            rng.next_u32() % 40,
        );
        let text = format!(
            "{{\"device\":\"{device}\",\"seq\":{},\"temp_c\":{}.{},\"humidity\":{}.{},\"battery_mv\":{b},\"rssi\":-{}}}",
            self.seq,
            15 + t / 10,
            t % 10,
            30 + h / 10,
            h % 10,
            50 + r
        );
        push(out, &text);
        Ok(())
    }
}

impl TemplateFn for Mqtt {
    fn render(
        &mut self,
        call: &FnCall<'_>,
        rng: &mut dyn RngCore,
        out: &mut Vec<u8>,
    ) -> Result<(), FnError> {
        if call.module != MODULE_NAME {
            return Err(FnError::new(
                FnErrorKind::Unsupported,
                format!("module '{}' is not mqtt", call.module),
            ));
        }
        let function = call.function;
        match function {
            "connect" => self.connect(call, out)?,
            "connack" => self.connack(call, out)?,
            "publish" => self.publish(call, out)?,
            "puback" => self.ack(function, AckKind::Puback, call, out)?,
            "pubrec" => self.ack(function, AckKind::Pubrec, call, out)?,
            "pubrel" => self.ack(function, AckKind::Pubrel, call, out)?,
            "pubcomp" => self.ack(function, AckKind::Pubcomp, call, out)?,
            "subscribe" => self.subscribe(call, out)?,
            "suback" => {
                let a = args::bind(&catalog::SUBACK_SPEC, &call.args)?;
                let id = self.answered_id(&a, function)?;
                let cs = codes(function, "codes", utf8(function, "codes", a.bytes("codes")?)?)?;
                wire::suback(out, self.version, id, &cs).map_err(bad_arg(function))?;
            }
            "unsubscribe" => {
                let a = args::bind(&catalog::UNSUBSCRIBE_SPEC, &call.args)?;
                let filters = list(function, "filters", a.text("filters")?)?;
                let id = self.new_id(&a, function)?;
                wire::unsubscribe(out, self.version, id, &filters).map_err(bad_arg(function))?;
            }
            "unsuback" => {
                let a = args::bind(&catalog::UNSUBACK_SPEC, &call.args)?;
                let id = self.answered_id(&a, function)?;
                let count = ranged(&a, function, "count", 1, 1000)? as usize;
                let reason = ranged(&a, function, "reason", 0, 255)? as u8;
                wire::unsuback(out, self.version, id, &vec![reason; count])
                    .map_err(bad_arg(function))?;
            }
            "pingreq" => {
                args::bind(&catalog::PINGREQ_SPEC, &call.args)?;
                wire::pingreq(out);
            }
            "pingresp" => {
                args::bind(&catalog::PINGRESP_SPEC, &call.args)?;
                wire::pingresp(out);
            }
            "disconnect" => {
                let a = args::bind(&catalog::DISCONNECT_SPEC, &call.args)?;
                let reason = ranged(&a, function, "reason", 0, 255)? as u8;
                wire::disconnect(out, self.version, reason, nonempty(&a, "reason_string")?)
                    .map_err(bad_arg(function))?;
            }
            "varint" => {
                let a = args::bind(&catalog::VARINT_SPEC, &call.args)?;
                let v = ranged(&a, function, "value", 0, i64::from(wire::MAX_REMAINING_LENGTH))?;
                wire::put_varint(out, v as u32).map_err(bad_arg(function))?;
            }
            "varint_value" => {
                let a = args::bind(&catalog::VARINT_VALUE_SPEC, &call.args)?;
                let data = a.bytes("data")?;
                let (v, n) = wire::get_varint(data).map_err(bad_msg(function))?;
                if n != data.len() {
                    return Err(protocol(
                        "'varint_value' reads a variable byte integer, and bytes follow it",
                    ));
                }
                push(out, &v.to_string());
            }
            "client_id" => {
                let a = args::bind(&catalog::CLIENT_ID_SPEC, &call.args)?;
                let prefix = a.text("prefix")?;
                if prefix.len() > 15 || prefix.contains('\0') {
                    return Err(invalid(
                        "'prefix' of 'client_id' must hold at most 15 bytes and no null character",
                    ));
                }
                push(out, prefix);
                push(out, &Self::random_hex(rng));
            }
            "reading" => self.reading(call, rng, out)?,
            "feed" => {
                let a = args::bind(&catalog::FEED_SPEC, &call.args)?;
                let n = self.assembler.feed(a.bytes("data")?).map_err(bad_msg(function))?;
                push(out, &n.to_string());
            }
            "recv" => self.recv(call, out)?,
            "field" => {
                let a = args::bind(&catalog::FIELD_SPEC, &call.args)?;
                let p = self.last.as_ref().ok_or_else(|| {
                    invalid("'field' reads the packet that recv read last, and recv read none")
                })?;
                out.extend_from_slice(
                    &read::field(p, a.text("name")?).map_err(bad_msg(function))?,
                );
            }
            "granted" => {
                let a = args::bind(&catalog::GRANTED_SPEC, &call.args)?;
                let max = ranged(&a, function, "max_qos", 0, 2)? as u8;
                let p = self
                    .last
                    .as_ref()
                    .filter(|p| p.kind == Kind::Subscribe)
                    .ok_or_else(|| {
                        invalid("'granted' needs a SUBSCRIBE as the packet that recv read last")
                    })?;
                let granted: Vec<String> =
                    p.filters.iter().map(|f| f.qos().min(max).to_string()).collect();
                push(out, &granted.join(","));
            }
            "pace" => self.pace(call, out)?,
            other => {
                return Err(FnError::new(
                    FnErrorKind::Unsupported,
                    format!("{MODULE_NAME} has no function '{other}'"),
                ))
            }
        }
        Ok(())
    }
}
