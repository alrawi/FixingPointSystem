// SPDX-License-Identifier: GPL-3.0-only
//! Unit tests of the core functions `int_bytes` and `now` of scenario
//! contract v0.6.

use super::*;
use crate::clock::PinnedClock;
use crate::function::NoFunctions;

/// 2026-11-10T06:15:04Z, the example of the contract, plus 123 ms.
const T0_MS: i64 = 1_794_291_304_123;

fn vars_of(pairs: &[(&str, Value)]) -> Vars {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_string(), v.clone()))
        .collect()
}

/// Renders a compiled template once under `clock` with one fixed generator.
fn render_at(t: &Template, vars: &Vars, clock: &dyn Clock) -> Result<Vec<u8>, TemplateError> {
    let filler = Filler::new(1);
    let overlay = Vars::new();
    let mut rng = Pcg64Mcg::new(7);
    let mut none = NoFunctions;
    let mut ctx = RenderCtx {
        vars,
        overlay: &overlay,
        session_id: 1,
        flow: "f",
        last: &[],
        rng: &mut rng,
        filler: &filler,
        functions: &mut none,
        clock,
    };
    t.render_to_vec(&mut ctx)
}

fn bytes(src: &str, vars: &Vars) -> Result<Vec<u8>, TemplateError> {
    render_at(&Template::compile(src)?, vars, &PinnedClock::new(T0_MS))
}

fn text_at(src: &str, ms: i64) -> Result<String, TemplateError> {
    let t = Template::compile(src)?;
    let out = render_at(&t, &Vars::new(), &PinnedClock::new(ms))?;
    Ok(String::from_utf8(out).unwrap())
}

// ---- int_bytes ----------------------------------------------------------

/// The largest value of an `int` that fits `width` bytes.
fn largest(width: u32) -> i64 {
    if width == 8 {
        i64::MAX
    } else {
        (1_i64 << (8 * width)) - 1
    }
}

#[test]
fn int_bytes_renders_the_contract_examples() {
    assert_eq!(
        bytes("{{ int_bytes(258, 2) }}", &Vars::new()).unwrap(),
        [1, 2]
    );
    assert_eq!(
        bytes("{{ int_bytes(258, 2, 'little') }}", &Vars::new()).unwrap(),
        [2, 1]
    );
    assert_eq!(
        bytes("{{ int_bytes(258, 2, 'big') }}", &Vars::new()).unwrap(),
        [1, 2]
    );
}

#[test]
fn int_bytes_takes_keyword_arguments_in_any_order() {
    let want = [0, 0, 1, 2];
    for src in [
        "{{ int_bytes(value=258, width=4) }}",
        "{{ int_bytes(width=4, value=258, endianness='big') }}",
        "{{ int_bytes(258, width=4) }}",
        "{{ int_bytes(258, 4, endianness='big') }}",
    ] {
        assert_eq!(bytes(src, &Vars::new()).unwrap(), want, "{src}");
    }
    assert_eq!(
        bytes(
            "{{ int_bytes(endianness='little', width=4, value=258) }}",
            &Vars::new()
        )
        .unwrap(),
        [2, 1, 0, 0]
    );
}

#[test]
fn int_bytes_every_width_and_both_byte_orders() {
    for width in 1..=8_u32 {
        // Zero, one, an asymmetric pattern, and the largest value of the width.
        let mut values = vec![0, 1, largest(width)];
        let pattern = 0x0807_0605_0403_0201_i64 & largest(width);
        values.push(pattern);
        for value in values {
            let be = value.to_be_bytes();
            let le = value.to_le_bytes();
            let w = width as usize;
            let want_be = be[8 - w..].to_vec();
            let want_le = le[..w].to_vec();
            let src_be = format!("{{{{ int_bytes({value}, {width}) }}}}");
            let src_le = format!("{{{{ int_bytes({value}, {width}, 'little') }}}}");
            assert_eq!(bytes(&src_be, &Vars::new()).unwrap(), want_be, "{src_be}");
            assert_eq!(bytes(&src_le, &Vars::new()).unwrap(), want_le, "{src_le}");
            assert_eq!(want_be.len(), w);
            let mut rev = want_le.clone();
            rev.reverse();
            assert_eq!(rev, want_be);
        }
    }
}

