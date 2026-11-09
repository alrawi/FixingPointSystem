// SPDX-License-Identifier: GPL-3.0-only
//! Native template functions of the `pop3` module.
//!
//! Decision D10 lets first party modules implement their template functions in
//! Rust. This crate implements the `pop3` functions behind the [`TemplateFn`]
//! trait of `fps-flow`, so the interpreter renders
//! `{{ pop3.retr_reply(last, vars.msg, vars.messages) }}` without a
//! WebAssembly runtime. The functions render the parts of a POP3 dialogue that
//! a scenario cannot write as plain text: the timestamp banner of the greeting
//! and the `APOP` digest that the client computes from it, the multi line
//! answers to `LIST`, `UIDL`, `RETR`, and `TOP` with the dot stuffing of RFC
//! 1939, and the MIME messages that `RETR` carries.
//!
//! [`FUNCTIONS`] is the catalog that `modules/pop3/manifest.yaml` lists under
//! `functions`. The catalog test compares the two, so the manifest stays the
//! single catalog that the validator and the controller read.
#![forbid(unsafe_code)]
#![warn(missing_docs, clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod args;
pub mod catalog;
mod md5;
mod mime;
mod names;
mod reply;
mod rng;

use fps_flow::{FnCall, FnError, FnErrorKind, TemplateFn};
use rand::RngCore;

pub use catalog::{DefaultValue, FunctionSpec, ParamSpec, FUNCTIONS, MAX_MESSAGES, UID_BASE};

/// Name of the module that this crate implements.
pub const MODULE_NAME: &str = "pop3";

/// The template functions of the `pop3` module for one session.
///
/// The functions keep no state between calls, so one value may serve any
/// number of sessions.
#[derive(Clone, Copy, Debug, Default)]
pub struct Pop3;

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
    if value.is_empty()
        || !value
            .bytes()
            .all(|b| b.is_ascii_graphic() && b != b'<' && b != b'>')
    {
        return Err(invalid(format!(
            "'{param}' of '{function}' must be non empty printable ASCII without spaces or angle brackets"
        )));
    }
    Ok(())
}

fn push(out: &mut Vec<u8>, text: &str) {
    out.extend_from_slice(text.as_bytes());
}

