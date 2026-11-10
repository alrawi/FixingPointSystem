// SPDX-License-Identifier: GPL-3.0-only
//! Native template functions of the `sip_rtp` module.
//!
//! Decision D10 lets first party modules implement their template functions in
//! Rust. This crate implements the `sip_rtp` functions behind the
//! [`TemplateFn`] trait of `fps-flow`, so the interpreter renders
//! `{{ sip_rtp.invite(server=vars.server_ip, client=vars.client_ip, rtp_port=vars.offer_port) }}`
//! without a WebAssembly runtime. The functions write the messages of one
//! call, which are the INVITE with an SDP offer (RFC 3261, RFC 8866), the
//! provisional and final responses with an SDP answer (RFC 3264), the ACK, the
//! BYE, and the RTP packets of the media stream (RFC 3550, RFC 3551). They
//! read back what the other side announced, which is the audio port and the
//! codec of a description.
//!
//! The functions `invite`, `response`, `ack`, `bye`, `port`, `sdp_port`, and
//! `codec` keep no state. The function `rtp` does: a [`SipRtp`] value holds the
//! sequence number, the timestamp, and the synchronization source of the
//! stream that one flow side sends, so the engine must give every flow side
//! its own value, as the fixture peer and the tests do.
//!
//! [`FUNCTIONS`] is the catalog that `modules/sip_rtp/manifest.yaml` lists
//! under `functions`. The catalog test compares the two, so the manifest stays
//! the single catalog that the validator and the controller read.
#![forbid(unsafe_code)]
#![warn(missing_docs, clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod args;
pub mod catalog;
mod codec;
mod rtp;
mod sdp;
mod sip;

use fps_flow::{FnCall, FnError, FnErrorKind, TemplateFn};
use rand::RngCore;

pub use catalog::{DefaultValue, FunctionSpec, ParamSpec, DEFAULT_ACCEPT, FUNCTIONS};

/// Name of the module that this crate implements.
pub const MODULE_NAME: &str = "sip_rtp";

/// The smallest and the largest packet time that the module writes, in
/// milliseconds. Packet times are multiples of ten, which every codec of the
/// module can frame.
pub const PTIME_RANGE: std::ops::RangeInclusive<i64> = 10..=120;

/// The template functions of the `sip_rtp` module for one flow side.
#[derive(Clone, Debug, Default)]
pub struct SipRtp {
    stream: Option<rtp::Stream>,
    /// When the next RTP packet is due, which `pace` advances by one packet time.
    due: Option<tokio::time::Instant>,
}

fn invalid(message: String) -> FnError {
    FnError::new(FnErrorKind::InvalidArgument, message)
}

fn push(out: &mut Vec<u8>, text: &str) {
    out.extend_from_slice(text.as_bytes());
}

fn port_arg(function: &str, name: &str, port: i64) -> Result<u16, FnError> {
    u16::try_from(port).ok().filter(|p| *p > 0).ok_or_else(|| {
        invalid(format!(
            "'{name}' of '{function}' must lie between 1 and 65535, not {port}"
        ))
    })
}

fn ptime_arg(function: &str, ptime: i64) -> Result<u32, FnError> {
    if PTIME_RANGE.contains(&ptime) && ptime % 10 == 0 {
        Ok(ptime as u32)
    } else {
        Err(invalid(format!(
            "'ptime' of '{function}' must be a multiple of 10 between {} and {}, not {ptime}",
            PTIME_RANGE.start(),
            PTIME_RANGE.end()
        )))
    }
}

fn host_arg<'a>(function: &str, name: &str, host: &'a str) -> Result<&'a str, FnError> {
    let ok = !host.is_empty()
        && host.len() <= 253
        && host
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-' || b == b':');
    if ok {
        Ok(host)
    } else {
        Err(invalid(format!(
            "'{name}' of '{function}' must be an address or a host name, not '{host}'"
        )))
    }
}

fn codecs_arg(function: &str, list: &str) -> Result<Vec<&'static codec::Codec>, FnError> {
    codec::parse_list(function, list).map_err(invalid)
}