#[test]
fn int_bytes_width_8_takes_zero_to_i64_max() {
    assert_eq!(
        bytes("{{ int_bytes(9223372036854775807, 8) }}", &Vars::new()).unwrap(),
        [0x7f, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff]
    );
    assert_eq!(
        bytes("{{ int_bytes(0, 8, 'little') }}", &Vars::new()).unwrap(),
        [0; 8]
    );
}

#[test]
fn int_bytes_length_does_not_depend_on_the_value() {
    for width in 1..=8_u32 {
        for value in [0, 1, largest(width)] {
            let src = format!("{{{{ int_bytes({value}, {width}) }}}}");
            assert_eq!(bytes(&src, &Vars::new()).unwrap().len(), width as usize);
        }
    }
}

#[test]
fn int_bytes_value_that_does_not_fit_is_an_error_and_never_wraps() {
    for width in 1..8_u32 {
        let over = largest(width) + 1;
        let vars = vars_of(&[("v", Value::Int(over))]);
        assert_eq!(
            bytes(&format!("{{{{ int_bytes(vars.v, {width}) }}}}"), &vars),
            Err(TemplateError::IntBytesRange {
                value: over,
                width: i64::from(width)
            })
        );
        // The largest value still renders.
        let vars = vars_of(&[("v", Value::Int(largest(width)))]);
        assert!(bytes(&format!("{{{{ int_bytes(vars.v, {width}) }}}}"), &vars).is_ok());
    }
}

#[test]
fn int_bytes_negative_value_is_an_error_at_every_width() {
    for width in 1..=8_i64 {
        for value in [-1, i64::MIN] {
            let vars = vars_of(&[("v", Value::Int(value))]);
            assert_eq!(
                bytes(&format!("{{{{ int_bytes(vars.v, {width}) }}}}"), &vars),
                Err(TemplateError::IntBytesRange { value, width })
            );
        }
    }
}

#[test]
fn int_bytes_width_outside_1_to_8_is_an_error() {
    for width in [0_i64, 9, 16, -1, i64::MAX, i64::MIN] {
        let vars = vars_of(&[("w", Value::Int(width))]);
        assert_eq!(
            bytes("{{ int_bytes(1, vars.w) }}", &vars),
            Err(TemplateError::IntBytesWidth(width))
        );
    }
}

#[test]
fn int_bytes_unknown_endianness_is_an_error() {
    for order in ["", "Big", "LITTLE", "network", "middle"] {
        let vars = vars_of(&[("o", Value::Str(order.into()))]);
        assert_eq!(
            bytes("{{ int_bytes(1, 2, vars.o) }}", &vars),
            Err(TemplateError::IntBytesEndianness(order.into())),
            "{order:?}"
        );
    }
}

#[test]
fn int_bytes_checks_the_order_width_endianness_value() {
    // Several faults at once report the width first, then the endianness.
    let vars = vars_of(&[
        ("v", Value::Int(-1)),
        ("w", Value::Int(0)),
        ("o", Value::Str("x".into())),
    ]);
    assert_eq!(
        bytes("{{ int_bytes(vars.v, vars.w, vars.o) }}", &vars),
        Err(TemplateError::IntBytesWidth(0))
    );
    assert_eq!(
        bytes("{{ int_bytes(vars.v, 2, vars.o) }}", &vars),
        Err(TemplateError::IntBytesEndianness("x".into()))
    );
}

#[test]
fn int_bytes_argument_types_follow_the_parameters() {
    let vars = vars_of(&[
        ("s", Value::Str("12".into())),
        ("bad", Value::Str("twelve".into())),
        ("f", Value::Float(2.0)),
        ("i", Value::Int(1)),
    ]);
    // A text that holds an integer binds an int parameter, as it does for fill().
    assert_eq!(bytes("{{ int_bytes(vars.s, 2) }}", &vars).unwrap(), [0, 12]);
    assert_eq!(bytes("{{ int_bytes(1, vars.f) }}", &vars).unwrap(), [0, 1]);
    assert!(matches!(
        bytes("{{ int_bytes(vars.bad, 2) }}", &vars),
        Err(TemplateError::NotInteger(_))
    ));
    // The endianness is text.
    assert!(matches!(
        bytes("{{ int_bytes(1, 2, vars.i) }}", &vars),
        Err(TemplateError::ArgumentType { param, .. }) if param == "endianness"
    ));
    // Literals of the wrong type fail at compile time.
    for src in [
        "{{ int_bytes('a', 2) }}",
        "{{ int_bytes(1.5, 2) }}",
        "{{ int_bytes(1, 2, 3) }}",
        "{{ int_bytes(hex('01'), 2) }}",
    ] {
        assert!(
            matches!(
                Template::compile(src),
                Err(TemplateError::ArgumentType { .. })
            ),
            "{src}"
        );
    }
}

