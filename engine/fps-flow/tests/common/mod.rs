// SPDX-License-Identifier: GPL-3.0-only
//! Shared helpers of the template tests.

use std::sync::OnceLock;

use fps_flow::function::{ArgValue, FnCall, FnError, FnErrorKind, TemplateFn};
use fps_flow::template::{Filler, RenderCtx};
use fps_flow::{Template, TemplateError, Value, Vars};
use rand::RngCore;
use rand_pcg::Pcg64Mcg;

/// One filler for every case, because building it costs a random draw per byte.
pub fn filler() -> &'static Filler {
    static FILLER: OnceLock<Filler> = OnceLock::new();
    FILLER.get_or_init(|| Filler::new(7))
}

/// A module stand in that echoes every call with its evaluated arguments and
/// one number from the session generator, so tests see the argument order,
/// the value types, and the generator position.
pub struct Echo;

impl TemplateFn for Echo {
    fn render(
        &mut self,
        call: &FnCall<'_>,
        rng: &mut dyn RngCore,
        out: &mut Vec<u8>,
    ) -> Result<(), FnError> {
        if call.function == "fail" {
            return Err(FnError::new(
                FnErrorKind::InvalidArgument,
                "fail was called",
            ));
        }
        let mut args: Vec<String> = call.args.positional.iter().map(show).collect();
        args.extend(
            call.args
                .named
                .iter()
                .map(|n| format!("{}={}", n.name, show(&n.value))),
        );
        let text = format!(
            "<{}.{}({}) r{}>",
            call.module,
            call.function,
            args.join(", "),
            rng.next_u32() % 1000
        );
        out.extend_from_slice(text.as_bytes());
        Ok(())
    }
}

/// Shows an argument, with long text and bytes cut to their length.
fn show(v: &ArgValue<'_>) -> String {
    match v {
        ArgValue::Text(s) if s.len() > 24 => format!("Text(len {})", s.len()),
        ArgValue::Bytes(b) if b.len() > 24 => format!("Bytes(len {})", b.len()),
        ArgValue::Bytes(b) => format!("Bytes(\"{}\")", b.escape_ascii()),
        other => format!("{other:?}"),
    }
}

/// Scenario variables of the template tests.
pub fn vars() -> Vars {
    let mut v = Vars::new();
    v.insert("host".into(), Value::Str("www.example.test".into()));
    v.insert("size".into(), Value::Int(12));
    v.insert("ratio".into(), Value::Float(0.25));
    v.insert("tls".into(), Value::Bool(true));
    v.insert("digits".into(), Value::Str("cafe".into()));
    v
}

/// Returns the session generator for a run seed and session id, as the
/// coding standards define it.
pub fn session_rng(seed: u64, session_id: u64) -> Pcg64Mcg {
    Pcg64Mcg::new(u128::from(seed) << 64 | u128::from(session_id))
}

/// Renders a template for one session with [`Echo`] as the module table.
pub fn render(
    t: &Template,
    vars: &Vars,
    seed: u64,
    session_id: u64,
    last: &[u8],
) -> Result<Vec<u8>, TemplateError> {
    let overlay = Vars::new();
    let mut rng = session_rng(seed, session_id);
    let mut echo = Echo;
    let mut ctx = RenderCtx {
        vars,
        overlay: &overlay,
        session_id,
        flow: "page",
        last,
        rng: &mut rng,
        filler: filler(),
        functions: &mut echo,
    };
    t.render_to_vec(&mut ctx)
}
