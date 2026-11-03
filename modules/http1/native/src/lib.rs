// SPDX-License-Identifier: GPL-3.0-only
//! Native template functions of the `http1` module.
//!
//! Decision D10 lets first party modules implement their template functions in
//! Rust. This crate implements `http1.request` and `http1.response` behind the
//! [`TemplateFn`] trait of `fps-flow`, so the interpreter renders
//! `{{ http1.response(200, body=fill(vars.n)) }}` without a WebAssembly
//! runtime. Both functions build HTTP/1.0 and HTTP/1.1 messages as RFC 9112
//! frames them, with a length delimited body or with chunked transfer coding.
//!
//! [`FUNCTIONS`] is the catalog that `modules/http1/manifest.yaml` lists under
//! `functions`. The catalog test compares the two, so the manifest stays the
//! single catalog that the validator and the controller read.
#![forbid(unsafe_code)]
#![warn(missing_docs, clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod args;
pub mod catalog;
pub mod message;

use fps_flow::{FnCall, FnError, FnErrorKind, TemplateFn};
use rand::RngCore;

pub use catalog::{DefaultValue, FunctionSpec, ParamSpec, FUNCTIONS};
pub use message::{Framing, Request, Response, Version};

/// Name of the module that this crate implements.
pub const MODULE_NAME: &str = "http1";

/// The template functions of the `http1` module for one session.
///
/// The functions keep no state between calls and draw nothing from the
/// session generator, so one value may serve any number of sessions.
#[derive(Clone, Copy, Debug, Default)]
pub struct Http1;

impl TemplateFn for Http1 {
    fn render(
        &mut self,
        call: &FnCall<'_>,
        _rng: &mut dyn RngCore,
        out: &mut Vec<u8>,
    ) -> Result<(), FnError> {
        if call.module != MODULE_NAME {
            return Err(FnError::new(
                FnErrorKind::Unsupported,
                format!("module '{}' is not http1", call.module),
            ));
        }
        match call.function {
            "request" => {
                let a = args::bind(&catalog::REQUEST, &call.args)?;
                Request::from_args(&a)?.render(out)
            }
            "response" => {
                let a = args::bind(&catalog::RESPONSE, &call.args)?;
                Response::from_args(&a)?.render(out)
            }
            other => Err(FnError::new(
                FnErrorKind::Unsupported,
                format!("http1 has no function '{other}'"),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fps_flow::function::NamedArg;
    use fps_flow::{ArgValue, CallArgs};

    fn call(module: &'static str, function: &'static str) -> FnCall<'static> {
        FnCall {
            module,
            function,
            args: CallArgs {
                positional: &[],
                named: &[],
            },
        }
    }

    fn render(c: &FnCall<'_>) -> Result<Vec<u8>, FnError> {
        let mut rng = rand_pcg::Pcg64Mcg::new(42);
        let mut out = Vec::new();
        Http1.render(c, &mut rng, &mut out).map(|()| out)
    }

    #[test]
    fn module_name_is_set() {
        assert_eq!(MODULE_NAME, "http1");
    }

    #[test]
    fn rejects_other_modules_and_functions() {
        assert_eq!(
            render(&call("dns", "request")).unwrap_err().kind,
            FnErrorKind::Unsupported
        );
        assert_eq!(
            render(&call("http1", "parse")).unwrap_err().kind,
            FnErrorKind::Unsupported
        );
    }

    #[test]
    fn renders_a_call_with_positional_and_keyword_arguments() {
        let positional = [ArgValue::Text("GET"), ArgValue::Text("/a")];
        let named = [NamedArg {
            name: "host",
            value: ArgValue::Text("www.example.test"),
        }];
        let c = FnCall {
            module: "http1",
            function: "request",
            args: CallArgs {
                positional: &positional,
                named: &named,
            },
        };
        let out = render(&c).unwrap();
        assert!(out.starts_with(b"GET /a HTTP/1.1\r\nHost: www.example.test\r\n"));
        assert!(out.ends_with(b"\r\n\r\n"));
    }

    #[test]
    fn a_missing_required_argument_is_invalid() {
        let positional = [ArgValue::Text("GET")];
        let c = FnCall {
            module: "http1",
            function: "request",
            args: CallArgs {
                positional: &positional,
                named: &[],
            },
        };
        assert_eq!(render(&c).unwrap_err().kind, FnErrorKind::InvalidArgument);
    }
}
