// SPDX-License-Identifier: GPL-3.0-only
//! Native template functions of the `ftp` module.
//!
//! Decision D10 lets first party modules implement their template functions in
//! Rust. This crate implements the `ftp` functions behind the [`TemplateFn`]
//! trait of `fps-flow`, so the interpreter renders
//! `{{ ftp.pasv_reply(vars.server_ip, vars.data_port) }}` without a
//! WebAssembly runtime. The functions render the parts of an FTP dialogue that
//! a scenario cannot write as plain text, which are the passive mode replies,
//! the multi line `FEAT` reply, directory listings, and synthetic names, and
//! they read back what the other side announced, which is the data port of a
//! 227 or 229 reply and the number in an `ALLO` command or a `213` reply.
//!
//! [`FUNCTIONS`] is the catalog that `modules/ftp/manifest.yaml` lists under
//! `functions`. The catalog test compares the two, so the manifest stays the
//! single catalog that the validator and the controller read.
#![forbid(unsafe_code)]
#![warn(missing_docs, clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod args;
pub mod catalog;
mod names;
mod rng;

use fps_flow::{FnCall, FnError, FnErrorKind, TemplateFn};
use rand::RngCore;

pub use catalog::{DefaultValue, FunctionSpec, ParamSpec, FUNCTIONS, MAX_ENTRIES};

/// Name of the module that this crate implements.
pub const MODULE_NAME: &str = "ftp";

/// The template functions of the `ftp` module for one session.
///
/// The functions keep no state between calls, so one value may serve any
/// number of sessions.
#[derive(Clone, Copy, Debug, Default)]
pub struct Ftp;

fn invalid(message: String) -> FnError {
    FnError::new(FnErrorKind::InvalidArgument, message)
}

/// Mixes the function name into the seed, so two functions called with one
/// seed do not share a random sequence.
fn derive(function: &str, seed: i64, rng: &mut dyn RngCore) -> rng::Rng {
    let base = if seed >= 0 {
        seed as u64
    } else {
        rng.next_u64()
    };
    let salt = function.bytes().fold(0xCBF2_9CE4_8422_2325u64, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01B3)
    });
    rng::Rng::new(base ^ salt)
}

fn push(out: &mut Vec<u8>, text: &str) {
    out.extend_from_slice(text.as_bytes());
}

fn port_arg(function: &str, port: i64) -> Result<u16, FnError> {
    u16::try_from(port).ok().filter(|p| *p > 0).ok_or_else(|| {
        invalid(format!(
            "'port' of '{function}' must lie between 1 and 65535, not {port}"
        ))
    })
}

/// Parses the dotted quad of a PASV host.
fn octets(host: &str) -> Result<[u8; 4], FnError> {
    let bad = || {
        invalid(format!(
            "'host' of 'pasv_reply' must be a dotted IPv4 address, not '{host}'"
        ))
    };
    let mut out = [0u8; 4];
    let mut parts = host.split('.');
    for slot in &mut out {
        *slot = parts
            .next()
            .filter(|p| !p.is_empty() && p.len() <= 3 && p.bytes().all(|b| b.is_ascii_digit()))
            .and_then(|p| p.parse().ok())
            .ok_or_else(bad)?;
    }
    if parts.next().is_some() {
        return Err(bad());
    }
    Ok(out)
}

/// Reads the text between the first `(` and the next `)` of a reply that
/// starts with `code` and a space.
fn parenthesized<'a>(function: &str, reply: &'a [u8], code: &str) -> Result<&'a str, FnError> {
    let bad = |why: &str| {
        invalid(format!(
            "'reply' of '{function}' must be a {code} reply, {why}"
        ))
    };
    let text = std::str::from_utf8(reply).map_err(|_| bad("not UTF-8 text"))?;
    if !text.starts_with(code) || text.as_bytes().get(3) != Some(&b' ') {
        return Err(bad("it starts with another code"));
    }
    let open = text.find('(').ok_or_else(|| bad("it has no '('"))?;
    let close = text[open..].find(')').ok_or_else(|| bad("it has no ')'"))? + open;
    Ok(&text[open + 1..close])
}