#[test]
fn int_bytes_call_shape_errors() {
    assert!(matches!(
        Template::compile("{{ int_bytes(1) }}"),
        Err(TemplateError::MissingParameter { param, .. }) if param == "width"
    ));
    assert!(matches!(
        Template::compile("{{ int_bytes() }}"),
        Err(TemplateError::MissingParameter { param, .. }) if param == "value"
    ));
    assert!(matches!(
        Template::compile("{{ int_bytes(1, 2, 'big', 4) }}"),
        Err(TemplateError::Arity { max: 3, got: 4, .. })
    ));
    assert!(matches!(
        Template::compile("{{ int_bytes(1, 2, bits=3) }}"),
        Err(TemplateError::UnknownParameter { .. })
    ));
    assert!(matches!(
        Template::compile("{{ int_bytes(1, value=2) }}"),
        Err(TemplateError::DuplicateParameter { .. })
    ));
}

#[test]
fn int_bytes_literal_arguments_fail_at_compile_time() {
    assert_eq!(
        Template::compile("{{ int_bytes(1, 0) }}"),
        Err(TemplateError::IntBytesWidth(0))
    );
    assert_eq!(
        Template::compile("{{ int_bytes(1, 9) }}"),
        Err(TemplateError::IntBytesWidth(9))
    );
    assert_eq!(
        Template::compile("{{ int_bytes(1, -3) }}"),
        Err(TemplateError::IntBytesWidth(-3))
    );
    assert_eq!(
        Template::compile("{{ int_bytes(1, 2, 'middle') }}"),
        Err(TemplateError::IntBytesEndianness("middle".into()))
    );
    assert_eq!(
        Template::compile("{{ int_bytes(-1, 2) }}"),
        Err(TemplateError::IntBytesRange {
            value: -1,
            width: 2
        })
    );
    assert_eq!(
        Template::compile("{{ int_bytes(65536, 2) }}"),
        Err(TemplateError::IntBytesRange {
            value: 65536,
            width: 2
        })
    );
    assert_eq!(
        Template::compile("{{ int_bytes(256, 1, 'little') }}"),
        Err(TemplateError::IntBytesRange {
            value: 256,
            width: 1
        })
    );
    // A literal value that no width holds, with a width that is not literal.
    assert_eq!(
        Template::compile("{{ int_bytes(-5, vars.w) }}"),
        Err(TemplateError::IntBytesRange {
            value: -5,
            width: 8
        })
    );
    // A literal value that fits some widths waits for the width.
    assert!(Template::compile("{{ int_bytes(65536, vars.w) }}").is_ok());
    // Literals that break no rule compile, at every extreme.
    for src in [
        "{{ int_bytes(0, 1) }}",
        "{{ int_bytes(255, 1) }}",
        "{{ int_bytes(65535, 2, 'little') }}",
        "{{ int_bytes(9223372036854775807, 8) }}",
    ] {
        assert!(Template::compile(src).is_ok(), "{src}");
    }
}

#[test]
fn int_bytes_with_known_arguments_folds_into_the_literal_bytes() {
    let t =
        Template::compile("a{{ int_bytes(258, 2) }}b{{ int_bytes(258, 2, 'little') }}").unwrap();
    assert!(t.is_static());
    assert_eq!(t.as_static(), Some(&b"a\x01\x02b\x02\x01"[..]));

    let t = Template::compile("{{ int_bytes(vars.len, 4) }}").unwrap();
    assert!(!t.is_static());
    let folded = t
        .fold_vars(&vars_of(&[("len", Value::Int(0x0102_0304))]))
        .unwrap();
    assert_eq!(folded.as_static(), Some(&[1, 2, 3, 4][..]));
}

