// SPDX-License-Identifier: GPL-3.0-only
//! Native template functions of the `smtp` module.
//!
//! Decision D10 lets first party modules implement their template functions in
//! Rust. This crate implements the `smtp` functions behind the [`TemplateFn`]
//! trait of `fps-flow`, so the interpreter renders
//! `{{ smtp.data(smtp.message(...)) }}` without a WebAssembly runtime. The
//! functions render the parts of an SMTP dialogue that a scenario cannot write
//! as plain text: the multi line EHLO reply, `RCPT TO` lines for a variable
//! number of recipients, the `AUTH PLAIN` token, a MIME message with a base64
//! attachment, and the dot stuffed DATA payload.
//!
//! [`FUNCTIONS`] is the catalog that `modules/smtp/manifest.yaml` lists under
//! `functions`. The catalog test compares the two, so the manifest stays the
//! single catalog that the validator and the controller read.
#![forbid(unsafe_code)]
#![warn(missing_docs, clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod args;
pub mod catalog;
mod mime;
mod names;
mod rng;

use fps_flow::{FnCall, FnError, FnErrorKind, TemplateFn};
use rand::RngCore;

pub use catalog::{DefaultValue, FunctionSpec, ParamSpec, FUNCTIONS, MAX_RECIPIENTS};

/// Name of the module that this crate implements.
pub const MODULE_NAME: &str = "smtp";

/// The template functions of the `smtp` module for one session.
///
/// The functions keep no state between calls, so one value may serve any
/// number of sessions.
#[derive(Clone, Copy, Debug, Default)]
pub struct Smtp;

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

/// Checks a domain or host name argument: printable ASCII without spaces.
fn check_name(function: &str, param: &str, value: &str) -> Result<(), FnError> {
    if value.is_empty() || !value.bytes().all(|b| b.is_ascii_graphic()) {
        return Err(invalid(format!(
            "'{param}' of '{function}' must be non empty printable ASCII without spaces"
        )));
    }
    Ok(())
}

fn count_arg(function: &str, param: &str, n: i64, min: i64) -> Result<i64, FnError> {
    if (min..=catalog::MAX_RECIPIENTS).contains(&n) {
        Ok(n)
    } else {
        Err(invalid(format!(
            "'{param}' of '{function}' must lie between {min} and {}, not {n}",
            catalog::MAX_RECIPIENTS
        )))
    }
}

fn push(out: &mut Vec<u8>, text: &str) {
    out.extend_from_slice(text.as_bytes());
}

fn ehlo_reply(a: &args::Args<'_>, out: &mut Vec<u8>) -> Result<(), FnError> {
    let domain = a.text("domain")?;
    let client = a.text("client")?;
    check_name("ehlo_reply", "domain", domain)?;
    check_name("ehlo_reply", "client", client)?;
    let size = a.int("size")?;
    if size < 0 {
        return Err(invalid(format!(
            "'size' of 'ehlo_reply' must not be negative, not {size}"
        )));
    }
    let auth = a.text("auth")?;
    let mechanisms: Vec<&str> = auth.split_whitespace().collect();
    let token = |m: &&str| {
        m.bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
    };
    if !mechanisms.iter().all(token) {
        return Err(invalid(format!(
            "'auth' of 'ehlo_reply' must list upper case mechanism names, not '{auth}'"
        )));
    }
    // Every line but the last has a hyphen after the code, and the last line
    // has a space, so a reader finds the end of the reply.
    let mut lines = vec![format!("{domain} Hello {client}")];
    if a.boolean("pipelining")? {
        lines.push("PIPELINING".into());
    }
    lines.push(format!("SIZE {size}"));
    lines.push("ETRN".into());
    // Postfix lists STARTTLS between ETRN and AUTH. A server that offers it
    // leaves AUTH out of this reply, and a scenario shows the same server
    // after the upgrade with AUTH and without STARTTLS.
    if a.boolean("starttls")? {
        lines.push("STARTTLS".into());
    }
    if !mechanisms.is_empty() {
        lines.push(format!("AUTH {}", mechanisms.join(" ")));
    }
    lines.push("8BITMIME".into());
    lines.push("ENHANCEDSTATUSCODES".into());
    lines.push("SMTPUTF8".into());
    let last = lines.len() - 1;
    for (i, l) in lines.iter().enumerate() {
        push(out, if i == last { "250 " } else { "250-" });
        push(out, l);
        push(out, "\r\n");
    }
    Ok(())
}