/// The port of a 227 reply, which ends the reply in `(h1,h2,h3,h4,p1,p2)`.
fn pasv_port(reply: &[u8]) -> Result<u16, FnError> {
    let inner = parenthesized("pasv_port", reply, "227")?;
    let nums: Result<Vec<u8>, _> = inner.split(',').map(|p| p.trim().parse::<u8>()).collect();
    match nums.as_deref() {
        Ok([_, _, _, _, hi, lo]) => Ok(u16::from(*hi) << 8 | u16::from(*lo)),
        _ => Err(invalid(format!(
            "'reply' of 'pasv_port' must list six numbers below 256, not '({inner})'"
        ))),
    }
}

/// The port of a 229 reply, which ends the reply in `(|||port|)`.
fn epsv_port(reply: &[u8]) -> Result<u16, FnError> {
    let inner = parenthesized("epsv_port", reply, "229")?;
    inner
        .strip_prefix("|||")
        .and_then(|r| r.strip_suffix('|'))
        .and_then(|p| p.parse::<u16>().ok())
        .filter(|p| *p > 0)
        .ok_or_else(|| {
            invalid(format!(
                "'reply' of 'epsv_port' must hold (|||port|), not '({inner})'"
            ))
        })
}

/// The last run of decimal digits in `line`.
fn last_number(line: &[u8]) -> Result<u64, FnError> {
    let end = line
        .iter()
        .rposition(u8::is_ascii_digit)
        .ok_or_else(|| invalid("'line' of 'number' holds no digit".into()))?;
    let start = line[..=end]
        .iter()
        .rposition(|b| !b.is_ascii_digit())
        .map_or(0, |i| i + 1);
    std::str::from_utf8(&line[start..=end])
        .ok()
        .and_then(|s| s.parse::<i64>().ok())
        .and_then(|n| u64::try_from(n).ok())
        .ok_or_else(|| invalid("'line' of 'number' holds a number that is too large".into()))
}

/// The text after the first space of a command line, without the line break.
/// The result holds only printable ASCII, so a server can echo it in a reply.
fn command_arg(line: &[u8]) -> Result<&str, FnError> {
    let text = std::str::from_utf8(line)
        .map_err(|_| invalid("'line' of 'arg' must be UTF-8 text".into()))?
        .trim_end_matches(['\r', '\n']);
    let (_, arg) = text
        .split_once(' ')
        .filter(|(_, a)| !a.is_empty())
        .ok_or_else(|| invalid("'line' of 'arg' holds no argument".into()))?;
    if !arg.bytes().all(|b| b.is_ascii_graphic() || b == b' ') {
        return Err(invalid(
            "the argument in 'line' of 'arg' must be printable ASCII".into(),
        ));
    }
    Ok(arg)
}

fn feat_reply(utf8: bool, out: &mut Vec<u8>) {
    // The feature list of a typical vsftpd server. Every line but the last has
    // a hyphen after the code on the first line and a leading space on the
    // others, as RFC 959 section 4.2 and RFC 2389 require.
    push(out, "211-Features:\r\n");
    for f in [
        "EPRT",
        "EPSV",
        "MDTM",
        "PASV",
        "REST STREAM",
        "SIZE",
        "TVFS",
    ] {
        push(out, " ");
        push(out, f);
        push(out, "\r\n");
    }
    if utf8 {
        push(out, " UTF8\r\n");
    }
    push(out, "211 End\r\n");
}

