// SPDX-License-Identifier: GPL-3.0-only
//! Component tests of the scheduler through its public interface.

use std::time::Duration;

use fps_sched::{
    stream_id, Arrival, CapPolicy, LoadProfile, ManualClock, Objective, Phase, Schedule,
    ScheduleConfig, SessionStart, Shape,
};

fn secs(s: u64) -> Duration {
    Duration::from_secs(s)
}

/// Ramp up for 10 s, hold for 10 s, and ramp down for 10 s.
fn ramp_steady_down() -> LoadProfile {
    LoadProfile::new(vec![
        Phase::new("ramp_up", secs(10), Shape::Linear, 0.0, 1.0),
        Phase::new("steady", secs(10), Shape::Hold, 1.0, 1.0),
        Phase::new("ramp_down", secs(10), Shape::Linear, 1.0, 0.0),
    ])
    .unwrap()
}

fn rate_schedule(rate: f64, arrival: Arrival, profile: LoadProfile, seed: u64) -> Schedule {
    let mut cfg = ScheduleConfig::new(Objective::SessionsPerSecond(rate), profile, seed);
    cfg.arrival = arrival;
    Schedule::new(cfg).unwrap()
}

/// Drains every start at once with the clock at the end and no open session.
fn drain_all(sched: &mut Schedule) -> Vec<SessionStart> {
    let clock = ManualClock::new(Duration::MAX);
    sched.drain(&clock, 0).collect()
}

fn count_in(starts: &[SessionStart], from: f64, to: f64) -> usize {
    starts
        .iter()
        .filter(|s| {
            let t = s.scheduled_at.as_secs_f64();
            t >= from && t < to
        })
        .count()
}

/// The two sided Kolmogorov Smirnov statistic of `samples` against `cdf`.
fn ks_statistic(mut samples: Vec<f64>, cdf: impl Fn(f64) -> f64) -> f64 {
    samples.sort_by(f64::total_cmp);
    let n = samples.len() as f64;
    samples
        .iter()
        .enumerate()
        .map(|(i, &x)| {
            let f = cdf(x);
            (f - i as f64 / n).abs().max(((i + 1) as f64 / n - f).abs())
        })
        .fold(0.0, f64::max)
}

/// The asymptotic critical value of the KS statistic at alpha 0.05.
fn ks_critical(n: usize) -> f64 {
    1.358 / (n as f64).sqrt()
}

fn exp_cdf(rate: f64) -> impl Fn(f64) -> f64 {
    move |x| 1.0 - (-rate * x).exp()
}

fn gaps(starts: &[SessionStart]) -> Vec<f64> {
    starts
        .windows(2)
        .map(|w| (w[1].scheduled_at - w[0].scheduled_at).as_secs_f64())
        .collect()
}

#[test]
fn same_seed_gives_identical_schedules() {
    for arrival in [Arrival::Constant, Arrival::Poisson] {
        let a = drain_all(&mut rate_schedule(500.0, arrival, ramp_steady_down(), 42));
        let b = drain_all(&mut rate_schedule(500.0, arrival, ramp_steady_down(), 42));
        assert!(!a.is_empty());
        assert_eq!(a, b, "{arrival:?}");
    }
    let a = drain_all(&mut rate_schedule(
        500.0,
        Arrival::Poisson,
        ramp_steady_down(),
        42,
    ));
    let c = drain_all(&mut rate_schedule(
        500.0,
        Arrival::Poisson,
        ramp_steady_down(),
        43,
    ));
    assert_ne!(a, c, "another seed gives another schedule");
}

#[test]
fn streams_of_one_seed_differ() {
    let mut cfg = ScheduleConfig::new(Objective::SessionsPerSecond(100.0), ramp_steady_down(), 7);
    cfg.stream = stream_id(0, 0);
    let a = drain_all(&mut Schedule::new(cfg.clone()).unwrap());
    cfg.stream = stream_id(1, 0);
    let b = drain_all(&mut Schedule::new(cfg.clone()).unwrap());
    cfg.stream = stream_id(0, 1);
    let c = drain_all(&mut Schedule::new(cfg).unwrap());
    assert_ne!(a, b);
    assert_ne!(a, c);
    assert_ne!(b, c);
}

#[test]
fn draining_in_ticks_matches_draining_at_once() {
    let at_once = drain_all(&mut rate_schedule(
        300.0,
        Arrival::Poisson,
        ramp_steady_down(),
        9,
    ));
    let mut sched = rate_schedule(300.0, Arrival::Poisson, ramp_steady_down(), 9);
    let clock = ManualClock::default();
    let mut ticked = Vec::new();
    while !sched.is_finished(&clock) {
        clock.advance(Duration::from_millis(1));
        ticked.extend(sched.drain(&clock, 0));
    }
    assert_eq!(at_once, ticked);
    for s in &ticked {
        assert!(s.scheduled_at <= secs(30));
    }
}

