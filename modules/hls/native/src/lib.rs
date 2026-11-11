// SPDX-License-Identifier: GPL-3.0-only
//! Native template functions of the `hls` module.
//!
//! Decision D10 lets first party modules implement their template functions in
//! Rust. This crate implements the `hls` functions behind the [`TemplateFn`]
//! trait of `fps-flow`, so the interpreter renders
//! `{{ http1.response(200, body=hls.master(vars.ladder), content_type='application/vnd.apple.mpegurl') }}`
//! without a WebAssembly runtime. The module writes what HTTP Live Streaming
//! (RFC 8216) puts in the bodies of a session: the master playlist of a
//! bitrate ladder, the media playlist of one rung in its VOD or live form, and
//! the MPEG-TS segments, whose size follows the bit rate of the rung. It also
//! reads what a player reads, which is the URIs of a playlist and the length of
//! a body. The HTTP messages around the bodies come from the `http1` module.
//!
//! The functions `master`, `media`, `segment`, `segment_size`,
//! `content_length`, `playlist_uri`, `playlist_count`, `resolve`, and
//! `rendition` keep no state. The functions `next` and `pace` do: an [`Hls`]
//! value holds the counter and the schedule of one flow side, so the engine
//! must give every flow side its own value, as the fixture peer and the tests
//! do.
//!
//! [`FUNCTIONS`] is the catalog that `modules/hls/manifest.yaml` lists under
//! `functions`. The catalog test compares the two, so the manifest stays the
//! single catalog that the validator and the controller read.
#![forbid(unsafe_code)]
#![warn(missing_docs, clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod args;
pub mod catalog;
pub mod ladder;
pub mod playlist;
pub mod ts;

use fps_flow::{FnCall, FnError, FnErrorKind, TemplateFn};
use rand::RngCore;

pub use catalog::{manifest_functions, DefaultValue, FunctionSpec, ParamSpec, FUNCTIONS};

/// Name of the module that this crate implements.
pub const MODULE_NAME: &str = "hls";

/// The shortest and the longest segment that the module writes, in milliseconds.
pub const SEGMENT_MS_RANGE: std::ops::RangeInclusive<i64> = 100..=60_000;

/// The most segments of one playlist.
pub const MAX_SEGMENTS: i64 = 100_000;

/// The template functions of the `hls` module for one flow side.
#[derive(Clone, Debug, Default)]
pub struct Hls {
    /// The number that the next call of `next` returns.
    counter: u64,
    /// When the next segment is due, which `pace` advances by one segment duration.
    due: Option<tokio::time::Instant>,
}

fn invalid(message: String) -> FnError {
    FnError::new(FnErrorKind::InvalidArgument, message)
}

fn push(out: &mut Vec<u8>, text: &str) {
    out.extend_from_slice(text.as_bytes());
}

fn ranged(
    function: &str,
    name: &str,
    v: i64,
    range: std::ops::RangeInclusive<i64>,
) -> Result<u64, FnError> {
    if range.contains(&v) {
        Ok(v as u64)
    } else {
        Err(invalid(format!(
            "'{name}' of '{function}' must lie between {} and {}, not {v}",
            range.start(),
            range.end()
        )))
    }
}

fn segment_ms_arg(function: &str, v: i64) -> Result<u64, FnError> {
    ranged(function, "segment_ms", v, SEGMENT_MS_RANGE)
}

fn ladder_arg(function: &str, text: &str) -> Result<Vec<ladder::Rung>, FnError> {
    ladder::parse(text).map_err(|e| invalid(format!("'ladder' of '{function}': {e}")))
}

fn reading<T>(function: &str, r: Result<T, String>) -> Result<T, FnError> {
    r.map_err(|e| invalid(format!("'{function}': {e}")))
}

