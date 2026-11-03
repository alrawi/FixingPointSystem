// SPDX-License-Identifier: GPL-3.0-only
//! Property tests of the template compiler and renderer.

mod common;

use fps_flow::template::decode_hex;
use fps_flow::Template;

use proptest::prelude::*;

/// Placeholders that cover every expression form. Each one compiles, and each
/// one renders with the variables of `common::vars`.
const PIECES: &[&str] = &[
    "{{ vars.host }}",
    "{{ vars.size }}",
    "{{ vars.ratio }}",
    "{{ session.id }}",
    "{{ session.flow }}",
    "{{ last }}",
    "{{ 17 }}",
    "{{ -2.5 }}",
    "{{ true }}",
    "{{ 'lit' }}",
    "{{ hex('0d0a') }}",
    "{{ hex(vars.digits) }}",
    "{{ fill(vars.size) }}",
    "{{ fill(n=rand_int(0, 64)) }}",
    "{{ rand_int(0, 1000000) }}",
    "{{ rand_int(a=-5, b=vars.size) }}",
    "{{ http1.request('GET', '/', host=vars.host) }}",
    "{{ http1.response(200, body=fill(vars.size)) }}",
    "{{ dns.response(last, address=vars.host) }}",
    "{{ m.f(m.g(rand_int(1, 9)), k=session.id) }}",
    "plain text ",
    "\r\n",
];

/// Session ids are sequence numbers, which the contract types as `int`.
const SESSIONS: std::ops::RangeInclusive<u64> = 0..=(i64::MAX as u64);

/// A template built from a sequence of pieces.
fn template_source() -> impl Strategy<Value = String> {
    prop::collection::vec(prop::sample::select(PIECES), 0..12).prop_map(|p| p.concat())
}

proptest! {
    #[test]
    fn text_without_braces_renders_unchanged(text in "[^{}]{0,200}") {
        let t = Template::compile(&text).expect("compile");
        prop_assert!(t.is_static());
        let vars = common::vars();
        prop_assert_eq!(common::render(&t, &vars, 0, 0, b"").expect("render"), text.into_bytes());
    }

    #[test]
    fn hex_round_trips(bytes in prop::collection::vec(any::<u8>(), 0..128)) {
        let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
        prop_assert_eq!(decode_hex(&hex).expect("decode"), bytes);
    }

    /// The acceptance property of ENG-03: one seed and one session id always
    /// render the same bytes, module calls included.
    #[test]
    fn rendering_is_deterministic_for_a_seed_and_session(
        src in template_source(),
        seed in any::<u64>(),
        session in SESSIONS,
        last in prop::collection::vec(any::<u8>(), 0..32),
    ) {
        let t = Template::compile(&src).expect("compile");
        let vars = common::vars();
        let first = common::render(&t, &vars, seed, session, &last).expect("render");
        let again = Template::compile(&src).expect("compile again");
        prop_assert_eq!(&first, &common::render(&again, &vars, seed, session, &last).expect("render"));
        prop_assert_eq!(&first, &common::render(&t, &vars, seed, session, &last).expect("render"));
    }

    /// Folding the variables changes neither the bytes nor the generator
    /// draws, so a folded scenario replays the original.
    #[test]
    fn folding_variables_preserves_the_rendering(
        src in template_source(),
        seed in any::<u64>(),
        session in SESSIONS,
    ) {
        let t = Template::compile(&src).expect("compile");
        let vars = common::vars();
        let folded = t.fold_vars(&vars).expect("fold");
        prop_assert!(folded.variables().is_empty());
        prop_assert_eq!(
            common::render(&t, &vars, seed, session, b"ab").expect("render"),
            common::render(&folded, &vars, seed, session, b"ab").expect("render folded")
        );
    }

    /// Regression test of bug #90: ids that differ only in the two lowest
    /// bits used to share one generator stream.
    #[test]
    fn neighboring_session_ids_draw_distinct_streams(seed in any::<u64>(), k in 0u64..(1 << 40)) {
        let t = Template::compile("{{ rand_int(0, 1000000000) }}").expect("compile");
        let vars = common::vars();
        let draws: Vec<Vec<u8>> = (0..4)
            .map(|i| common::render(&t, &vars, seed, 4 * k + i, b"").expect("render"))
            .collect();
        for i in 0..4 {
            for j in i + 1..4 {
                prop_assert_ne!(&draws[i], &draws[j]);
            }
        }
    }

    #[test]
    fn fill_and_rand_int_repeat_for_one_seed_and_session(
        seed in any::<u64>(),
        session in any::<u64>(),
        n in 0usize..5000,
    ) {
        let t = Template::compile(&format!("{{{{ rand_int(0, 1000) }}}}:{{{{ fill({n}) }}}}"))
            .expect("compile");
        let vars = common::vars();
        let first = common::render(&t, &vars, seed, session, b"").expect("render");
        prop_assert_eq!(&first, &common::render(&t, &vars, seed, session, b"").expect("render"));
        let colon = first.iter().position(|b| *b == b':').expect("separator");
        prop_assert_eq!(first.len() - colon - 1, n);
    }
}