#[test]
fn a_worker_loop_driven_by_next_due_sees_every_start_on_time() {
    let want = drain_all(&mut rate_schedule(
        200.0,
        Arrival::Poisson,
        ramp_steady_down(),
        3,
    ));
    let mut sched = rate_schedule(200.0, Arrival::Poisson, ramp_steady_down(), 3);
    let clock = ManualClock::default();
    let mut got = Vec::new();
    while let Some(due) = sched.next_due(&clock, 0) {
        clock.set(due);
        let batch: Vec<_> = sched.drain(&clock, 0).collect();
        assert!(!batch.is_empty(), "a due time releases a start");
        for s in &batch {
            assert_eq!(s.scheduled_at, due);
        }
        got.extend(batch);
    }
    assert!(sched.is_finished(&clock));
    assert_eq!(want, got);
}

#[test]
fn ks_test_accepts_poisson_inter_arrival_times() {
    let profile =
        || LoadProfile::new(vec![Phase::new("steady", secs(100), Shape::Hold, 1.0, 1.0)]).unwrap();
    let rate = 200.0;
    let starts = drain_all(&mut rate_schedule(rate, Arrival::Poisson, profile(), 42));
    let g = gaps(&starts);
    let n = g.len();
    assert!(n > 19_000, "{n} arrivals");
    let d = ks_statistic(g, exp_cdf(rate));
    assert!(d < ks_critical(n), "D {d} critical {}", ks_critical(n));

    // Across 40 seeds a level 0.05 test rejects about 2 times. More than 8
    // rejections would mean that the gaps are not exponential.
    let rejections = (0..40)
        .filter(|&seed| {
            let starts = drain_all(&mut rate_schedule(rate, Arrival::Poisson, profile(), seed));
            let g = gaps(&starts);
            let n = g.len();
            ks_statistic(g, exp_cdf(rate)) >= ks_critical(n)
        })
        .count();
    assert!(rejections <= 8, "{rejections} of 40 seeds rejected");

    // The same test rejects constant arrivals, so it has power.
    let constant = drain_all(&mut rate_schedule(rate, Arrival::Constant, profile(), 42));
    let g = gaps(&constant);
    let n = g.len();
    assert!(ks_statistic(g, exp_cdf(rate)) > ks_critical(n));
}

#[test]
fn ks_test_accepts_rescaled_gaps_during_a_ramp() {
    // By the time rescaling theorem the gaps of the integrated rate between
    // consecutive starts are exponential with mean 1, also while the rate ramps.
    let profile = ramp_steady_down();
    let rate = 400.0;
    let starts = drain_all(&mut rate_schedule(
        rate,
        Arrival::Poisson,
        profile.clone(),
        11,
    ));
    let ramp: Vec<f64> = starts
        .iter()
        .map(|s| s.scheduled_at.as_secs_f64())
        .filter(|&t| t < 10.0)
        .map(|t| rate * profile.mass_at(t))
        .collect();
    let g: Vec<f64> = ramp.windows(2).map(|w| w[1] - w[0]).collect();
    let n = g.len();
    assert!(n > 1_800, "{n} arrivals in the ramp");
    let d = ks_statistic(g, exp_cdf(1.0));
    assert!(d < ks_critical(n), "D {d} critical {}", ks_critical(n));
}

#[test]
fn constant_arrivals_hold_the_expected_count_at_every_phase_boundary() {
    let starts = drain_all(&mut rate_schedule(
        100.0,
        Arrival::Constant,
        ramp_steady_down(),
        1,
    ));
    // Ramp up from 0 to 100 per second over 10 s holds 500 starts, a quarter of
    // them in its first half.
    assert_eq!(count_in(&starts, 0.0, 10.0), 500);
    assert_eq!(count_in(&starts, 0.0, 5.0), 125);
    assert_eq!(count_in(&starts, 5.0, 10.0), 375);
    // Steady holds 100 per second.
    assert_eq!(count_in(&starts, 10.0, 20.0), 1000);
    for second in 10..20 {
        let s = f64::from(second);
        assert_eq!(count_in(&starts, s, s + 1.0), 100, "second {second}");
    }
    // Ramp down mirrors ramp up.
    assert_eq!(count_in(&starts, 20.0, 30.0), 500);
    assert_eq!(count_in(&starts, 20.0, 25.0), 375);
    assert_eq!(count_in(&starts, 25.0, 30.0), 125);
    assert_eq!(starts.len(), 2000);
    // Each start names the phase that holds it, and phases appear in order.
    for s in &starts {
        let t = s.scheduled_at.as_secs_f64();
        assert_eq!(s.phase, (t / 10.0) as usize, "start at {t}");
    }
    assert!(starts
        .windows(2)
        .all(|w| w[0].scheduled_at < w[1].scheduled_at));
    // Starts are densest in steady and spread out toward the ramp ends.
    let first = starts[0].scheduled_at.as_secs_f64();
    assert!(first > 0.2 && first < 0.4, "first start at {first}");
}