impl TemplateFn for Pop3 {
    fn render(
        &mut self,
        call: &FnCall<'_>,
        rng: &mut dyn RngCore,
        out: &mut Vec<u8>,
    ) -> Result<(), FnError> {
        if call.module != MODULE_NAME {
            return Err(FnError::new(
                FnErrorKind::Unsupported,
                format!("module '{}' is not pop3", call.module),
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
            "banner" => {
                let a = args::bind(&catalog::BANNER, &call.args)?;
                let host = a.text("host")?;
                check_name("banner", "host", host)?;
                let mut r = derive("banner", a.int("seed")?, rng);
                push(out, &reply::banner(host, &mut r));
            }
            "apop_digest" => {
                let a = args::bind(&catalog::APOP_DIGEST, &call.args)?;
                push(
                    out,
                    &reply::apop_digest(a.bytes("greeting")?, a.text("secret")?)?,
                );
            }
            "dot_stuff" => {
                let a = args::bind(&catalog::DOT_STUFF, &call.args)?;
                reply::dot_stuff(a.bytes("data")?, out);
            }
            "stat_reply" => {
                let a = args::bind(&catalog::STAT_REPLY, &call.args)?;
                reply::stat(a.int("count")?, a.int("box")?, out)?;
            }
            "list_reply" => {
                let a = args::bind(&catalog::LIST_REPLY, &call.args)?;
                reply::list(a.bytes("command")?, a.int("count")?, a.int("box")?, out)?;
            }
            "uidl_reply" => {
                let a = args::bind(&catalog::UIDL_REPLY, &call.args)?;
                reply::uidl(a.bytes("command")?, a.int("count")?, a.int("box")?, out)?;
            }
            "retr_reply" => {
                let a = args::bind(&catalog::RETR_REPLY, &call.args)?;
                reply::retr(
                    a.bytes("command")?,
                    a.bytes("message")?,
                    a.int("count")?,
                    out,
                )?;
            }
            "top_reply" => {
                let a = args::bind(&catalog::TOP_REPLY, &call.args)?;
                reply::top(
                    a.bytes("command")?,
                    a.bytes("message")?,
                    a.int("count")?,
                    out,
                )?;
            }
            "dele_reply" => {
                let a = args::bind(&catalog::DELE_REPLY, &call.args)?;
                reply::dele(a.bytes("command")?, a.int("count")?, out)?;
            }
            other => {
                return Err(FnError::new(
                    FnErrorKind::Unsupported,
                    format!("pop3 has no function '{other}'"),
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
            module: "pop3",
            function,
            args: CallArgs { positional, named },
        };
        Pop3.render(&c, &mut rng, &mut out)
            .map(|()| String::from_utf8(out).unwrap())
    }

    fn text(t: &str) -> ArgValue<'_> {
        ArgValue::Bytes(t.as_bytes())
    }

    #[test]
    fn module_name_is_set() {
        assert_eq!(MODULE_NAME, "pop3");
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
            Pop3.render(&c, &mut rng, &mut Vec::new()).unwrap_err().kind,
            FnErrorKind::Unsupported
        );
        assert_eq!(
            render("idle", &[], &[]).unwrap_err().kind,
            FnErrorKind::Unsupported
        );
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
    fn the_banner_follows_the_seed_and_the_digest_follows_the_banner() {
        let named = |seed| {
            [NamedArg {
                name: "seed",
                value: ArgValue::Int(seed),
            }]
        };
        let b = |seed| {
            render(
                "banner",
                &[ArgValue::Text("pop3.example.test")],
                &named(seed),
            )
        };
        assert_eq!(b(5).unwrap(), b(5).unwrap());
        assert_ne!(b(5).unwrap(), b(6).unwrap());
        // Without a seed the session generator draws it.
        let drawn = render("banner", &[ArgValue::Text("pop3.example.test")], &[]).unwrap();
        assert!(drawn.ends_with("@pop3.example.test>"));
        assert!(render("banner", &[ArgValue::Text("a b")], &[]).is_err());
        assert!(render("banner", &[ArgValue::Text("a>b")], &[]).is_err());
        let greeting = format!("+OK Dovecot ready. {drawn}\r\n");
        let d = render(
            "apop_digest",
            &[text(&greeting), ArgValue::Text("Tst-Kamurai4821")],
            &[],
        )
        .unwrap();
        assert_eq!(d.len(), 32);
        assert!(d
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()));
    }

    #[test]
    fn a_retrieved_message_carries_its_size_and_the_closing_line() {
        let m = render(
            "message",
            &[
                ArgValue::Text("a.b@example.test"),
                ArgValue::Text("c.d@example.test"),
                ArgValue::Text("Hi"),
                ArgValue::Text("hello\n.\nbye\n"),
            ],
            &[],
        )
        .unwrap();
        let r = render(
            "retr_reply",
            &[text("RETR 2\r\n"), text(&m), ArgValue::Int(3)],
            &[],
        )
        .unwrap();
        assert!(r.starts_with(&format!(
            "+OK {} octets\r\nFrom: a.b@example.test\r\n",
            m.len()
        )));
        assert!(r.contains("\r\nhello\r\n..\r\nbye\r\n"), "{r}");
        assert!(r.ends_with("bye\r\n.\r\n"));
        // A command that is not RETR fails the call.
        assert!(render(
            "retr_reply",
            &[text("DELE 2\r\n"), text(&m), ArgValue::Int(3)],
            &[]
        )
        .is_err());
        let t = render(
            "top_reply",
            &[text("TOP 1 1\r\n"), text(&m), ArgValue::Int(3)],
            &[],
        )
        .unwrap();
        assert!(t.starts_with("+OK\r\nFrom: "));
        assert!(t.ends_with("\r\n\r\nhello\r\n.\r\n"), "{t}");
    }

    #[test]
    fn mailbox_replies_need_a_valid_count() {
        for count in [-1, MAX_MESSAGES + 1] {
            let e =
                render("stat_reply", &[ArgValue::Int(count), ArgValue::Int(1)], &[]).unwrap_err();
            assert_eq!(e.kind, FnErrorKind::InvalidArgument);
        }
        let s = render("stat_reply", &[ArgValue::Int(2), ArgValue::Int(1)], &[]).unwrap();
        assert!(s.starts_with("+OK 2 ") && s.ends_with("\r\n"), "{s}");
        assert_eq!(
            render("dot_stuff", &[text(".a\r\n")], &[]).unwrap(),
            "..a\r\n.\r\n"
        );
        assert_eq!(render("next", &[ArgValue::Int(6)], &[]).unwrap(), "7");
    }
}
