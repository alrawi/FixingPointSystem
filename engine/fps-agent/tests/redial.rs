// SPDX-License-Identifier: GPL-3.0-only
//! Component tests of the redial policy of decision D12 item 11: the backoff
//! restarts at 200 ms with jitter after an abort, and failed attempts end the
//! agent after a limit.
//!
//! The policy takes the time as an argument and draws its jitter from a
//! seeded generator, so these tests run without a clock and without a
//! controller. The last test runs [`serve`] against a closed port.

use std::net::TcpListener;
use std::time::{Duration, Instant};

use fps_agent::session::{serve, Backoff, Config, Redial, RedialPolicy, SessionError};

const INITIAL: Duration = Duration::from_millis(200);
const SEED: u64 = 7;

fn policy(limit: Option<Duration>) -> RedialPolicy {
    RedialPolicy::new(Backoff::default(), limit, SEED)
}

/// Asserts that `redial` waits for a jittered `base`, which lies in
/// `[base / 2, base]`, and returns the wait.
fn wait_near(redial: Redial, base: Duration) -> Duration {
    match redial {
        Redial::After(wait) => {
            assert!(
                wait >= base / 2 && wait <= base,
                "wait {wait:?} is outside [{:?}, {base:?}]",
                base / 2
            );
            wait
        }
        Redial::GiveUp { failing_for } => panic!("gave up after {failing_for:?}"),
    }
}

#[test]
fn failed_dials_back_off_exponentially_up_to_the_maximum() {
    let mut p = policy(None);
    let t0 = Instant::now();
    let bases = [200, 400, 800, 1600, 3200, 6400, 10_000, 10_000];
    for (i, base) in bases.into_iter().enumerate() {
        let now = t0 + Duration::from_secs(i as u64);
        wait_near(
            p.session_ended(now, false, false),
            Duration::from_millis(base),
        );
    }
}

#[test]
fn an_abort_restarts_the_backoff_at_200_ms_with_jitter() {
    let mut p = policy(None);
    let t0 = Instant::now();
    // Three failed dials push the next wait to 1.6 s.
    for i in 0..3 {
        wait_near(
            p.session_ended(t0 + Duration::from_secs(i), false, false),
            INITIAL * 2u32.pow(i as u32),
        );
    }
    // The controller aborts the session after a dial that opened. The
    // backoff goes back to its start, and the jitter keeps it from being
    // exactly 200 ms every time.
    let mut waits = Vec::new();
    for i in 0..6 {
        let now = t0 + Duration::from_secs(10 + i);
        waits.push(wait_near(p.session_ended(now, true, true), INITIAL));
    }
    waits.sort();
    waits.dedup();
    assert!(waits.len() > 1, "no jitter: {waits:?}");
    // The redial that follows the last abort fails. Its wait is the second
    // step of the restarted backoff, 400 ms, and not the 3.2 s that the
    // earlier failures had reached.
    let after = p.session_ended(t0 + Duration::from_secs(20), false, false);
    wait_near(after, INITIAL * 2);
}

#[test]
fn an_abort_that_ended_a_dial_before_it_opened_also_restarts_the_backoff() {
    let mut p = policy(None);
    let t0 = Instant::now();
    for i in 0..4 {
        p.session_ended(t0 + Duration::from_secs(i), false, false);
    }
    wait_near(
        p.session_ended(t0 + Duration::from_secs(5), false, true),
        INITIAL,
    );
}

#[test]
fn a_session_that_opened_resets_the_backoff_and_the_failure_window() {
    let mut p = policy(Some(Duration::from_secs(10)));
    let t0 = Instant::now();
    for i in 0..4 {
        p.session_ended(t0 + Duration::from_secs(i), false, false);
    }
    // A session that ran and then dropped. The window clears, so a failure
    // 9 s later does not reach the limit, and the backoff went back to
    // 200 ms for the redial before it, so this failure waits 400 ms.
    wait_near(
        p.session_ended(t0 + Duration::from_secs(5), true, false),
        INITIAL,
    );
    wait_near(
        p.session_ended(t0 + Duration::from_secs(14), false, false),
        INITIAL * 2,
    );
}

#[test]
fn failed_attempts_give_up_after_the_limit() {
    let limit = Duration::from_secs(300);
    let mut p = policy(Some(limit));
    let t0 = Instant::now();
    // The first failure starts the window at t0.
    assert!(matches!(
        p.session_ended(t0, false, false),
        Redial::After(_)
    ));
    let almost = t0 + limit - Duration::from_millis(1);
    assert!(matches!(
        p.session_ended(almost, false, false),
        Redial::After(_)
    ));
    assert_eq!(
        p.session_ended(t0 + limit, false, false),
        Redial::GiveUp { failing_for: limit }
    );
}

#[test]
fn aborts_alone_cannot_keep_the_agent_redialing_forever() {
    let limit = Duration::from_secs(60);
    let mut p = policy(Some(limit));
    let t0 = Instant::now();
    // Two agents of one identity replace each other every 5 s. Each session
    // opens and ends with ABORTED, and the window keeps running.
    let mut gave_up_at = None;
    for i in 0..30 {
        let now = t0 + Duration::from_secs(5 * i);
        if let Redial::GiveUp { failing_for } = p.session_ended(now, true, true) {
            assert!(failing_for >= limit);
            gave_up_at = Some(5 * i);
            break;
        }
    }
    assert_eq!(gave_up_at, Some(60));
}

#[test]
fn no_limit_redials_forever() {
    let mut p = policy(None);
    let t0 = Instant::now();
    for i in 0..100 {
        let now = t0 + Duration::from_secs(3600 * i);
        assert!(matches!(
            p.session_ended(now, false, false),
            Redial::After(_)
        ));
    }
}

#[test]
fn the_same_seed_gives_the_same_waits_and_another_seed_gives_others() {
    let run = |seed: u64| {
        let mut p = RedialPolicy::new(Backoff::default(), None, seed);
        let t0 = Instant::now();
        (0..5)
            .map(|i| p.session_ended(t0 + Duration::from_secs(i), false, false))
            .collect::<Vec<_>>()
    };
    assert_eq!(run(1), run(1));
    assert_ne!(run(1), run(2));
    // A zero seed still jitters.
    assert_ne!(run(0)[0], Redial::After(INITIAL));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn serve_exits_with_a_redial_limit_error_when_the_controller_stays_unreachable() {
    // Bind and drop a listener to get a port that refuses connections.
    let port = match TcpListener::bind("127.0.0.1:0").and_then(|l| l.local_addr()) {
        Ok(addr) => addr.port(),
        Err(e) => panic!("reserve a port: {e}"),
    };
    let mut cfg = Config::new(
        format!("http://127.0.0.1:{port}"),
        "redial-limit".into(),
        std::env::temp_dir(),
    );
    cfg.backoff = Backoff {
        initial: Duration::from_millis(20),
        max: Duration::from_millis(80),
    };
    cfg.redial_limit = Some(Duration::from_millis(500));
    cfg.jitter_seed = SEED;

    let started = Instant::now();
    let result = tokio::time::timeout(
        Duration::from_secs(30),
        serve(cfg, std::future::pending::<()>()),
    )
    .await;
    let elapsed = started.elapsed();
    match result {
        Ok(Err(SessionError::RedialLimit { failing_for, last })) => {
            assert!(failing_for >= Duration::from_millis(500), "{failing_for:?}");
            assert!(!last.is_empty());
        }
        other => panic!("expected the redial limit error, got {other:?}"),
    }
    assert!(elapsed >= Duration::from_millis(500), "{elapsed:?}");
}