impl TemplateFn for Smtp {
    fn render(
        &mut self,
        call: &FnCall<'_>,
        rng: &mut dyn RngCore,
        out: &mut Vec<u8>,
    ) -> Result<(), FnError> {
        if call.module != MODULE_NAME {
            return Err(FnError::new(
                FnErrorKind::Unsupported,
                format!("module '{}' is not smtp", call.module),
            ));
        }
        match call.function {
            "credential" => {
                let a = args::bind(&catalog::CREDENTIAL, &call.args)?;
                let mut r = derive("credential", a.int("seed")?, rng);
                match a.text("field")? {
                    "user" => push(out, &names::address(r.next_u64() as i64, 1, "example.test")),
                    "password" => push(out, &names::password(&mut r)),
                    other => {
                        return Err(invalid(format!(
                            "field '{other}' of 'credential' is not supported, use user or password"
                        )))
                    }
                }
            }
            "address" => {
                let a = args::bind(&catalog::ADDRESS, &call.args)?;
                let domain = a.text("domain")?;
                check_name("address", "domain", domain)?;
                push(
                    out,
                    &names::address(a.int("seed")?, a.int("index")?, domain),
                );
            }
            "recipients" => {
                let a = args::bind(&catalog::RECIPIENTS, &call.args)?;
                let domain = a.text("domain")?;
                check_name("recipients", "domain", domain)?;
                let count = count_arg("recipients", "count", a.int("count")?, 1)?;
                let seed = a.int("seed")?;
                for i in 1..=count {
                    if i > 1 {
                        push(out, ", ");
                    }
                    push(out, &names::address(seed, i, domain));
                }
            }
            "rcpt_to" => {
                let a = args::bind(&catalog::RCPT_TO, &call.args)?;
                let domain = a.text("domain")?;
                check_name("rcpt_to", "domain", domain)?;
                let count = count_arg("rcpt_to", "count", a.int("count")?, 1)?;
                let start = count_arg("rcpt_to", "start", a.int("start")?, 1)?;
                let seed = a.int("seed")?;
                for i in start..start + count {
                    push(
                        out,
                        &format!("RCPT TO:<{}>\r\n", names::address(seed, i, domain)),
                    );
                }
            }
            "auth_plain" => {
                let a = args::bind(&catalog::AUTH_PLAIN, &call.args)?;
                let (authzid, user, password) =
                    (a.text("authzid")?, a.text("user")?, a.text("password")?);
                for (name, value) in [("authzid", authzid), ("user", user), ("password", password)]
                {
                    if value.contains('\0') {
                        return Err(invalid(format!(
                            "'{name}' of 'auth_plain' must not contain a NUL byte"
                        )));
                    }
                }
                if user.is_empty() || password.is_empty() {
                    return Err(invalid(
                        "'user' and 'password' of 'auth_plain' must not be empty".into(),
                    ));
                }
                // RFC 4616: [authzid] NUL authcid NUL passwd.
                let token = format!("{authzid}\0{user}\0{password}");
                mime::base64(token.as_bytes(), 0, out);
            }
            "base64" => {
                let a = args::bind(&catalog::BASE64, &call.args)?;
                let wrap = a.int("wrap")?;
                if !(0..=998).contains(&wrap) || (wrap > 0 && wrap % 4 != 0) {
                    return Err(invalid(format!(
                        "'wrap' of 'base64' must be zero or a multiple of 4 up to 996, not {wrap}"
                    )));
                }
                mime::base64(a.bytes("data")?, wrap as usize, out);
            }
            "message" => {
                let a = args::bind(&catalog::MESSAGE, &call.args)?;
                mime::Message {
                    sender: a.utf8("sender")?,
                    to: a.utf8("to")?,
                    cc: a.utf8("cc")?,
                    subject: a.text("subject")?,
                    body: a.bytes("body")?,
                    attachment: a.bytes("attachment")?,
                    filename: a.text("filename")?,
                    content_type: a.text("content_type")?,
                    mailer: a.text("mailer")?,
                }
                .render(rng, out)?;
            }
            "data" => {
                let a = args::bind(&catalog::DATA, &call.args)?;
                mime::data(a.bytes("message")?, out)?;
            }
            "ehlo_reply" => {
                let a = args::bind(&catalog::EHLO_REPLY, &call.args)?;
                ehlo_reply(&a, out)?;
            }
            "repeat_line" => {
                let a = args::bind(&catalog::REPEAT_LINE, &call.args)?;
                let line = a.text("line")?;
                if line.bytes().any(|b| b == b'\r' || b == b'\n' || b == 0) {
                    return Err(invalid(
                        "'line' of 'repeat_line' must not contain a line break or a NUL byte"
                            .into(),
                    ));
                }
                for _ in 0..count_arg("repeat_line", "count", a.int("count")?, 0)? {
                    push(out, line);
                    push(out, "\r\n");
                }
            }
            other => {
                return Err(FnError::new(
                    FnErrorKind::Unsupported,
                    format!("smtp has no function '{other}'"),
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
            module: "smtp",
            function,
            args: CallArgs { positional, named },
        };
        Smtp.render(&c, &mut rng, &mut out)
            .map(|()| String::from_utf8(out).unwrap())
    }

    #[test]
    fn module_name_is_set() {
        assert_eq!(MODULE_NAME, "smtp");
    }

    #[test]
    fn rejects_other_modules_and_functions() {
        let mut rng = rand_pcg::Pcg64Mcg::new(1);
        let c = FnCall {
            module: "dns",
            function: "data",
            args: CallArgs {
                positional: &[],
                named: &[],
            },
        };
        assert_eq!(
            Smtp.render(&c, &mut rng, &mut Vec::new()).unwrap_err().kind,
            FnErrorKind::Unsupported
        );
        assert_eq!(
            render("vrfy", &[], &[]).unwrap_err().kind,
            FnErrorKind::Unsupported
        );
    }

    #[test]
    fn auth_plain_matches_the_rfc_4616_layout() {
        // base64 of NUL "tim" NUL "tanstaaftanstaaf", the example of RFC 4616.
        let s = render(
            "auth_plain",
            &[ArgValue::Text("tim"), ArgValue::Text("tanstaaftanstaaf")],
            &[],
        )
        .unwrap();
        assert_eq!(s, "AHRpbQB0YW5zdGFhZnRhbnN0YWFm");
        let s = render(
            "auth_plain",
            &[
                ArgValue::Text("tim"),
                ArgValue::Text("pw"),
                ArgValue::Text("boss"),
            ],
            &[],
        )
        .unwrap();
        assert_eq!(s, "Ym9zcwB0aW0AcHc=");
        assert!(render(
            "auth_plain",
            &[ArgValue::Text("a\0"), ArgValue::Text("b")],
            &[]
        )
        .is_err());
        assert!(render(
            "auth_plain",
            &[ArgValue::Text(""), ArgValue::Text("b")],
            &[]
        )
        .is_err());
    }

    #[test]
    fn ehlo_reply_is_a_well_formed_multi_line_reply() {
        let s = render(
            "ehlo_reply",
            &[
                ArgValue::Text("mail.example.test"),
                ArgValue::Text("client.example.test"),
            ],
            &[NamedArg {
                name: "auth",
                value: ArgValue::Text("PLAIN LOGIN"),
            }],
        )
        .unwrap();
        let lines: Vec<&str> = s.split_terminator("\r\n").collect();
        assert_eq!(lines[0], "250-mail.example.test Hello client.example.test");
        assert!(lines.contains(&"250-PIPELINING"));
        assert!(lines.contains(&"250-AUTH PLAIN LOGIN"));
        assert!(lines.last().unwrap().starts_with("250 "));
        assert_eq!(lines.iter().filter(|l| l.starts_with("250 ")).count(), 1);
        let plain = render(
            "ehlo_reply",
            &[ArgValue::Text("m.test"), ArgValue::Text("c.test")],
            &[
                NamedArg {
                    name: "pipelining",
                    value: ArgValue::Boolean(false),
                },
                NamedArg {
                    name: "size",
                    value: ArgValue::Int(1000),
                },
            ],
        )
        .unwrap();
        assert!(!plain.contains("PIPELINING") && !plain.contains("AUTH"));
        assert!(plain.contains("250-SIZE 1000\r\n"));
        assert!(!plain.contains("STARTTLS"));
        assert!(render(
            "ehlo_reply",
            &[ArgValue::Text("m.test"), ArgValue::Text("c.test")],
            &[NamedArg {
                name: "auth",
                value: ArgValue::Text("plain\r\n")
            }]
        )
        .is_err());
        assert!(render(
            "ehlo_reply",
            &[ArgValue::Text("m t"), ArgValue::Text("c.test")],
            &[]
        )
        .is_err());
    }

    #[test]
    fn ehlo_reply_offers_starttls_between_etrn_and_auth() {
        let s = render(
            "ehlo_reply",
            &[ArgValue::Text("mx.example.test"), ArgValue::Text("c.test")],
            &[NamedArg {
                name: "starttls",
                value: ArgValue::Boolean(true),
            }],
        )
        .unwrap();
        let lines: Vec<&str> = s.split_terminator("\r\n").collect();
        let at = |needle: &str| lines.iter().position(|l| l.contains(needle)).unwrap();
        assert_eq!(lines[at("STARTTLS")], "250-STARTTLS");
        assert_eq!(at("STARTTLS"), at("ETRN") + 1);
        assert!(lines.last().unwrap().starts_with("250 "));
        // With a mechanism list the line follows STARTTLS, in the order of Postfix.
        let both = render(
            "ehlo_reply",
            &[ArgValue::Text("mx.example.test"), ArgValue::Text("c.test")],
            &[
                NamedArg {
                    name: "starttls",
                    value: ArgValue::Boolean(true),
                },
                NamedArg {
                    name: "auth",
                    value: ArgValue::Text("PLAIN"),
                },
            ],
        )
        .unwrap();
        let lines: Vec<&str> = both.split_terminator("\r\n").collect();
        let at = |needle: &str| lines.iter().position(|l| l.contains(needle)).unwrap();
        assert_eq!(at("AUTH PLAIN"), at("STARTTLS") + 1);
    }

    #[test]
    fn rcpt_to_renders_one_line_per_recipient() {
        let s = render("rcpt_to", &[ArgValue::Int(5), ArgValue::Int(3)], &[]).unwrap();
        let lines: Vec<&str> = s.split_terminator("\r\n").collect();
        assert_eq!(lines.len(), 3);
        assert!(lines
            .iter()
            .all(|l| l.starts_with("RCPT TO:<") && l.ends_with("@example.test>")));
        // The list of a To header names the same addresses.
        let list = render("recipients", &[ArgValue::Int(5), ArgValue::Int(3)], &[]).unwrap();
        for (l, addr) in lines.iter().zip(list.split(", ")) {
            assert_eq!(*l, format!("RCPT TO:<{addr}>"));
        }
        // start picks up the sequence where an earlier call stopped.
        let tail = render(
            "rcpt_to",
            &[ArgValue::Int(5)],
            &[NamedArg {
                name: "start",
                value: ArgValue::Int(3),
            }],
        )
        .unwrap();
        assert_eq!(tail, format!("{}\r\n", lines[2]));
        assert!(render("rcpt_to", &[ArgValue::Int(5), ArgValue::Int(0)], &[]).is_err());
        assert!(render("rcpt_to", &[ArgValue::Int(5), ArgValue::Int(1001)], &[]).is_err());
    }

    #[test]
    fn credentials_follow_the_seed_and_stay_synthetic() {
        let user = |seed| {
            render(
                "credential",
                &[ArgValue::Text("user")],
                &[NamedArg {
                    name: "seed",
                    value: ArgValue::Int(seed),
                }],
            )
            .unwrap()
        };
        assert_eq!(user(3), user(3));
        assert_ne!(user(3), user(4));
        assert!(user(3).ends_with("@example.test"));
        let pw = render("credential", &[ArgValue::Text("password")], &[]).unwrap();
        assert!(pw.starts_with("Tst-"));
        assert!(render("credential", &[ArgValue::Text("token")], &[]).is_err());
    }

    #[test]
    fn repeat_line_writes_the_line_count_times() {
        let s = render(
            "repeat_line",
            &[ArgValue::Text("250 2.1.5 Ok"), ArgValue::Int(3)],
            &[],
        )
        .unwrap();
        assert_eq!(s, "250 2.1.5 Ok\r\n250 2.1.5 Ok\r\n250 2.1.5 Ok\r\n");
        let none = render("repeat_line", &[ArgValue::Text("x"), ArgValue::Int(0)], &[]).unwrap();
        assert_eq!(none, "");
        for bad in [
            [ArgValue::Text("a\r\nb"), ArgValue::Int(1)],
            [ArgValue::Text("a"), ArgValue::Int(-1)],
            [ArgValue::Text("a"), ArgValue::Int(1001)],
        ] {
            assert_eq!(
                render("repeat_line", &bad, &[]).unwrap_err().kind,
                FnErrorKind::InvalidArgument
            );
        }
    }

    #[test]
    fn base64_validates_the_wrap_width() {
        assert_eq!(
            render("base64", &[ArgValue::Text("foobar")], &[]).unwrap(),
            "Zm9vYmFy"
        );
        let wrapped = render("base64", &[ArgValue::Text("foobar"), ArgValue::Int(4)], &[]).unwrap();
        assert_eq!(wrapped, "Zm9v\r\nYmFy");
        assert!(render("base64", &[ArgValue::Text("x"), ArgValue::Int(5)], &[]).is_err());
        assert!(render("base64", &[ArgValue::Text("x"), ArgValue::Int(-4)], &[]).is_err());
    }

    #[test]
    fn data_and_message_compose() {
        let m = render(
            "message",
            &[
                ArgValue::Text("a.b@example.test"),
                ArgValue::Text("c.d@example.test"),
                ArgValue::Text("Hi"),
                ArgValue::Text(".starts with a dot\n"),
            ],
            &[],
        )
        .unwrap();
        let d = render("data", &[ArgValue::Text(&m)], &[]).unwrap();
        assert!(d.contains("\r\n..starts with a dot\r\n"));
        assert!(d.ends_with("\r\n.\r\n"));
        assert_eq!(d.matches("\r\n.\r\n").count(), 1);
    }
}