#[test]
fn poisson_arrivals_hold_the_expected_count_per_phase() {
    let starts = drain_all(&mut rate_schedule(
        1000.0,
        Arrival::Poisson,
        ramp_steady_down(),
        5,
    ));
    // Each count is Poisson, so it stays within five standard deviations.
    for (from, to, want) in [
        (0.0, 5.0, 1250.0),
        (5.0, 10.0, 3750.0),
        (10.0, 20.0, 10_000.0),
        (20.0, 25.0, 3750.0),
        (25.0, 30.0, 1250.0),
    ] {
        let got = count_in(&starts, from, to) as f64;
        let f64_want: f64 = want;
        assert!(
            (got - f64_want).abs() < 5.0 * f64_want.sqrt(),
            "[{from}, {to}) got {got} want {want}"
        );
    }
}

#[test]
fn step_ramp_down_holds_equal_steps() {
    let profile = LoadProfile::new(vec![
        Phase::new("steady", secs(2), Shape::Hold, 1.0, 1.0),
        Phase::new("ramp_down", secs(5), Shape::Step { steps: 5 }, 1.0, 0.0),
    ])
    .unwrap();
    let starts = drain_all(&mut rate_schedule(1000.0, Arrival::Constant, profile, 1));
    let per_second: Vec<usize> = (0..7)
        .map(|s| count_in(&starts, f64::from(s), f64::from(s) + 1.0))
        .collect();
    assert_eq!(per_second, [1000, 1000, 1000, 800, 600, 400, 200]);
}

#[test]
fn step_ramp_up_starts_idle_and_rises() {
    let profile = LoadProfile::new(vec![
        Phase::new("ramp_up", secs(4), Shape::Step { steps: 4 }, 0.0, 1.0),
        Phase::new("steady", secs(1), Shape::Hold, 1.0, 1.0),
    ])
    .unwrap();
    let starts = drain_all(&mut rate_schedule(100.0, Arrival::Constant, profile, 1));
    let per_second: Vec<usize> = (0..5)
        .map(|s| count_in(&starts, f64::from(s), f64::from(s) + 1.0))
        .collect();
    assert_eq!(per_second, [0, 25, 50, 75, 100]);
}

#[test]
fn concurrency_targets_follow_the_phases() {
    let cfg = ScheduleConfig::new(Objective::ConcurrentSessions(100), ramp_steady_down(), 1);
    let mut sched = Schedule::new(cfg).unwrap();
    let clock = ManualClock::default();
    // At the start of ramp up the target is 0, and the first start is due once
    // the target reaches 1 at 0.1 s.
    assert_eq!(sched.drain(&clock, 0).count(), 0);
    let due = sched.next_due(&clock, 0).unwrap().as_secs_f64();
    assert!((due - 0.1).abs() < 1e-6, "due {due}");
    clock.set(Duration::from_secs_f64(due));
    assert_eq!(sched.drain(&clock, 0).count(), 1);
    // Half way up the ramp the target is 50.
    clock.set(secs(5));
    assert_eq!(sched.drain(&clock, 1).count(), 49);
    assert_eq!(sched.drain(&clock, 50).count(), 0);
    let due = sched.next_due(&clock, 50).unwrap().as_secs_f64();
    assert!((due - 5.1).abs() < 1e-6, "due {due}");
    // Steady keeps 100 open and refills closed sessions.
    clock.set(secs(10));
    assert_eq!(sched.drain(&clock, 50).count(), 50);
    clock.set(secs(15));
    assert_eq!(sched.drain(&clock, 97).count(), 3);
    assert_eq!(sched.next_due(&clock, 100), None);
    assert_eq!(sched.phase_name(&clock), Some("steady"));
    // Ramp down starts nothing while open sessions exceed the target.
    clock.set(secs(25));
    assert_eq!(sched.drain(&clock, 80).count(), 0);
    assert_eq!(sched.drain(&clock, 40).count(), 10);
    assert_eq!(sched.next_due(&clock, 50), None);
    assert_eq!(sched.phase_name(&clock), Some("ramp_down"));
    // After the end nothing starts.
    clock.set(secs(30));
    assert_eq!(sched.drain(&clock, 0).count(), 0);
    assert!(sched.is_finished(&clock));
    assert_eq!(sched.phase_name(&clock), None);
}