#[test]
fn int_bytes_with_an_invalid_variable_fails_when_rendered_not_when_folded() {
    let t = Template::compile("{{ int_bytes(vars.len, 1) }}").unwrap();
    let constants = vars_of(&[("len", Value::Int(300))]);
    let folded = t.fold_vars(&constants).unwrap();
    assert!(!folded.is_static());
    assert_eq!(
        render_at(&folded, &constants, &PinnedClock::new(0)),
        Err(TemplateError::IntBytesRange {
            value: 300,
            width: 1
        })
    );
}

#[test]
fn int_bytes_composes_with_other_calls() {
    let vars = vars_of(&[("n", Value::Int(5))]);
    let out = bytes("{{ int_bytes(vars.n, 2) }}{{ fill(vars.n) }}", &vars).unwrap();
    assert_eq!(&out[..2], [0, 5]);
    assert_eq!(out.len(), 7);
    assert_eq!(
        bytes("{{ int_bytes(rand_int(3, 3), 2) }}", &Vars::new()).unwrap(),
        [0, 3]
    );
}

#[test]
fn int_bytes_and_now_draw_nothing_from_the_generator() {
    let vars = Vars::new();
    let plain = bytes("{{ rand_int(1, 1000000) }}|{{ fill(8) }}", &vars).unwrap();
    for lead in [
        "{{ int_bytes(7, 4) }}",
        "{{ int_bytes(vars.k, 4) }}",
        "{{ now('http_date') }}",
        "{{ now('epoch_ms') }}",
        "{{ int_bytes(7, 4) }}{{ now('epoch_s') }}",
    ] {
        let vars = vars_of(&[("k", Value::Int(7))]);
        let src = format!("{lead}{{{{ rand_int(1, 1000000) }}}}|{{{{ fill(8) }}}}");
        let out = bytes(&src, &vars).unwrap();
        assert!(out.ends_with(&plain), "{lead}: {out:?} vs {plain:?}");
    }
}

#[test]
fn int_bytes_dump_shows_the_bound_default() {
    let t = Template::compile("{{ int_bytes(vars.n, 2) }}").unwrap();
    assert_eq!(
        t.dump(),
        "dynamic, 1 segment\n\
         [0] int_bytes\n\
         \x20   value = vars.n\n\
         \x20   width = int 2\n\
         \x20   endianness = text \"big\"\n"
    );
}

// ---- now ------------------------------------------------------------------

#[test]
fn now_renders_the_three_formats_under_a_pinned_clock() {
    assert_eq!(
        text_at("{{ now('http_date') }}", T0_MS).unwrap(),
        "Tue, 10 Nov 2026 06:15:04 GMT"
    );
    assert_eq!(
        text_at("{{ now('epoch_s') }}", T0_MS).unwrap(),
        "1794291304"
    );
    assert_eq!(
        text_at("{{ now('epoch_ms') }}", T0_MS).unwrap(),
        "1794291304123"
    );
    // The keyword form.
    assert_eq!(
        text_at("{{ now(format='epoch_s') }}", T0_MS).unwrap(),
        "1794291304"
    );
}

#[test]
fn now_truncates_toward_zero() {
    assert_eq!(text_at("{{ now('epoch_s') }}", 999).unwrap(), "0");
    assert_eq!(text_at("{{ now('epoch_s') }}", 1000).unwrap(), "1");
    assert_eq!(text_at("{{ now('epoch_s') }}", -1).unwrap(), "0");
    assert_eq!(text_at("{{ now('epoch_s') }}", -1001).unwrap(), "-1");
    assert_eq!(text_at("{{ now('epoch_ms') }}", -1).unwrap(), "-1");
    // The date of an instant before the epoch counts whole seconds downward.
    assert_eq!(
        text_at("{{ now('http_date') }}", -1).unwrap(),
        "Wed, 31 Dec 1969 23:59:59 GMT"
    );
}

