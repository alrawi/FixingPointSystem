// SPDX-License-Identifier: GPL-3.0-only
//! Property tests of the template compiler and renderer.

use fps_flow::template::{decode_hex, Filler, RenderCtx};
use fps_flow::{Template, Vars};
use std::sync::OnceLock;

use proptest::prelude::*;
use rand_pcg::Pcg64Mcg;

/// One filler for every case, because building it costs a random draw per byte.
fn filler() -> &'static Filler {
    static FILLER: OnceLock<Filler> = OnceLock::new();
    FILLER.get_or_init(|| Filler::new(7))
}

fn render(t: &Template, seed: u64, session_id: u64, filler: &Filler) -> Vec<u8> {
    let vars = Vars::new();
    let overlay = Vars::new();
    let mut rng = Pcg64Mcg::new(u128::from(seed) << 64 | u128::from(session_id));
    let mut ctx = RenderCtx {
        vars: &vars,
        overlay: &overlay,
        session_id,
        flow: "f",
        rng: &mut rng,
        filler,
    };
    t.render_to_vec(&mut ctx).expect("render")
}

proptest! {
    #[test]
    fn text_without_braces_renders_unchanged(text in "[^{}]{0,200}") {
        let t = Template::compile(&text).expect("compile");
        prop_assert!(t.is_static());
        prop_assert_eq!(render(&t, 0, 0, filler()), text.into_bytes());
    }

    #[test]
    fn hex_round_trips(bytes in prop::collection::vec(any::<u8>(), 0..128)) {
        let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
        prop_assert_eq!(decode_hex(&hex).expect("decode"), bytes);
    }

    #[test]
    fn fill_and_rand_int_repeat_for_one_seed_and_session(
        seed in any::<u64>(),
        session in any::<u64>(),
        n in 0usize..5000,
    ) {
        let t = Template::compile(&format!("{{{{ rand_int(0, 1000) }}}}:{{{{ fill({n}) }}}}"))
            .expect("compile");
        let first = render(&t, seed, session, filler());
        prop_assert_eq!(&first, &render(&t, seed, session, filler()));
        let colon = first.iter().position(|b| *b == b':').expect("separator");
        prop_assert_eq!(first.len() - colon - 1, n);
    }
}
