// SPDX-License-Identifier: GPL-3.0-only
//! Native template functions of the `content` module.
//!
//! Decision D10 lets first party modules implement their template functions in
//! Rust. This crate implements `content.html`, `content.json`, `content.text`,
//! `content.email`, and `content.binary` behind the [`TemplateFn`] trait of
//! `fps-flow`, so the interpreter renders `{{ content.html(size=8192) }}`
//! without a WebAssembly runtime. Other modules call these functions to fill
//! message bodies, such as the body of an HTTP response or an email.
//!
//! Every generator is synthetic. It builds its output from word lists and
//! grammars that this crate holds, reads no file, opens no connection, and
//! knows no real person, place, or organisation.
//!
//! Each function takes a `size` in bytes and a `seed`. The output has at least
//! `size` bytes, and at most about a kilobyte more, because a generator
//! finishes the element that it writes when it reaches the size. The `bin`
//! kind of `content.binary` has exactly `size` bytes. A negative `seed` draws
//! the seed from the session generator, so the output depends only on the run
//! seed and the session id, and a replay reproduces it. A seed of zero or more
//! fixes the output for every session.
#![forbid(unsafe_code)]
#![warn(missing_docs, clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod args;
mod binary;
pub mod catalog;
mod email;
mod html;
mod json;
mod prose;
mod rng;
mod words;

use fps_flow::{FnCall, FnError, FnErrorKind, TemplateFn};
use rand::RngCore;

pub use catalog::{DefaultValue, FunctionSpec, ParamSpec, FUNCTIONS, MAX_SIZE};

/// Name of the module that this crate implements.
pub const MODULE_NAME: &str = "content";

/// The language codes that `content.text` and `content.email` accept.
pub const LANGUAGES: &[&str] = &["en", "de", "fr", "es", "it", "ru", "zh"];

/// The file kinds that `content.binary` accepts.
pub const BINARY_KINDS: &[&str] = &["png", "gif", "jpeg", "pdf", "zip", "gzip", "bin"];

/// The template functions of the module. The value holds no state, so every
/// session can own a copy.
#[derive(Clone, Copy, Debug, Default)]
pub struct Content;

fn invalid(message: String) -> FnError {
    FnError::new(FnErrorKind::InvalidArgument, message)
}

fn size_arg(function: &str, size: i64) -> Result<usize, FnError> {
    if (0..=MAX_SIZE).contains(&size) {
        usize::try_from(size).map_err(|_| invalid(format!("size of '{function}' is too large")))
    } else {
        Err(invalid(format!(
            "size of '{function}' must lie between 0 and {MAX_SIZE}, not {size}"
        )))
    }
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

impl Content {
    /// Finds a language by code or fails with the supported codes.
    fn lang(code: &str) -> Result<&'static prose::Lang, FnError> {
        prose::lang(code).ok_or_else(|| {
            invalid(format!(
                "language '{code}' is not supported, use one of {}",
                LANGUAGES.join(", ")
            ))
        })
    }
}