impl TemplateFn for Ftp {
    fn render(
        &mut self,
        call: &FnCall<'_>,
        rng: &mut dyn RngCore,
        out: &mut Vec<u8>,
    ) -> Result<(), FnError> {
        if call.module != MODULE_NAME {
            return Err(FnError::new(
                FnErrorKind::Unsupported,
                format!("module '{}' is not ftp", call.module),
            ));
        }
        match call.function {
            "credential" => {
                let a = args::bind(&catalog::CREDENTIAL, &call.args)?;
                let mut r = derive("credential", a.int("seed")?, rng);
                match a.text("field")? {
                    "user" => push(out, &names::user(&mut r)),
                    "password" => push(out, &names::password(&mut r)),
                    other => {
                        return Err(invalid(format!(
                            "field '{other}' of 'credential' is not supported, use user or password"
                        )))
                    }
                }
            }
            "file_name" => {
                let a = args::bind(&catalog::FILE_NAME, &call.args)?;
                let ext = a.text("ext")?;
                if ext.is_empty()
                    || ext.len() > 8
                    || !ext
                        .bytes()
                        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'.')
                {
                    return Err(invalid(format!(
                        "'ext' of 'file_name' must be 1 to 8 lower case letters, digits, or dots, not '{ext}'"
                    )));
                }
                let seed = a.int("seed")?;
                if seed < 0 {
                    return Err(invalid(format!(
                        "'seed' of 'file_name' must not be negative, not {seed}"
                    )));
                }
                let mut r = derive("file_name", seed, rng);
                push(out, &names::file_name(&mut r, ext));
            }
            "listing" => {
                let a = args::bind(&catalog::LISTING, &call.args)?;
                let count = a.int("count")?;
                if !(1..=catalog::MAX_ENTRIES).contains(&count) {
                    return Err(invalid(format!(
                        "'count' of 'listing' must lie between 1 and {}, not {count}",
                        catalog::MAX_ENTRIES
                    )));
                }
                let seed = a.int("seed")?;
                if seed < 0 {
                    return Err(invalid(format!(
                        "'seed' of 'listing' must not be negative, not {seed}"
                    )));
                }
                let mut r = derive("listing", seed, rng);
                names::listing(&mut r, count as usize, out);
            }
            "length" => {
                let a = args::bind(&catalog::LENGTH, &call.args)?;
                push(out, &a.bytes("data")?.len().to_string());
            }
            "number" => {
                let a = args::bind(&catalog::NUMBER, &call.args)?;
                push(out, &last_number(a.bytes("line")?)?.to_string());
            }
            "arg" => {
                let a = args::bind(&catalog::ARG, &call.args)?;
                push(out, command_arg(a.bytes("line")?)?);
            }
            "pasv_reply" => {
                let a = args::bind(&catalog::PASV_REPLY, &call.args)?;
                let [h1, h2, h3, h4] = octets(a.text("host")?)?;
                let port = port_arg("pasv_reply", a.int("port")?)?;
                push(
                    out,
                    &format!(
                        "227 Entering Passive Mode ({h1},{h2},{h3},{h4},{},{}).\r\n",
                        port >> 8,
                        port & 0xff
                    ),
                );
            }
            "pasv_port" => {
                let a = args::bind(&catalog::PASV_PORT, &call.args)?;
                push(out, &pasv_port(a.bytes("reply")?)?.to_string());
            }
            "epsv_reply" => {
                let a = args::bind(&catalog::EPSV_REPLY, &call.args)?;
                let port = port_arg("epsv_reply", a.int("port")?)?;
                push(
                    out,
                    &format!("229 Entering Extended Passive Mode (|||{port}|).\r\n"),
                );
            }
            "epsv_port" => {
                let a = args::bind(&catalog::EPSV_PORT, &call.args)?;
                push(out, &epsv_port(a.bytes("reply")?)?.to_string());
            }
            "feat_reply" => {
                let a = args::bind(&catalog::FEAT_REPLY, &call.args)?;
                feat_reply(a.boolean("utf8")?, out);
            }
            other => {
                return Err(FnError::new(
                    FnErrorKind::Unsupported,
                    format!("ftp has no function '{other}'"),
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

    fn render(
        function: &'static str,
        positional: &[ArgValue<'_>],
        named: &[NamedArg<'_>],
    ) -> Result<String, FnError> {
        let mut rng = rand_pcg::Pcg64Mcg::new(42);
        let mut out = Vec::new();
        let c = FnCall {
            module: "ftp",
            function,
            args: CallArgs { positional, named },
        };
        Ftp.render(&c, &mut rng, &mut out)
            .map(|()| String::from_utf8(out).unwrap())
    }

    #[test]
    fn rejects_other_modules_and_functions() {
        let mut rng = rand_pcg::Pcg64Mcg::new(1);
        let c = FnCall {
            module: "dns",
            function: "length",
            args: CallArgs {
                positional: &[],
                named: &[],
            },
        };
        assert_eq!(
            Ftp.render(&c, &mut rng, &mut Vec::new()).unwrap_err().kind,
            FnErrorKind::Unsupported
        );
        assert_eq!(
            render("mlsd", &[], &[]).unwrap_err().kind,
            FnErrorKind::Unsupported
        );
    }

    #[test]
    fn pasv_reply_splits_the_port_into_two_bytes() {
        let s = render(
            "pasv_reply",
            &[ArgValue::Text("192.0.2.10"), ArgValue::Int(50_123)],
            &[],
        )
        .unwrap();
        // 50123 = 195 * 256 + 203.
        assert_eq!(s, "227 Entering Passive Mode (192,0,2,10,195,203).\r\n");
        let back = render("pasv_port", &[ArgValue::Text(&s)], &[]).unwrap();
        assert_eq!(back, "50123");
    }

    #[test]
    fn pasv_reply_checks_the_host_and_the_port() {
        for host in [
            "192.0.2",
            "192.0.2.10.1",
            "a.b.c.d",
            "256.1.1.1",
            "1..2.3",
            "",
        ] {
            let e = render(
                "pasv_reply",
                &[ArgValue::Text(host), ArgValue::Int(21)],
                &[],
            );
            assert_eq!(e.unwrap_err().kind, FnErrorKind::InvalidArgument, "{host}");
        }
        for port in [0, -1, 65_536] {
            let e = render(
                "pasv_reply",
                &[ArgValue::Text("192.0.2.1"), ArgValue::Int(port)],
                &[],
            );
            assert_eq!(e.unwrap_err().kind, FnErrorKind::InvalidArgument, "{port}");
        }
    }

    #[test]
    fn pasv_port_reads_replies_of_other_servers() {
        // The wording differs from server to server, the parentheses do not.
        let reply = "227 Entering Passive Mode (10, 1, 2, 3, 4, 1)\r\n";
        assert_eq!(
            render("pasv_port", &[ArgValue::Text(reply)], &[]).unwrap(),
            "1025"
        );
        for bad in [
            "229 Entering Passive Mode (1,2,3,4,5,6)\r\n",
            "227 Entering Passive Mode 1,2,3,4,5,6\r\n",
            "227 Entering Passive Mode (1,2,3,4,5)\r\n",
            "227 Entering Passive Mode (1,2,3,4,5,300)\r\n",
            "227Entering Passive Mode (1,2,3,4,5,6)\r\n",
        ] {
            let e = render("pasv_port", &[ArgValue::Text(bad)], &[]);
            assert_eq!(e.unwrap_err().kind, FnErrorKind::InvalidArgument, "{bad}");
        }
    }

    #[test]
    fn epsv_reply_round_trips() {
        let s = render("epsv_reply", &[ArgValue::Int(50_001)], &[]).unwrap();
        assert_eq!(s, "229 Entering Extended Passive Mode (|||50001|).\r\n");
        assert_eq!(
            render("epsv_port", &[ArgValue::Text(&s)], &[]).unwrap(),
            "50001"
        );
        for bad in [
            "229 Entering Extended Passive Mode (||50001|)\r\n",
            "229 Entering Extended Passive Mode (|||0|)\r\n",
            "229 Entering Extended Passive Mode (|||70000|)\r\n",
            "227 Entering Extended Passive Mode (|||5|)\r\n",
        ] {
            let e = render("epsv_port", &[ArgValue::Text(bad)], &[]);
            assert_eq!(e.unwrap_err().kind, FnErrorKind::InvalidArgument, "{bad}");
        }
    }

    #[test]
    fn number_takes_the_last_run_of_digits() {
        for (line, want) in [
            ("ALLO 48213\r\n", "48213"),
            ("213 1048576\r\n", "1048576"),
            (
                "150 Opening BINARY mode data connection for a-2.pdf (777 bytes).\r\n",
                "777",
            ),
            ("7", "7"),
        ] {
            assert_eq!(
                render("number", &[ArgValue::Text(line)], &[]).unwrap(),
                want,
                "{line}"
            );
        }
        for bad in ["no digits\r\n", "", "99999999999999999999"] {
            let e = render("number", &[ArgValue::Text(bad)], &[]);
            assert_eq!(e.unwrap_err().kind, FnErrorKind::InvalidArgument, "{bad}");
        }
    }

    #[test]
    fn arg_returns_the_text_after_the_command() {
        let s = render("arg", &[ArgValue::Text("RETR a b.pdf\r\n")], &[]).unwrap();
        assert_eq!(s, "a b.pdf");
        for bad in ["QUIT\r\n", "RETR \r\n", "RETR a\u{e9}\r\n"] {
            let e = render("arg", &[ArgValue::Text(bad)], &[]);
            assert_eq!(e.unwrap_err().kind, FnErrorKind::InvalidArgument, "{bad}");
        }
    }

    #[test]
    fn length_counts_bytes() {
        let s = render("length", &[ArgValue::Bytes(&[0, 1, 2, 255])], &[]).unwrap();
        assert_eq!(s, "4");
    }

    #[test]
    fn feat_reply_is_a_multi_line_reply_that_ends_in_a_space_line() {
        let s = render("feat_reply", &[], &[]).unwrap();
        let lines: Vec<&str> = s.split("\r\n").collect();
        assert_eq!(lines.first(), Some(&"211-Features:"));
        assert_eq!(lines[lines.len() - 2], "211 End");
        assert!(lines[1..lines.len() - 2].iter().all(|l| l.starts_with(' ')));
        assert!(s.contains(" EPSV\r\n") && s.contains(" SIZE\r\n") && s.contains(" UTF8\r\n"));
        let s = render("feat_reply", &[ArgValue::Boolean(false)], &[]).unwrap();
        assert!(!s.contains("UTF8"));
    }

    #[test]
    fn listing_and_names_depend_only_on_their_arguments() {
        let a = render("listing", &[ArgValue::Int(7), ArgValue::Int(12)], &[]).unwrap();
        let b = render("listing", &[ArgValue::Int(7), ArgValue::Int(12)], &[]).unwrap();
        let c = render("listing", &[ArgValue::Int(8), ArgValue::Int(12)], &[]).unwrap();
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert_eq!(a.matches("\r\n").count(), 12);
        let f = render("file_name", &[ArgValue::Int(3)], &[]).unwrap();
        assert!(f.ends_with(".bin"));
        assert_eq!(f, render("file_name", &[ArgValue::Int(3)], &[]).unwrap());
        for bad in [("listing", 0), ("listing", 501)] {
            let e = render(bad.0, &[ArgValue::Int(1), ArgValue::Int(bad.1)], &[]);
            assert_eq!(e.unwrap_err().kind, FnErrorKind::InvalidArgument);
        }
    }

    #[test]
    fn credentials_follow_the_session_generator_unless_seeded() {
        let a = render("credential", &[ArgValue::Text("password")], &[]).unwrap();
        assert!(a.starts_with("Tst-"));
        let s1 = render(
            "credential",
            &[ArgValue::Text("user"), ArgValue::Int(5)],
            &[],
        )
        .unwrap();
        let s2 = render(
            "credential",
            &[ArgValue::Text("user"), ArgValue::Int(5)],
            &[],
        )
        .unwrap();
        assert_eq!(s1, s2);
        let e = render("credential", &[ArgValue::Text("token")], &[]);
        assert_eq!(e.unwrap_err().kind, FnErrorKind::InvalidArgument);
    }
}
