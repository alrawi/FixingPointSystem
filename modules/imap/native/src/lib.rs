// SPDX-License-Identifier: GPL-3.0-only
//! Native template functions of the `imap` module.
//!
//! Decision D10 lets first party modules implement their template functions in
//! Rust. This crate implements the `imap` functions behind the [`TemplateFn`]
//! trait of `fps-flow`, so the interpreter renders
//! `{{ imap.fetch(last, vars.msg) }}` without a WebAssembly runtime. The
//! functions render the parts of an IMAP dialogue that a scenario cannot write
//! as plain text: the tag that follows the previous one, the tagged completion
//! of the command that the server just read, `SELECT` data, `FETCH` responses
//! with literals that carry a MIME message, `SEARCH` and `STORE` responses,
//! and the sizes of literals that the client announces.
//!
//! [`FUNCTIONS`] is the catalog that `modules/imap/manifest.yaml` lists under
//! `functions`. The catalog test compares the two, so the manifest stays the
//! single catalog that the validator and the controller read.
#![forbid(unsafe_code)]
#![warn(missing_docs, clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod args;
pub mod catalog;
mod mime;
mod names;
mod reply;
mod rng;

use fps_flow::{FnCall, FnError, FnErrorKind, TemplateFn};
use rand::RngCore;

pub use catalog::{DefaultValue, FunctionSpec, ParamSpec, FUNCTIONS, MAX_MESSAGES, UID_BASE};

/// Name of the module that this crate implements.
pub const MODULE_NAME: &str = "imap";

/// The template functions of the `imap` module for one session.
///
/// The functions keep no state between calls, so one value may serve any
/// number of sessions.
#[derive(Clone, Copy, Debug, Default)]
pub struct Imap;

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

/// Checks a domain name argument: printable ASCII without spaces.
fn check_name(function: &str, param: &str, value: &str) -> Result<(), FnError> {
    if value.is_empty() || !value.bytes().all(|b| b.is_ascii_graphic()) {
        return Err(invalid(format!(
            "'{param}' of '{function}' must be non empty printable ASCII without spaces"
        )));
    }
    Ok(())
}

/// Checks text that goes into a response line.
fn check_line(function: &str, param: &str, value: &str) -> Result<(), FnError> {
    if value.bytes().any(|b| b == b'\r' || b == b'\n' || b == 0) {
        return Err(invalid(format!(
            "'{param}' of '{function}' must not contain a line break or a NUL byte"
        )));
    }
    Ok(())
}

fn push(out: &mut Vec<u8>, text: &str) {
    out.extend_from_slice(text.as_bytes());
}

fn tagged(a: &args::Args<'_>, out: &mut Vec<u8>, rng: &mut dyn RngCore) -> Result<(), FnError> {
    let command = reply::parse_command("tagged", a.bytes("command")?)?;
    let status = a.text("status")?;
    if !matches!(status, "OK" | "NO" | "BAD") {
        return Err(invalid(format!(
            "'status' of 'tagged' must be OK, NO, or BAD, not '{status}'"
        )));
    }
    let (text, code) = (a.text("text")?, a.text("code")?);
    check_line("tagged", "text", text)?;
    check_line("tagged", "code", code)?;
    if code.bytes().any(|b| b == b']' || b == b'[') {
        return Err(invalid(
            "'code' of 'tagged' must not contain a bracket".into(),
        ));
    }
    push(out, command.tag);
    push(out, " ");
    push(out, status);
    if !code.is_empty() {
        push(out, &format!(" [{code}]"));
    }
    push(out, " ");
    push(out, text);
    if a.boolean("timing")? {
        // The timing suffix of Dovecot, in milliseconds: parse time plus
        // sorting time.
        let mut r = rng::Rng::new(rng.next_u64());
        let (a, b) = (r.range(1, 9), r.below(4));
        push(out, &format!(" (0.00{a} + 0.00{b} secs)."));
    }
    push(out, "\r\n");
    Ok(())
}