impl TemplateFn for Content {
    fn render(
        &mut self,
        call: &FnCall<'_>,
        rng: &mut dyn RngCore,
        out: &mut Vec<u8>,
    ) -> Result<(), FnError> {
        if call.module != MODULE_NAME {
            return Err(FnError::new(
                FnErrorKind::Unsupported,
                format!("module '{}' is not content", call.module),
            ));
        }
        match call.function {
            "html" => {
                let a = args::bind(&catalog::HTML, &call.args)?;
                let size = size_arg("html", a.int("size")?)?;
                let mut r = derive("html", a.int("seed")?, rng);
                out.extend_from_slice(&html::generate(&mut r, size));
            }
            "json" => {
                let a = args::bind(&catalog::JSON, &call.args)?;
                let size = size_arg("json", a.int("size")?)?;
                let mut r = derive("json", a.int("seed")?, rng);
                out.extend_from_slice(&json::generate(&mut r, size));
            }
            "text" => {
                let a = args::bind(&catalog::TEXT, &call.args)?;
                let lang = Self::lang(a.text("lang")?)?;
                let size = size_arg("text", a.int("size")?)?;
                let mut r = derive("text", a.int("seed")?, rng);
                out.extend_from_slice(lang.prose(&mut r, size).as_bytes());
            }
            "email" => {
                let a = args::bind(&catalog::EMAIL, &call.args)?;
                let lang = Self::lang(a.text("lang")?)?;
                let size = size_arg("email", a.int("size")?)?;
                let mut r = derive("email", a.int("seed")?, rng);
                out.extend_from_slice(&email::generate(&mut r, lang, size));
            }
            "binary" => {
                let a = args::bind(&catalog::BINARY, &call.args)?;
                let kind = a.text("kind")?;
                let size = size_arg("binary", a.int("size")?)?;
                let mut r = derive("binary", a.int("seed")?, rng);
                let file = binary::generate(kind, r.next_u64(), size).ok_or_else(|| {
                    invalid(format!(
                        "kind '{kind}' is not supported, use one of {}",
                        BINARY_KINDS.join(", ")
                    ))
                })?;
                out.extend_from_slice(&file);
            }
            other => {
                return Err(FnError::new(
                    FnErrorKind::Unsupported,
                    format!("content has no function '{other}'"),
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
        rng_seed: u64,
        named: &[NamedArg<'_>],
    ) -> Result<Vec<u8>, FnError> {
        let mut rng = rand_pcg::Pcg64Mcg::new(u128::from(rng_seed));
        let mut out = Vec::new();
        let c = FnCall {
            module: "content",
            function,
            args: CallArgs {
                positional: &[],
                named,
            },
        };
        Content.render(&c, &mut rng, &mut out).map(|()| out)
    }

    fn kw<'a>(name: &'a str, value: ArgValue<'a>) -> NamedArg<'a> {
        NamedArg { name, value }
    }

    #[test]
    fn lists_match_the_internal_tables() {
        let codes: Vec<&str> = prose::LANGS.iter().map(|l| l.code).collect();
        assert_eq!(codes, LANGUAGES);
    }

    #[test]
    fn rejects_other_modules_and_functions() {
        let mut rng = rand_pcg::Pcg64Mcg::new(1);
        let c = FnCall {
            module: "dns",
            function: "html",
            args: CallArgs {
                positional: &[],
                named: &[],
            },
        };
        let err = Content.render(&c, &mut rng, &mut Vec::new()).unwrap_err();
        assert_eq!(err.kind, FnErrorKind::Unsupported);
        let err = render("video", 1, &[]).unwrap_err();
        assert_eq!(err.kind, FnErrorKind::Unsupported);
    }

    #[test]
    fn rejects_bad_arguments() {
        for named in [
            vec![kw("size", ArgValue::Int(-1))],
            vec![kw("size", ArgValue::Int(MAX_SIZE + 1))],
            vec![kw("size", ArgValue::Text("big"))],
            vec![kw("color", ArgValue::Int(1))],
        ] {
            let err = render("html", 1, &named).unwrap_err();
            assert_eq!(err.kind, FnErrorKind::InvalidArgument);
        }
        let err = render("text", 1, &[kw("lang", ArgValue::Text("xx"))]).unwrap_err();
        assert!(err.message.contains("not supported"), "{err}");
        let err = render("binary", 1, &[kw("kind", ArgValue::Text("exe"))]).unwrap_err();
        assert!(err.message.contains("not supported"), "{err}");
    }

    #[test]
    fn a_negative_seed_follows_the_session_generator() {
        let a = render("json", 1, &[]).unwrap();
        assert_eq!(a, render("json", 1, &[]).unwrap());
        assert_ne!(a, render("json", 2, &[]).unwrap());
    }

    #[test]
    fn a_fixed_seed_ignores_the_session_generator() {
        let s = [kw("seed", ArgValue::Int(9))];
        assert_eq!(
            render("html", 1, &s).unwrap(),
            render("html", 2, &s).unwrap()
        );
    }
}