impl TemplateFn for Hls {
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
            "master" => {
                let a = args::bind(&catalog::MASTER, &call.args)?;
                let rungs = ladder_arg("master", a.text("ladder")?)?;
                push(out, &playlist::master(&rungs));
            }
            "media" => {
                let a = args::bind(&catalog::MEDIA, &call.args)?;
                let m = playlist::Media {
                    segments: ranged("media", "segments", a.int("segments")?, 0..=MAX_SEGMENTS)?,
                    segment_ms: segment_ms_arg("media", a.int("segment_ms")?)?,
                    first: ranged("media", "first", a.int("first")?, 0..=MAX_SEGMENTS)?,
                    count: ranged("media", "count", a.int("count")?, 0..=MAX_SEGMENTS)?,
                    live: a.boolean("live")?,
                };
                push(out, &reading("media", playlist::media(&m))?);
            }
            "segment" => {
                let a = args::bind(&catalog::SEGMENT, &call.args)?;
                let rungs = ladder_arg("segment", a.text("ladder")?)?;
                let segment_ms = segment_ms_arg("segment", a.int("segment_ms")?)?;
                let (variant, index) = reading(
                    "segment",
                    playlist::parse_segment_request(a.bytes("request")?),
                )?;
                let rung = reading("segment", ladder::rung(&rungs, variant))?;
                let size = reading("segment", ts::size(rung.bits_per_second, segment_ms))?;
                out.reserve(size as usize);
                ts::segment(
                    index,
                    segment_ms,
                    ts::packets(rung.bits_per_second, segment_ms),
                    rng,
                    out,
                );
            }
            "segment_size" => {
                let a = args::bind(&catalog::SEGMENT_SIZE, &call.args)?;
                let rungs = ladder_arg("segment_size", a.text("ladder")?)?;
                let segment_ms = segment_ms_arg("segment_size", a.int("segment_ms")?)?;
                let rung = reading("segment_size", ladder::rung(&rungs, a.int("variant")?))?;
                let size = reading("segment_size", ts::size(rung.bits_per_second, segment_ms))?;
                push(out, &size.to_string());
            }
            "content_length" => {
                let a = args::bind(&catalog::CONTENT_LENGTH, &call.args)?;
                let n = reading(
                    "content_length",
                    playlist::content_length(a.bytes("message")?),
                )?;
                push(out, &n.to_string());
            }
            "playlist_uri" => {
                let a = args::bind(&catalog::PLAYLIST_URI, &call.args)?;
                let u = reading(
                    "playlist_uri",
                    playlist::uri(a.bytes("playlist")?, a.int("index")?),
                )?;
                push(out, &u);
            }
            "playlist_count" => {
                let a = args::bind(&catalog::PLAYLIST_COUNT, &call.args)?;
                let n = reading("playlist_count", playlist::count(a.bytes("playlist")?))?;
                push(out, &n.to_string());
            }
            "resolve" => {
                let a = args::bind(&catalog::RESOLVE, &call.args)?;
                // The arguments are bytes, because a nested call renders bytes.
                let text = |name: &str| {
                    std::str::from_utf8(a.bytes(name)?)
                        .map_err(|_| invalid(format!("'{name}' of 'resolve' is not text")))
                };
                let p = reading("resolve", playlist::resolve(text("base")?, text("uri")?))?;
                push(out, &p);
            }
            "rendition" => {
                let a = args::bind(&catalog::RENDITION, &call.args)?;
                let rungs = ladder_arg("rendition", a.text("ladder")?)?;
                let i = reading(
                    "rendition",
                    ladder::pick(&rungs, a.text("plan")?, a.int("n")?),
                )?;
                push(out, &i.to_string());
            }
            "next" => {
                let a = args::bind(&catalog::NEXT, &call.args)?;
                let step = ranged("next", "step", a.int("step")?, 0..=MAX_SEGMENTS)?;
                push(out, &self.counter.to_string());
                self.counter += step;
            }
            "pace" => {
                let a = args::bind(&catalog::PACE, &call.args)?;
                let step =
                    std::time::Duration::from_millis(segment_ms_arg("pace", a.int("segment_ms")?)?);
                let now = tokio::time::Instant::now();
                // The first call is due at once, so the first segment follows the playlist
                // without a gap. Every later call sleeps until the next slot of the schedule,
                // so the sleeps do not add their rounding up to a drift, and a late wake
                // shortens the next sleep.
                let due = self.due.map_or(now, |d| d + step);
                self.due = Some(due);
                let wait = due.saturating_duration_since(now);
                let ms = (wait.as_micros() + 500) / 1000;
                push(out, &ms.to_string());
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
        f: &mut Hls,
        function: &'static str,
        positional: &[ArgValue<'_>],
        named: &[NamedArg<'_>],
    ) -> Result<Vec<u8>, FnError> {
        let mut rng = rand_pcg::Pcg64Mcg::new(42);
        let mut out = Vec::new();
        let c = FnCall {
            module: MODULE_NAME,
            function,
            args: CallArgs { positional, named },
        };
        f.render(&c, &mut rng, &mut out).map(|()| out)
    }

    fn text(v: Vec<u8>) -> String {
        String::from_utf8(v).unwrap()
    }

    const LADDER: &str = "200000@426x240,400000@640x360,800000@960x540";

    #[test]
    fn rejects_other_modules_and_functions() {
        let mut rng = rand_pcg::Pcg64Mcg::new(1);
        let c = FnCall {
            module: "dns",
            function: "master",
            args: CallArgs {
                positional: &[],
                named: &[],
            },
        };
        let err = Hls::default()
            .render(&c, &mut rng, &mut Vec::new())
            .unwrap_err();
        assert_eq!(err.kind, FnErrorKind::Unsupported);
        let err = call(&mut Hls::default(), "teapot", &[], &[]).unwrap_err();
        assert_eq!(err.kind, FnErrorKind::Unsupported);
    }

    #[test]
    fn master_and_media_render() {
        let mut f = Hls::default();
        let m = text(call(&mut f, "master", &[ArgValue::Text(LADDER)], &[]).unwrap());
        assert_eq!(playlist::count(m.as_bytes()).unwrap(), 3);
        let p = text(
            call(
                &mut f,
                "media",
                &[ArgValue::Int(4), ArgValue::Int(2000)],
                &[],
            )
            .unwrap(),
        );
        assert!(p.contains("#EXT-X-PLAYLIST-TYPE:VOD") && p.ends_with("#EXT-X-ENDLIST\n"));
        assert_eq!(playlist::count(p.as_bytes()).unwrap(), 4);
        let live = text(
            call(
                &mut f,
                "media",
                &[ArgValue::Int(0), ArgValue::Int(2000)],
                &[
                    NamedArg {
                        name: "first",
                        value: ArgValue::Int(5),
                    },
                    NamedArg {
                        name: "count",
                        value: ArgValue::Int(3),
                    },
                    NamedArg {
                        name: "live",
                        value: ArgValue::Boolean(true),
                    },
                ],
            )
            .unwrap(),
        );
        assert!(live.contains("seg00005.ts") && live.contains("seg00007.ts"));
    }

    #[test]
    fn segment_has_the_size_of_its_rung() {
        let mut f = Hls::default();
        for (variant, path) in [
            (0, "/v0/seg00002.ts"),
            (1, "/v1/seg00002.ts"),
            (2, "/v2/seg00002.ts"),
        ] {
            let req = format!("GET {path} HTTP/1.1\r\nHost: h\r\n\r\n");
            let body = call(
                &mut f,
                "segment",
                &[
                    ArgValue::Bytes(req.as_bytes()),
                    ArgValue::Text(LADDER),
                    ArgValue::Int(2000),
                ],
                &[],
            )
            .unwrap();
            let size = text(
                call(
                    &mut f,
                    "segment_size",
                    &[
                        ArgValue::Text(LADDER),
                        ArgValue::Int(variant),
                        ArgValue::Int(2000),
                    ],
                    &[],
                )
                .unwrap(),
            );
            assert_eq!(body.len().to_string(), size);
            // The size is within one packet of the bit rate times the duration.
            let rate = [200_000u64, 400_000, 800_000][variant as usize];
            let want = rate * 2000 / 8000;
            assert!(
                (body.len() as u64).abs_diff(want) <= 94,
                "{} vs {want}",
                body.len()
            );
            assert_eq!(body[0], 0x47);
        }
    }

    #[test]
    fn segment_rejects_a_rung_that_is_not_in_the_ladder() {
        let req = b"GET /v3/seg00000.ts HTTP/1.1\r\n\r\n";
        let err = call(
            &mut Hls::default(),
            "segment",
            &[
                ArgValue::Bytes(req),
                ArgValue::Text(LADDER),
                ArgValue::Int(2000),
            ],
            &[],
        )
        .unwrap_err();
        assert_eq!(err.kind, FnErrorKind::InvalidArgument);
    }

    #[test]
    fn readers_work_on_what_the_writers_wrote() {
        let mut f = Hls::default();
        let m = call(&mut f, "master", &[ArgValue::Text(LADDER)], &[]).unwrap();
        let uri = text(
            call(
                &mut f,
                "playlist_uri",
                &[ArgValue::Bytes(&m), ArgValue::Int(2)],
                &[],
            )
            .unwrap(),
        );
        assert_eq!(uri, "v2/index.m3u8");
        let path = text(
            call(
                &mut f,
                "resolve",
                &[ArgValue::Text("/index.m3u8"), ArgValue::Text(&uri)],
                &[],
            )
            .unwrap(),
        );
        assert_eq!(path, "/v2/index.m3u8");
        let n = text(call(&mut f, "playlist_count", &[ArgValue::Bytes(&m)], &[]).unwrap());
        assert_eq!(n, "3");
        let head = b"HTTP/1.1 200 OK\r\nContent-Length: 77\r\n\r\n";
        assert_eq!(
            text(call(&mut f, "content_length", &[ArgValue::Bytes(head)], &[]).unwrap()),
            "77"
        );
    }

    #[test]
    fn rendition_follows_the_plan() {
        let mut f = Hls::default();
        let got: Vec<String> = (0..5)
            .map(|n| {
                text(
                    call(
                        &mut f,
                        "rendition",
                        &[
                            ArgValue::Text(LADDER),
                            ArgValue::Text("ramp:2"),
                            ArgValue::Int(n),
                        ],
                        &[],
                    )
                    .unwrap(),
                )
            })
            .collect();
        assert_eq!(got, ["0", "0", "1", "1", "2"]);
    }

    #[test]
    fn next_counts_per_value() {
        let mut a = Hls::default();
        let mut b = Hls::default();
        let seq: Vec<String> = (0..3)
            .map(|_| text(call(&mut a, "next", &[], &[]).unwrap()))
            .collect();
        assert_eq!(seq, ["0", "1", "2"]);
        assert_eq!(text(call(&mut b, "next", &[], &[]).unwrap()), "0");
        assert_eq!(text(call(&mut a.clone(), "next", &[], &[]).unwrap()), "3");
        // A step of 0 reads the counter and leaves it.
        assert_eq!(
            text(call(&mut a, "next", &[ArgValue::Int(0)], &[]).unwrap()),
            "3"
        );
        assert_eq!(text(call(&mut a, "next", &[], &[]).unwrap()), "3");
        assert_eq!(
            text(call(&mut a, "next", &[ArgValue::Int(5)], &[]).unwrap()),
            "4"
        );
        assert_eq!(text(call(&mut a, "next", &[], &[]).unwrap()), "9");
    }

    #[tokio::test(start_paused = true)]
    async fn pace_is_due_at_once_and_then_one_segment_apart() {
        let mut f = Hls::default();
        let ms = |f: &mut Hls| text(call(f, "pace", &[ArgValue::Int(2000)], &[]).unwrap());
        assert_eq!(ms(&mut f), "0");
        // The wake came 150 ms late, so the next sleep is 150 ms shorter.
        tokio::time::advance(std::time::Duration::from_millis(150)).await;
        assert_eq!(ms(&mut f), "1850");
        tokio::time::advance(std::time::Duration::from_millis(1850)).await;
        assert_eq!(ms(&mut f), "2000");
    }

    #[test]
    fn arguments_are_checked() {
        let mut f = Hls::default();
        for (function, p) in [
            ("master", vec![ArgValue::Text("nonsense")]),
            (
                "segment_size",
                vec![ArgValue::Text(LADDER), ArgValue::Int(0), ArgValue::Int(5)],
            ),
            ("pace", vec![ArgValue::Int(10_000_000)]),
            (
                "rendition",
                vec![
                    ArgValue::Text(LADDER),
                    ArgValue::Text("jump"),
                    ArgValue::Int(0),
                ],
            ),
            (
                "playlist_uri",
                vec![ArgValue::Bytes(b"junk"), ArgValue::Int(0)],
            ),
        ] {
            let err = call(&mut f, function, &p, &[]).unwrap_err();
            assert_eq!(err.kind, FnErrorKind::InvalidArgument, "{function}: {err}");
        }
    }
}
