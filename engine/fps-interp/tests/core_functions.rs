// SPDX-License-Identifier: GPL-3.0-only
//! The core functions `int_bytes` and `now` of scenario contract v0.6 in a
//! running session, with the clock pinned through `Session::set_clock`.

mod common;

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use common::{run_pair, run_recorded, Opts};
use fps_flow::PinnedClock;
use fps_interp::Reason;

/// 2026-11-10T06:15:04Z plus 123 ms.
const T0_MS: i64 = 1_794_291_304_123;

fn pinned(start_ms: i64, step_ms: i64) -> Option<Arc<dyn fps_flow::Clock>> {
    Some(Arc::new(PinnedClock::new(start_ms).with_step_ms(step_ms)))
}

/// A scenario whose client runs `client` and whose server reads `n` bytes.
fn scenario(client: &str, n: usize) -> String {
    format!(
        r#"
scenario: core
vars: {{ size: 258 }}
flows:
  - id: c
    module: m
    client:
      - connect: {{ port: 1 }}
{client}
      - close: fin
    server:
      - accept: {{ port: 1 }}
      - expect: {{ bytes: {n}, timeout_ms: 1000 }}
      - close: fin
"#
    )
}

#[tokio::test]
async fn a_pinned_clock_fixes_every_byte_of_now() {
    let text = scenario(
        r#"      - send: "{{ now('http_date') }}|{{ now('epoch_s') }}|{{ now('epoch_ms') }}""#,
        29 + 1 + 10 + 1 + 13,
    );
    let opts = Opts {
        clock: pinned(T0_MS, 0),
        ..Opts::default()
    };
    let (pair, sent, _) = run_recorded(&text, opts).await;
    pair.client.unwrap();
    pair.server.unwrap();
    assert_eq!(
        String::from_utf8(sent).unwrap(),
        "Tue, 10 Nov 2026 06:15:04 GMT|1794291304|1794291304123"
    );
}

#[tokio::test]
async fn int_bytes_writes_exactly_width_bytes_from_variables() {
    // `size` folds into the program, and `len` is bound at run time.
    let text = scenario(
        r#"      - set: { len: "258" }
      - send: "{{ int_bytes(vars.size, 4) }}{{ int_bytes(vars.len, 2, 'little') }}{{ int_bytes(vars.len, 8) }}""#,
        4 + 2 + 8,
    );
    let (pair, sent, _) = run_recorded(&text, Opts::default()).await;
    pair.client.unwrap();
    pair.server.unwrap();
    assert_eq!(sent, [0, 0, 1, 2, 2, 1, 0, 0, 0, 0, 0, 0, 1, 2]);
    assert_eq!(pair.client_stats.bytes_tx, 14);
}

#[tokio::test]
async fn each_evaluation_reads_the_clock_and_set_binds_one_instant() {
    let text = scenario(
        r#"      - set: { t: "{{ now('epoch_ms') }}" }
      - send: "{{ vars.t }},{{ vars.t }},{{ now('epoch_ms') }},{{ now('epoch_ms') }}""#,
        13 * 4 + 3,
    );
    let opts = Opts {
        clock: pinned(1_000_000_000_000, 10),
        ..Opts::default()
    };
    let (pair, sent, _) = run_recorded(&text, opts).await;
    pair.client.unwrap();
    pair.server.unwrap();
    // The `set` read the first instant, and the two later calls the next two.
    assert_eq!(
        String::from_utf8(sent).unwrap(),
        "1000000000000,1000000000000,1000000000010,1000000000020"
    );
}

#[tokio::test]
async fn without_a_pinned_clock_now_reads_the_wall_clock() {
    let text = scenario(r#"      - send: "{{ now('epoch_s') }}""#, 10);
    let before = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let (pair, sent, _) = run_recorded(&text, Opts::default()).await;
    let after = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    pair.client.unwrap();
    pair.server.unwrap();
    let got: u64 = String::from_utf8(sent).unwrap().parse().unwrap();
    assert!(
        (before..=after).contains(&got),
        "{got} not in {before}..={after}"
    );
}

#[tokio::test(start_paused = true)]
async fn a_computed_argument_that_breaks_a_rule_fails_the_session_as_template_error() {
    for (bind, call) in [
        // A value that does not fit the width is never truncated.
        ("300", "int_bytes(vars.v, 1)"),
        ("256", "int_bytes(vars.v, 1, 'little')"),
        ("4294967296", "int_bytes(vars.v, 4)"),
        // A negative value is never encoded.
        ("-1", "int_bytes(vars.v, 8)"),
        // A width outside 1 to 8.
        ("0", "int_bytes(1, vars.v)"),
        ("9", "int_bytes(1, vars.v)"),
        // An endianness other than big or little.
        ("middle", "int_bytes(1, 2, vars.v)"),
        // An unknown format.
        ("rfc3339", "now(vars.v)"),
    ] {
        let text = scenario(
            &format!("      - set: {{ v: \"{bind}\" }}\n      - send: \"{{{{ {call} }}}}\""),
            1,
        );
        let opts = Opts {
            clock: pinned(T0_MS, 0),
            ..Opts::default()
        };
        let pair = run_pair(&text, opts).await;
        let f = pair.client.expect_err(call);
        assert_eq!(f.reason, Reason::TemplateError, "{bind} {call}: {f}");
        assert_eq!(f.action_type, "send", "{bind} {call}");
        assert_eq!(f.action_index, 2, "{bind} {call}");
        assert_eq!(
            pair.client_stats.bytes_tx, 0,
            "nothing was sent: {bind} {call}"
        );
    }
}

#[tokio::test(start_paused = true)]
async fn http_date_beyond_year_9999_fails_as_template_error() {
    let text = scenario(r#"      - send: "{{ now('http_date') }}""#, 29);
    let opts = Opts {
        clock: pinned(253_402_300_800_000, 0),
        ..Opts::default()
    };
    let pair = run_pair(&text, opts).await;
    let f = pair.client.expect_err("year 10000");
    assert_eq!(f.reason, Reason::TemplateError, "{f}");
}