#[test]
fn concurrency_ids_and_times_come_from_the_drain() {
    let profile =
        LoadProfile::new(vec![Phase::new("steady", secs(1), Shape::Hold, 1.0, 1.0)]).unwrap();
    let mut sched = Schedule::new(ScheduleConfig::new(
        Objective::ConcurrentSessions(3),
        profile,
        0,
    ))
    .unwrap();
    let clock = ManualClock::new(Duration::from_millis(250));
    let starts: Vec<_> = sched.drain(&clock, 0).collect();
    let ids: Vec<u64> = starts.iter().map(|s| s.session_id).collect();
    assert_eq!(ids, [0, 1, 2]);
    assert!(starts
        .iter()
        .all(|s| s.scheduled_at == Duration::from_millis(250) && s.phase == 0));
}

#[test]
fn the_cap_defers_rate_arrivals_until_sessions_close() {
    let profile =
        LoadProfile::new(vec![Phase::new("steady", secs(1), Shape::Hold, 1.0, 1.0)]).unwrap();
    let mut cfg = ScheduleConfig::new(Objective::SessionsPerSecond(100.0), profile, 0);
    cfg.arrival = Arrival::Constant;
    cfg.max_concurrent = Some(10);
    let mut sched = Schedule::new(cfg).unwrap();
    let clock = ManualClock::new(secs(1));
    let first: Vec<_> = sched.drain(&clock, 0).collect();
    assert_eq!(first.len(), 10);
    assert_eq!(sched.next_due(&clock, 10), None);
    assert_eq!(sched.drain(&clock, 10).count(), 0);
    // Five sessions close, so five deferred starts follow with their own times.
    let next: Vec<_> = sched.drain(&clock, 5).collect();
    assert_eq!(next.len(), 5);
    assert_eq!(next[0].session_id, 10);
    assert!(next[0].scheduled_at < secs(1));
    assert!((next[0].scheduled_at.as_secs_f64() - 0.105).abs() < 1e-9);
    assert_eq!(sched.skipped(), 0);
    let mut total = 15;
    while !sched.is_finished(&clock) {
        total += sched.drain(&clock, 0).count();
    }
    assert_eq!(total, 100);
}

#[test]
fn the_cap_skips_rate_arrivals_with_the_skip_policy() {
    let profile =
        LoadProfile::new(vec![Phase::new("steady", secs(1), Shape::Hold, 1.0, 1.0)]).unwrap();
    let mut cfg = ScheduleConfig::new(Objective::SessionsPerSecond(100.0), profile, 0);
    cfg.arrival = Arrival::Constant;
    cfg.max_concurrent = Some(10);
    cfg.cap_policy = CapPolicy::Skip;
    let mut sched = Schedule::new(cfg).unwrap();
    let clock = ManualClock::new(secs(1));
    assert_eq!(sched.drain(&clock, 0).count(), 10);
    assert_eq!(sched.skipped(), 90);
    assert!(sched.is_finished(&clock));
}

#[test]
fn the_cap_bounds_the_concurrency_target() {
    let profile =
        LoadProfile::new(vec![Phase::new("steady", secs(1), Shape::Hold, 1.0, 1.0)]).unwrap();
    let mut cfg = ScheduleConfig::new(Objective::ConcurrentSessions(100), profile, 0);
    cfg.max_concurrent = Some(30);
    let mut sched = Schedule::new(cfg).unwrap();
    let clock = ManualClock::default();
    assert_eq!(sched.drain(&clock, 0).count(), 30);
    assert_eq!(sched.next_due(&clock, 30), None);
}

#[test]
fn invalid_targets_are_rejected() {
    let profile = ramp_steady_down;
    for objective in [
        Objective::SessionsPerSecond(0.0),
        Objective::SessionsPerSecond(-1.0),
        Objective::SessionsPerSecond(f64::NAN),
        Objective::ConcurrentSessions(0),
    ] {
        assert!(Schedule::new(ScheduleConfig::new(objective, profile(), 0)).is_err());
    }
    let mut cfg = ScheduleConfig::new(Objective::ConcurrentSessions(1), profile(), 0);
    cfg.max_concurrent = Some(0);
    assert!(Schedule::new(cfg).is_err());
}