#[test]
fn now_output_length_holds_from_2001_to_2286() {
    // 10^9 s is 2001-09-09T01:46:40Z, and 10^10 - 1 s is 2286-11-20T17:46:39Z.
    for ms in [1_000_000_000_000_i64, T0_MS, 9_999_999_999_999] {
        assert_eq!(
            text_at("{{ now('http_date') }}", ms).unwrap().len(),
            29,
            "{ms}"
        );
        assert_eq!(
            text_at("{{ now('epoch_s') }}", ms).unwrap().len(),
            10,
            "{ms}"
        );
        assert_eq!(
            text_at("{{ now('epoch_ms') }}", ms).unwrap().len(),
            13,
            "{ms}"
        );
    }
    assert_eq!(
        text_at("{{ now('http_date') }}", 9_999_999_999_000).unwrap(),
        "Sat, 20 Nov 2286 17:46:39 GMT"
    );
    assert_eq!(
        text_at("{{ now('http_date') }}", 1_000_000_000_000).unwrap(),
        "Sun, 09 Sep 2001 01:46:40 GMT"
    );
}

#[test]
fn now_http_date_outside_four_digit_years_is_an_error() {
    assert_eq!(
        text_at("{{ now('http_date') }}", 253_402_300_800_000),
        Err(TemplateError::NowRange(253_402_300_800_000))
    );
    // The epoch formats still render there.
    assert_eq!(
        text_at("{{ now('epoch_s') }}", 253_402_300_800_000).unwrap(),
        "253402300800"
    );
}

#[test]
fn now_each_evaluation_reads_the_clock() {
    let t = Template::compile("{{ now('epoch_ms') }},{{ now('epoch_ms') }},{{ now('epoch_ms') }}")
        .unwrap();
    let clock = PinnedClock::new(1_000_000_000_000).with_step_ms(10);
    let out = render_at(&t, &Vars::new(), &clock).unwrap();
    assert_eq!(out, b"1000000000000,1000000000010,1000000000020");
    // A pinned clock without a step returns the same instant to every call.
    let fixed = PinnedClock::new(1_000_000_000_000);
    assert_eq!(
        render_at(&t, &Vars::new(), &fixed).unwrap(),
        b"1000000000000,1000000000000,1000000000000"
    );
}

#[test]
fn now_reads_the_clock_of_the_context_and_the_system_clock_by_default() {
    let t = Template::compile("{{ now('epoch_s') }}").unwrap();
    let out = render_at(&t, &Vars::new(), &crate::SystemClock).unwrap();
    let secs: i64 = String::from_utf8(out).unwrap().parse().unwrap();
    assert!(secs > 1_735_689_600, "{secs}");
}

#[test]
fn now_unknown_format_is_an_error() {
    assert_eq!(
        Template::compile("{{ now('rfc3339') }}"),
        Err(TemplateError::NowFormat("rfc3339".into()))
    );
    assert_eq!(
        Template::compile("{{ now('') }}"),
        Err(TemplateError::NowFormat(String::new()))
    );
    // The names are case sensitive, as every contract name is.
    assert_eq!(
        Template::compile("{{ now('EPOCH_S') }}"),
        Err(TemplateError::NowFormat("EPOCH_S".into()))
    );
    // A format from a variable fails when rendered.
    let vars = vars_of(&[("f", Value::Str("rfc3339".into())), ("i", Value::Int(1))]);
    assert_eq!(
        bytes("{{ now(vars.f) }}", &vars),
        Err(TemplateError::NowFormat("rfc3339".into()))
    );
    assert!(matches!(
        bytes("{{ now(vars.i) }}", &vars),
        Err(TemplateError::ArgumentType { function, .. }) if function == "now"
    ));
    // The format is required and a text.
    assert!(matches!(
        Template::compile("{{ now() }}"),
        Err(TemplateError::MissingParameter { .. })
    ));
    assert!(matches!(
        Template::compile("{{ now(1) }}"),
        Err(TemplateError::ArgumentType { .. })
    ));
    assert!(matches!(
        Template::compile("{{ now('epoch_s', 'epoch_ms') }}"),
        Err(TemplateError::Arity { .. })
    ));
}

#[test]
fn now_result_type_follows_a_literal_format() {
    // epoch_s and epoch_ms are int, so fill() takes them.
    assert!(Template::compile("{{ fill(now('epoch_s')) }}").is_ok());
    assert!(Template::compile("{{ rand_int(1, now('epoch_ms')) }}").is_ok());
    // http_date is text, which fill() does not take.
    assert!(matches!(
        Template::compile("{{ fill(now('http_date')) }}"),
        Err(TemplateError::ArgumentType { param, .. }) if param == "n"
    ));
    // The text of http_date binds a text parameter, and int_bytes takes an int.
    assert!(Template::compile("{{ hex(now('http_date')) }}").is_ok());
    assert!(matches!(
        Template::compile("{{ int_bytes(now('http_date'), 8) }}"),
        Err(TemplateError::ArgumentType { .. })
    ));
    assert!(Template::compile("{{ int_bytes(now('epoch_s'), 8) }}").is_ok());
    // A format from a variable gives any, which every parameter accepts.
    assert!(Template::compile("{{ fill(now(vars.f)) }}").is_ok());
}