fn sip_error(function: &str, e: String) -> FnError {
    invalid(format!("'{function}': {e}"))
}

impl TemplateFn for SipRtp {
    fn render(
        &mut self,
        call: &FnCall<'_>,
        rng: &mut dyn RngCore,
        out: &mut Vec<u8>,
    ) -> Result<(), FnError> {
        if call.module != MODULE_NAME {
            return Err(FnError::new(
                FnErrorKind::Unsupported,
                format!("module '{}' is not {MODULE_NAME}", call.module),
            ));
        }
        match call.function {
            "port" => {
                let a = args::bind(&catalog::PORT, &call.args)?;
                let lo = port_arg("port", "min", a.int("min")?)?;
                let hi = port_arg("port", "max", a.int("max")?)?;
                // The first even port at or above `min` and the last even port at or below `max`.
                let first = lo + lo % 2;
                let last = hi - hi % 2;
                if first > last {
                    return Err(invalid(format!(
                        "'port' has no even port between {lo} and {hi}"
                    )));
                }
                let slots = u32::from((last - first) / 2) + 1;
                let n = rng.next_u32() % slots;
                push(out, &(u32::from(first) + 2 * n).to_string());
            }
            "invite" => {
                let a = args::bind(&catalog::INVITE, &call.args)?;
                let codecs = codecs_arg("invite", a.text("codecs")?)?;
                if codecs.is_empty() {
                    return Err(invalid("'codecs' of 'invite' lists no codec".into()));
                }
                let p = sip::InviteParams {
                    server: host_arg("invite", "server", a.text("server")?)?,
                    client: host_arg("invite", "client", a.text("client")?)?,
                    rtp_port: port_arg("invite", "rtp_port", a.int("rtp_port")?)?,
                    codecs,
                    ptime: ptime_arg("invite", a.int("ptime")?)?,
                    sip_port: port_arg("invite", "sip_port", a.int("sip_port")?)?,
                };
                push(out, &sip::invite(&p, rng));
            }
            "response" => {
                let a = args::bind(&catalog::RESPONSE, &call.args)?;
                let code = u16::try_from(a.int("code")?)
                    .map_err(|_| invalid("'code' of 'response' is not a status code".into()))?;
                let server = a.text("server")?;
                if !server.is_empty() {
                    host_arg("response", "server", server)?;
                }
                let rtp_port = a.int("rtp_port")?;
                let p = sip::ResponseParams {
                    code,
                    server,
                    rtp_port: if rtp_port == 0 {
                        0
                    } else {
                        port_arg("response", "rtp_port", rtp_port)?
                    },
                    accept: codecs_arg("response", a.text("accept")?)?,
                    sip_port: port_arg("response", "sip_port", a.int("sip_port")?)?,
                };
                push(
                    out,
                    &sip::response(a.bytes("request")?, &p)
                        .map_err(|e| sip_error("response", e))?,
                );
            }
            "ack" | "bye" => {
                let bye = call.function == "bye";
                let spec = if bye { &catalog::BYE } else { &catalog::ACK };
                let a = args::bind(spec, &call.args)?;
                push(
                    out,
                    &sip::in_dialog(a.bytes("response")?, bye, rng)
                        .map_err(|e| sip_error(call.function, e))?,
                );
            }
            "sdp_port" => {
                let a = args::bind(&catalog::SDP_PORT, &call.args)?;
                let m = sip::Message::parse(a.bytes("message")?)
                    .map_err(|e| sip_error("sdp_port", e))?;
                let sdp = sdp::Sdp::parse(m.body).map_err(|e| sip_error("sdp_port", e))?;
                push(out, &sdp.port.to_string());
            }
            "codec" => {
                let a = args::bind(&catalog::CODEC, &call.args)?;
                let m =
                    sip::Message::parse(a.bytes("message")?).map_err(|e| sip_error("codec", e))?;
                let sdp = sdp::Sdp::parse(m.body).map_err(|e| sip_error("codec", e))?;
                let accept = codecs_arg("codec", a.text("accept")?)?;
                let chosen = sip::select(&sdp, &accept).map_err(|e| sip_error("codec", e))?;
                push(out, chosen.codec.name);
            }
            "rtp" => {
                let a = args::bind(&catalog::RTP, &call.args)?;
                let name = a.text("codec")?;
                let codec = codec::Codec::by_name(name)
                    .ok_or_else(|| invalid(format!("'{name}' is not a codec of this module")))?;
                let ptime = ptime_arg("rtp", a.int("ptime")?)?;
                let stream = self.stream.get_or_insert_with(|| rtp::Stream::start(rng));
                stream.next(codec, codec.pt, ptime, rng, out);
            }
            "packets" => {
                let a = args::bind(&catalog::PACKETS, &call.args)?;
                let talk = a.int("talk_ms")?;
                if !(1..=86_400_000).contains(&talk) {
                    return Err(invalid(format!(
                        "'talk_ms' of 'packets' must lie between 1 and 86400000, not {talk}"
                    )));
                }
                let ptime = i64::from(ptime_arg("packets", a.int("ptime")?)?);
                push(out, &(talk / ptime).max(1).to_string());
            }
            "pace" => {
                let a = args::bind(&catalog::PACE, &call.args)?;
                let ptime = u64::from(ptime_arg("pace", a.int("ptime")?)?);
                let step = std::time::Duration::from_millis(ptime);
                let now = tokio::time::Instant::now();
                // The first call comes right after the first packet. Every later call
                // sleeps until the next slot of the schedule, so the sleeps do not add
                // their rounding up to a drift, and a late wake shortens the next sleep.
                let due = self.due.map_or(now + step, |d| d + step);
                self.due = Some(due);
                let wait = due.saturating_duration_since(now);
                let ms = (wait.as_micros() + 500) / 1000;
                push(out, &ms.to_string());
            }
            "hangup_ms" => {
                let a = args::bind(&catalog::HANGUP_MS, &call.args)?;
                let talk = a.int("talk_ms")?;
                if !(1..=86_400_000).contains(&talk) {
                    return Err(invalid(format!(
                        "'talk_ms' of 'hangup_ms' must lie between 1 and 86400000, not {talk}"
                    )));
                }
                // The media loop sleeps a little longer than the packet time at every step, so
                // the BYE waits for the talk time, a tenth more, and a fixed margin.
                push(out, &(talk + talk / 10 + 200).to_string());
            }
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

#[cfg(test)]
mod tests {
    use super::*;
    use fps_flow::function::NamedArg;
    use fps_flow::{ArgValue, CallArgs};

    fn call(
        f: &mut SipRtp,
        function: &'static str,
        positional: &[ArgValue<'_>],
        named: &[NamedArg<'_>],
    ) -> Result<Vec<u8>, FnError> {
        let mut rng = rand_pcg::Pcg64Mcg::new(42);
        call_with(f, &mut rng, function, positional, named)
    }

    fn call_with(
        f: &mut SipRtp,
        rng: &mut rand_pcg::Pcg64Mcg,
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
        f.render(&c, rng, &mut out).map(|()| out)
    }

    fn text(v: Vec<u8>) -> String {
        String::from_utf8(v).unwrap()
    }

    #[test]
    fn rejects_other_modules_and_functions() {
        let mut rng = rand_pcg::Pcg64Mcg::new(1);
        let c = FnCall {
            module: "dns",
            function: "port",
            args: CallArgs {
                positional: &[],
                named: &[],
            },
        };
        let err = SipRtp::default()
            .render(&c, &mut rng, &mut Vec::new())
            .unwrap_err();
        assert_eq!(err.kind, FnErrorKind::Unsupported);
        let err = call(&mut SipRtp::default(), "teapot", &[], &[]).unwrap_err();
        assert_eq!(err.kind, FnErrorKind::Unsupported);
    }

    #[test]
    fn port_is_even_and_inside_the_range() {
        let mut f = SipRtp::default();
        let mut rng = rand_pcg::Pcg64Mcg::new(9);
        let mut seen = std::collections::BTreeSet::new();
        for _ in 0..300 {
            let p = text(
                call_with(
                    &mut f,
                    &mut rng,
                    "port",
                    &[ArgValue::Int(16385), ArgValue::Int(16400)],
                    &[],
                )
                .unwrap(),
            )
            .parse::<u32>()
            .unwrap();
            assert!(p % 2 == 0 && (16386..=16400).contains(&p), "{p}");
            seen.insert(p);
        }
        assert_eq!(seen.len(), 8, "every even port of the range comes up");
        let err = call(
            &mut f,
            "port",
            &[ArgValue::Int(16385), ArgValue::Int(16385)],
            &[],
        )
        .unwrap_err();
        assert_eq!(err.kind, FnErrorKind::InvalidArgument);
    }

    #[test]
    fn the_dialog_functions_chain_into_a_call() {
        let mut f = SipRtp::default();
        let invite = call(
            &mut f,
            "invite",
            &[],
            &[
                NamedArg {
                    name: "server",
                    value: ArgValue::Text("192.0.2.20"),
                },
                NamedArg {
                    name: "client",
                    value: ArgValue::Text("192.0.2.10"),
                },
                NamedArg {
                    name: "rtp_port",
                    value: ArgValue::Int(16384),
                },
                NamedArg {
                    name: "codecs",
                    value: ArgValue::Text("opus,PCMU"),
                },
            ],
        )
        .unwrap();
        let ok = call(
            &mut f,
            "response",
            &[ArgValue::Bytes(&invite), ArgValue::Int(200)],
            &[
                NamedArg {
                    name: "server",
                    value: ArgValue::Text("192.0.2.20"),
                },
                NamedArg {
                    name: "rtp_port",
                    value: ArgValue::Int(20000),
                },
            ],
        )
        .unwrap();
        assert_eq!(
            text(call(&mut f, "sdp_port", &[ArgValue::Bytes(&invite)], &[]).unwrap()),
            "16384"
        );
        assert_eq!(
            text(call(&mut f, "sdp_port", &[ArgValue::Bytes(&ok)], &[]).unwrap()),
            "20000"
        );
        // The offer lists opus first and the server accepts it, so both sides pick opus.
        let accept = [NamedArg {
            name: "accept",
            value: ArgValue::Text("opus,PCMA"),
        }];
        assert_eq!(
            text(call(&mut f, "codec", &[ArgValue::Bytes(&invite)], &accept).unwrap()),
            "opus"
        );
        assert_eq!(
            text(call(&mut f, "codec", &[ArgValue::Bytes(&ok)], &[]).unwrap()),
            "opus"
        );
        let ack = call(&mut f, "ack", &[ArgValue::Bytes(&ok)], &[]).unwrap();
        let bye = call(&mut f, "bye", &[ArgValue::Bytes(&ok)], &[]).unwrap();
        assert!(ack.starts_with(b"ACK sip:"));
        assert!(bye.starts_with(b"BYE sip:"));
    }

    #[test]
    fn an_opus_packet_starts_with_a_valid_table_of_contents_byte() {
        let args = [NamedArg {
            name: "codec",
            value: ArgValue::Text("opus"),
        }];
        let mut f = SipRtp::default();
        let mut rng = rand_pcg::Pcg64Mcg::new(5);
        for _ in 0..50 {
            let p = call_with(&mut f, &mut rng, "rtp", &[], &args).unwrap();
            // SILK wideband, 20 ms, mono, one frame (code 0).
            assert_eq!(p.get(rtp::HEADER_LEN), Some(&0x48));
        }
    }

    #[test]
    fn the_stream_state_belongs_to_the_value() {
        let args = [NamedArg {
            name: "codec",
            value: ArgValue::Text("PCMU"),
        }];
        let mut a = SipRtp::default();
        let mut rng = rand_pcg::Pcg64Mcg::new(3);
        let p1 = call_with(&mut a, &mut rng, "rtp", &[], &args).unwrap();
        let p2 = call_with(&mut a, &mut rng, "rtp", &[], &args).unwrap();
        let (h1, h2) = (rtp::parse(&p1).unwrap(), rtp::parse(&p2).unwrap());
        assert!(h1.marker && !h2.marker);
        assert_eq!(h2.seq, h1.seq.wrapping_add(1));
        assert_eq!(h2.ssrc, h1.ssrc);
        // A fresh value starts a new stream.
        let mut b = SipRtp::default();
        let mut rng2 = rand_pcg::Pcg64Mcg::new(4);
        let q = call_with(&mut b, &mut rng2, "rtp", &[], &args).unwrap();
        assert_ne!(rtp::parse(&q).unwrap().ssrc, h1.ssrc);
    }

    #[tokio::test(start_paused = true)]
    async fn pace_keeps_the_schedule_when_a_sleep_wakes_late() {
        let mut f = SipRtp::default();
        let pace = |f: &mut SipRtp| {
            text(call(f, "pace", &[ArgValue::Int(20)], &[]).unwrap())
                .parse::<u64>()
                .unwrap()
        };
        assert_eq!(pace(&mut f), 20);
        // The sleep wakes 1.5 ms late, so the next sleep is 1.5 ms shorter.
        tokio::time::advance(std::time::Duration::from_micros(21_500)).await;
        assert_eq!(pace(&mut f), 19);
        // A late wake of more than a whole packet time leaves no sleep at all.
        tokio::time::advance(std::time::Duration::from_millis(60)).await;
        assert_eq!(pace(&mut f), 0);
        assert!(call(&mut f, "pace", &[ArgValue::Int(25)], &[]).is_err());
    }

    #[test]
    fn counts_and_times_follow_the_packet_time() {
        let mut f = SipRtp::default();
        let n = |ms, pt| {
            text(
                call(
                    &mut SipRtp::default(),
                    "packets",
                    &[ArgValue::Int(ms), ArgValue::Int(pt)],
                    &[],
                )
                .unwrap(),
            )
        };
        assert_eq!(n(10_000, 20), "500");
        assert_eq!(n(10_000, 30), "333");
        assert_eq!(n(5, 20), "1");
        assert_eq!(
            text(call(&mut f, "hangup_ms", &[ArgValue::Int(10_000)], &[]).unwrap()),
            "11200"
        );
        for bad in [0, -1, 86_400_001] {
            assert!(call(&mut f, "packets", &[ArgValue::Int(bad)], &[]).is_err());
            assert!(call(&mut f, "hangup_ms", &[ArgValue::Int(bad)], &[]).is_err());
        }
    }

    #[test]
    fn bad_arguments_fail_the_call_and_not_the_process() {
        let mut f = SipRtp::default();
        let base = |ptime: i64, codecs: &'static str, server: &'static str| {
            call(
                &mut SipRtp::default(),
                "invite",
                &[
                    ArgValue::Text(server),
                    ArgValue::Text("192.0.2.10"),
                    ArgValue::Int(16384),
                ],
                &[
                    NamedArg {
                        name: "ptime",
                        value: ArgValue::Int(ptime),
                    },
                    NamedArg {
                        name: "codecs",
                        value: ArgValue::Text(codecs),
                    },
                ],
            )
        };
        assert!(base(20, "PCMU", "192.0.2.20").is_ok());
        assert!(base(25, "PCMU", "192.0.2.20").is_err());
        assert!(base(0, "PCMU", "192.0.2.20").is_err());
        assert!(base(130, "PCMU", "192.0.2.20").is_err());
        assert!(base(20, "", "192.0.2.20").is_err());
        assert!(base(20, "speex", "192.0.2.20").is_err());
        assert!(base(20, "PCMU", "bad host\r\n").is_err());
        assert!(base(20, "PCMU", "").is_err());
        let hostile: [&[u8]; 4] = [b"", b"\xff\xfe", b"INVITE", b"SIP/2.0 200 OK\r\n\r\n"];
        for h in hostile {
            for function in ["sdp_port", "codec", "ack", "bye"] {
                let r = call(&mut f, function, &[ArgValue::Bytes(h)], &[]);
                assert!(r.is_err(), "{function} accepted {h:?}");
            }
            let r = call(
                &mut f,
                "response",
                &[ArgValue::Bytes(h), ArgValue::Int(200)],
                &[],
            );
            assert!(r.is_err());
        }
        let r = call(&mut f, "rtp", &[ArgValue::Text("speex")], &[]);
        assert!(r.is_err());
    }
}