impl TemplateFn for Imap {
    fn render(
        &mut self,
        call: &FnCall<'_>,
        rng: &mut dyn RngCore,
        out: &mut Vec<u8>,
    ) -> Result<(), FnError> {
        if call.module != MODULE_NAME {
            return Err(FnError::new(
                FnErrorKind::Unsupported,
                format!("module '{}' is not imap", call.module),
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
            "next" => {
                let a = args::bind(&catalog::NEXT, &call.args)?;
                push(out, &reply::next(&a.any_text("value")?, a.int("step")?)?);
            }
            "literal_marker" => {
                let a = args::bind(&catalog::LITERAL_MARKER, &call.args)?;
                push(out, &format!("{{{}}}", a.bytes("data")?.len()));
            }
            "literal_size" => {
                let a = args::bind(&catalog::LITERAL_SIZE, &call.args)?;
                push(out, &reply::literal_size(a.bytes("command")?)?.to_string());
            }
            "tagged" => {
                let a = args::bind(&catalog::TAGGED, &call.args)?;
                tagged(&a, out, rng)?;
            }
            "select_data" => {
                let a = args::bind(&catalog::SELECT_DATA, &call.args)?;
                let validity = a.int("uidvalidity")?;
                reply::select_data(
                    a.int("exists")?,
                    a.int("recent")?,
                    a.int("unseen")?,
                    (validity >= 0).then_some(validity),
                    a.boolean("read_only")?,
                    rng.next_u64(),
                    out,
                )?;
            }
            "fetch" => {
                let a = args::bind(&catalog::FETCH, &call.args)?;
                reply::fetch(
                    a.bytes("command")?,
                    a.bytes("message")?,
                    a.int("exists")?,
                    out,
                )?;
            }
            "search_reply" => {
                let a = args::bind(&catalog::SEARCH_REPLY, &call.args)?;
                let mut r = rng::Rng::new(rng.next_u64());
                reply::search_reply(a.int("exists")?, a.int("hits")?, &mut r, out)?;
            }
            "search_hit" => {
                let a = args::bind(&catalog::SEARCH_HIT, &call.args)?;
                push(
                    out,
                    &reply::search_hit(a.bytes("response")?, a.int("index")?)?.to_string(),
                );
            }
            "store_reply" => {
                let a = args::bind(&catalog::STORE_REPLY, &call.args)?;
                reply::store(a.bytes("command")?, out)?;
            }
            "expunge_reply" => {
                let a = args::bind(&catalog::EXPUNGE_REPLY, &call.args)?;
                reply::expunge(a.bytes("stored")?, out)?;
            }
            other => {
                return Err(FnError::new(
                    FnErrorKind::Unsupported,
                    format!("imap has no function '{other}'"),
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
            module: "imap",
            function,
            args: CallArgs { positional, named },
        };
        Imap.render(&c, &mut rng, &mut out)
            .map(|()| String::from_utf8(out).unwrap())
    }

    fn text(t: &str) -> ArgValue<'_> {
        ArgValue::Bytes(t.as_bytes())
    }

    #[test]
    fn module_name_is_set() {
        assert_eq!(MODULE_NAME, "imap");
    }

    #[test]
    fn rejects_other_modules_and_functions() {
        let mut rng = rand_pcg::Pcg64Mcg::new(1);
        let c = FnCall {
            module: "smtp",
            function: "next",
            args: CallArgs {
                positional: &[],
                named: &[],
            },
        };
        assert_eq!(
            Imap.render(&c, &mut rng, &mut Vec::new()).unwrap_err().kind,
            FnErrorKind::Unsupported
        );
        assert_eq!(
            render("idle", &[], &[]).unwrap_err().kind,
            FnErrorKind::Unsupported
        );
    }

    #[test]
    fn auth_plain_matches_the_rfc_4616_layout() {
        let s = render(
            "auth_plain",
            &[ArgValue::Text("tim"), ArgValue::Text("tanstaaftanstaaf")],
            &[],
        )
        .unwrap();
        assert_eq!(s, "AHRpbQB0YW5zdGFhZnRhbnN0YWFm");
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
    fn tags_count_up_from_integers_text_and_bytes() {
        assert_eq!(
            render("next", &[ArgValue::Text("A000")], &[]).unwrap(),
            "A001"
        );
        assert_eq!(render("next", &[text("A009")], &[]).unwrap(), "A010");
        assert_eq!(render("next", &[ArgValue::Int(6)], &[]).unwrap(), "7");
        assert!(render("next", &[ArgValue::Boolean(true)], &[]).is_err());
    }

    #[test]
    fn the_tagged_completion_echoes_the_tag_of_the_command() {
        let s = render("tagged", &[text("A017 NOOP\r\n")], &[]).unwrap();
        assert_eq!(s, "A017 OK Completed\r\n");
        let s = render(
            "tagged",
            &[text("x9 SELECT INBOX\r\n")],
            &[
                NamedArg {
                    name: "text",
                    value: ArgValue::Text("Select completed"),
                },
                NamedArg {
                    name: "code",
                    value: ArgValue::Text("READ-WRITE"),
                },
                NamedArg {
                    name: "timing",
                    value: ArgValue::Boolean(true),
                },
            ],
        )
        .unwrap();
        assert!(
            s.starts_with("x9 OK [READ-WRITE] Select completed (0.00"),
            "{s}"
        );
        assert!(s.ends_with(" secs).\r\n"), "{s}");
        let no = render(
            "tagged",
            &[text("A1 NOOP\r\n")],
            &[NamedArg {
                name: "status",
                value: ArgValue::Text("NO"),
            }],
        )
        .unwrap();
        assert!(no.starts_with("A1 NO "));
        for (bad_cmd, status, code) in [
            ("A1\r\n", "OK", ""),
            ("A1 NOOP\r\n", "MAYBE", ""),
            ("A1 NOOP\r\n", "OK", "A]B"),
        ] {
            let r = render(
                "tagged",
                &[text(bad_cmd)],
                &[
                    NamedArg {
                        name: "status",
                        value: ArgValue::Text(status),
                    },
                    NamedArg {
                        name: "code",
                        value: ArgValue::Text(code),
                    },
                ],
            );
            assert!(r.is_err(), "{bad_cmd} {status} {code}");
        }
        let e = render(
            "tagged",
            &[text("A1 NOOP\r\n")],
            &[NamedArg {
                name: "text",
                value: ArgValue::Text("a\r\nb"),
            }],
        )
        .unwrap_err();
        assert_eq!(e.kind, FnErrorKind::InvalidArgument);
    }

    #[test]
    fn literals_announce_their_size_and_read_it_back() {
        assert_eq!(
            render("literal_marker", &[text("Tst-Kamurai4821")], &[]).unwrap(),
            "{15}"
        );
        assert_eq!(
            render("literal_size", &[text("A002 LOGIN u {15}\r\n")], &[]).unwrap(),
            "15"
        );
        assert_eq!(
            render("literal_size", &[text("A002 NOOP\r\n")], &[])
                .unwrap_err()
                .kind,
            FnErrorKind::Protocol
        );
    }

    #[test]
    fn a_message_fetches_as_a_literal_of_its_own_size() {
        let m = render(
            "message",
            &[
                ArgValue::Text("a.b@example.test"),
                ArgValue::Text("c.d@example.test"),
                ArgValue::Text("Hi"),
                ArgValue::Text("hello\n"),
            ],
            &[],
        )
        .unwrap();
        let f = render("fetch", &[text("A006 FETCH 2 (BODY[])\r\n"), text(&m)], &[]).unwrap();
        assert!(f.starts_with(&format!(
            "* 2 FETCH (BODY[] {{{}}}\r\nFrom: a.b@example.test\r\n",
            m.len()
        )));
        // Without the message, a command for body data fails the call.
        assert!(render("fetch", &[text("A006 FETCH 2 (BODY[])\r\n")], &[]).is_err());
    }
}