#[test]
fn now_epoch_feeds_int_bytes() {
    // A four byte timestamp, as a binary protocol writes it.
    let t = Template::compile("{{ int_bytes(now('epoch_s'), 4) }}").unwrap();
    let out = render_at(&t, &Vars::new(), &PinnedClock::new(T0_MS)).unwrap();
    assert_eq!(out, 1_794_291_304_u32.to_be_bytes());
    // A value that no longer fits the width is an error, not a wrapped value.
    let t = Template::compile("{{ int_bytes(now('epoch_ms'), 4) }}").unwrap();
    assert!(matches!(
        render_at(&t, &Vars::new(), &PinnedClock::new(T0_MS)),
        Err(TemplateError::IntBytesRange { width: 4, .. })
    ));
}

#[test]
fn now_never_folds() {
    for src in [
        "{{ now('http_date') }}",
        "{{ now('epoch_s') }}",
        "{{ now('epoch_ms') }}",
        "x{{ now(vars.f) }}",
    ] {
        let t = Template::compile(src).unwrap();
        assert!(!t.is_static(), "{src}");
        let folded = t
            .fold_vars(&vars_of(&[("f", Value::Str("epoch_s".into()))]))
            .unwrap();
        assert!(!folded.is_static(), "{src} after fold_vars");
        let folded = t.fold_profiles(&Profiles::default()).unwrap();
        assert!(!folded.is_static(), "{src} after fold_profiles");
    }
    // A folded variable gives the same render as the unfolded template.
    let t = Template::compile("{{ now(vars.f) }}").unwrap();
    let constants = vars_of(&[("f", Value::Str("epoch_s".into()))]);
    let folded = t.fold_vars(&constants).unwrap();
    assert_eq!(
        render_at(&folded, &Vars::new(), &PinnedClock::new(T0_MS)).unwrap(),
        b"1794291304"
    );
}

#[test]
fn now_dump_names_the_format() {
    let t = Template::compile("{{ now('epoch_s') }}").unwrap();
    assert_eq!(
        t.dump(),
        "dynamic, 1 segment\n[0] now\n\x20   format = text \"epoch_s\"\n"
    );
}

mod properties {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        /// Every value that fits renders `width` bytes that read back as the
        /// value in the byte order, and every other value is an error.
        #[test]
        fn int_bytes_round_trips_or_refuses(value in any::<i64>(), width in 1_i64..=8) {
            let big = int_bytes(value, width, "big");
            let little = int_bytes(value, width, "little");
            let fits = value >= 0 && (width == 8 || (value as u64) >> (8 * width) == 0);
            if fits {
                let (big, little) = (big.unwrap(), little.unwrap());
                prop_assert_eq!(big.len() as i64, width);
                let read = big.iter().fold(0_u64, |acc, b| (acc << 8) | u64::from(*b));
                prop_assert_eq!(read, value as u64);
                let mut rev = little;
                rev.reverse();
                prop_assert_eq!(rev, big);
            } else {
                prop_assert_eq!(big, Err(TemplateError::IntBytesRange { value, width }));
                prop_assert_eq!(little, Err(TemplateError::IntBytesRange { value, width }));
            }
        }

        /// Every instant between the years 2001 and 2286 renders the same
        /// lengths in all three formats.
        #[test]
        fn now_lengths_do_not_depend_on_the_instant(ms in 1_000_000_000_000_i64..10_000_000_000_000) {
            prop_assert_eq!(text_at("{{ now('http_date') }}", ms).unwrap().len(), 29);
            prop_assert_eq!(text_at("{{ now('epoch_s') }}", ms).unwrap().len(), 10);
            prop_assert_eq!(text_at("{{ now('epoch_ms') }}", ms).unwrap().len(), 13);
        }
    }
}
