// SPDX-License-Identifier: GPL-3.0-only
//! Component tests of the run time rate adjustment through the public interface.
//!
//! Every test drives the schedule with a `ManualClock` and never sleeps, so the
//! tests measure the rate in simulated seconds.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use fps_sched::{
    Arrival, Clock, LoadProfile, ManualClock, Objective, Phase, RateControl, RateHandle,
    SchedError, Schedule, ScheduleConfig, SessionStart, Shape,
};

/// The relative tolerance that the package acceptance states for a measured rate.
const TOLERANCE: f64 = 0.05;

fn secs(s: f64) -> Duration {
    Duration::from_secs_f64(s)
}

fn hold(total_secs: u64) -> LoadProfile {
    LoadProfile::new(vec![Phase::new(
        "hold",
        Duration::from_secs(total_secs),
        Shape::Hold,
        1.0,
        1.0,
    )])
    .unwrap()
}

fn schedule(rate: f64, arrival: Arrival, profile: LoadProfile, seed: u64) -> Schedule {
    let mut cfg = ScheduleConfig::new(Objective::SessionsPerSecond(rate), profile, seed);
    cfg.arrival = arrival;
    Schedule::new(cfg).unwrap()
}

/// Ticks the worker loop every `tick` seconds up to and including `to` and
/// returns the starts. The clock is set to every tick and no session stays open.
fn run(sched: &mut Schedule, clock: &ManualClock, to: f64, tick: f64) -> Vec<SessionStart> {
    let mut out = Vec::new();
    let mut t = clock.elapsed().as_secs_f64();
    while t + tick <= to + 1e-9 {
        t += tick;
        clock.set(secs(t));
        out.extend(sched.drain(clock, 0));
    }
    out
}

fn times(starts: &[SessionStart]) -> Vec<f64> {
    starts
        .iter()
        .map(|s| s.scheduled_at.as_secs_f64())
        .collect()
}

fn count_in(starts: &[SessionStart], from: f64, to: f64) -> usize {
    times(starts)
        .iter()
        .filter(|&&t| t >= from && t < to)
        .count()
}

fn rate_of(sched: &Schedule) -> f64 {
    match sched.config().objective {
        Objective::SessionsPerSecond(r) => r,
        Objective::ConcurrentSessions(_) => f64::NAN,
    }
}

#[test]
fn a_change_takes_effect_at_the_next_tick() {
    let mut sched = schedule(10.0, Arrival::Constant, hold(100), 1);
    let handle = sched.rate_handle();
    let clock = ManualClock::default();
    let before = run(&mut sched, &clock, 5.0, 0.01);
    assert_eq!(before.len(), 50);

    handle.set(Objective::SessionsPerSecond(100.0)).unwrap();
    // The schedule has not applied the target yet, and next_due asks for a tick now.
    assert_eq!(rate_of(&sched), 10.0);
    assert_eq!(handle.get(), Objective::SessionsPerSecond(100.0));
    assert!(sched.next_due(&clock, 0).unwrap() <= clock.elapsed());

    // At the next tick the schedule applies the new rate. The change lands at
    // that tick, 5.01 s, where the expected count is 50.1, so the next start at
    // count 50.5 lies 4 ms later, and later starts follow at 10 ms.
    clock.set(secs(5.01));
    assert_eq!(sched.drain(&clock, 0).count(), 0);
    assert_eq!(rate_of(&sched), 100.0);
    let due = sched.next_due(&clock, 0).unwrap().as_secs_f64();
    assert!((due - 5.014).abs() < 1e-6, "next start at {due}");
    clock.set(secs(5.02));
    let starts: Vec<_> = sched.drain(&clock, 0).collect();
    assert_eq!(starts.len(), 1);
    assert!((starts[0].scheduled_at.as_secs_f64() - 5.014).abs() < 1e-6);
    let due = sched.next_due(&clock, 0).unwrap().as_secs_f64();
    assert!((due - 5.024).abs() < 1e-6, "following start at {due}");
}

#[test]
fn constant_arrivals_have_no_burst_and_no_gap_across_a_change() {
    for (old, new) in [(10.0, 20.0), (100.0, 10.0), (10.0, 10.5)] {
        let mut sched = schedule(old, Arrival::Constant, hold(100), 1);
        let handle = sched.rate_handle();
        let clock = ManualClock::default();
        let mut starts = run(&mut sched, &clock, 5.03, 0.001);
        handle.set(Objective::SessionsPerSecond(new)).unwrap();
        starts.extend(run(&mut sched, &clock, 8.0, 0.001));

        let t = times(&starts);
        let slow = 1.0 / f64::min(old, new);
        let fast = 1.0 / f64::max(old, new);
        for pair in t.windows(2) {
            let gap = pair[1] - pair[0];
            // Every gap lies between the old spacing and the new one, so the
            // change neither bunches starts up nor leaves a hole.
            assert!(
                gap >= fast - 1e-6 && gap <= slow + 1e-6,
                "{old} to {new}: gap {gap} outside [{fast}, {slow}]"
            );
        }
        // The expected count is continuous at the change, so the total count
        // is the count at the change plus the new rate over the rest, rounded
        // by the half offset of constant arrivals.
        let expected = old * 5.03 + new * (8.0 - 5.03);
        let got = starts.len() as f64;
        assert!(
            (got - expected).abs() <= 1.0,
            "{old} to {new}: {got} starts, expected {expected}"
        );
    }
}

#[test]
fn the_realized_rate_after_two_changes_matches_the_new_rates() {
    // Three windows of 400 simulated seconds at 20, 50, and 30 sessions per
    // second. A Poisson count of 8000 or more has a relative standard deviation
    // of 1.1% or less, so 5% leaves four sigma, and the seed fixes the draw.
    let windows = [(20.0, 0.0), (50.0, 400.0), (30.0, 800.0)];
    let mut sched = schedule(20.0, Arrival::Poisson, hold(1200), 7);
    let handle = sched.rate_handle();
    let clock = ManualClock::default();
    let mut starts = Vec::new();
    for &(rate, from) in &windows {
        if from > 0.0 {
            handle.set(Objective::SessionsPerSecond(rate)).unwrap();
        }
        starts.extend(run(&mut sched, &clock, from + 400.0, 0.01));
    }
    for &(rate, from) in &windows {
        let got = count_in(&starts, from, from + 400.0) as f64 / 400.0;
        let err = (got - rate).abs() / rate;
        assert!(
            err <= TOLERANCE,
            "window at {from}: {got}/s against {rate}/s"
        );
    }
}

#[test]
fn constant_arrivals_realize_the_new_rate_exactly_after_two_changes() {
    let mut sched = schedule(8.0, Arrival::Constant, hold(300), 1);
    let handle = sched.rate_handle();
    let clock = ManualClock::default();
    let mut starts = run(&mut sched, &clock, 100.0, 0.05);
    handle.set(Objective::SessionsPerSecond(40.0)).unwrap();
    starts.extend(run(&mut sched, &clock, 200.0, 0.05));
    handle.set(Objective::SessionsPerSecond(15.0)).unwrap();
    starts.extend(run(&mut sched, &clock, 300.0, 0.05));
    for (rate, from) in [(8.0, 0.0), (40.0, 100.0), (15.0, 200.0)] {
        let got = count_in(&starts, from, from + 100.0) as f64 / 100.0;
        assert!(
            (got - rate).abs() / rate <= 0.01,
            "{got}/s against {rate}/s"
        );
    }
    assert!(sched.is_finished(&clock));
}

#[test]
fn poisson_arrivals_stay_poisson_across_a_change() {
    // The time change theorem says that N(t), the integral of the rate, turns
    // the arrivals into a unit rate process, so the gaps of N(t_i) are
    // exponential with mean 1. Test it with the two sided KS statistic.
    let mut sched = schedule(50.0, Arrival::Poisson, hold(400), 11);
    let handle = sched.rate_handle();
    let clock = ManualClock::default();
    let mut starts = run(&mut sched, &clock, 100.0, 0.001);
    handle.set(Objective::SessionsPerSecond(150.0)).unwrap();
    starts.extend(run(&mut sched, &clock, 200.0, 0.001));
    handle.set(Objective::SessionsPerSecond(20.0)).unwrap();
    starts.extend(run(&mut sched, &clock, 400.0, 0.001));

    // The ticks are 1 ms, so a change lands within 1 ms of 100 s and 200 s. A
    // start at the tick boundary shifts N by at most 0.15 of one arrival, which
    // the test absorbs by using the nominal boundaries.
    let n = |t: f64| {
        if t < 100.0 {
            50.0 * t
        } else if t < 200.0 {
            5000.0 + 150.0 * (t - 100.0)
        } else {
            20000.0 + 20.0 * (t - 200.0)
        }
    };
    let mut gaps: Vec<f64> = times(&starts)
        .windows(2)
        .map(|p| n(p[1]) - n(p[0]))
        .collect();
    let count = gaps.len() as f64;
    gaps.sort_by(f64::total_cmp);
    let ks = gaps
        .iter()
        .enumerate()
        .map(|(i, &x)| {
            let f = 1.0 - (-x).exp();
            (f - i as f64 / count)
                .abs()
                .max(((i + 1) as f64 / count - f).abs())
        })
        .fold(0.0, f64::max);
    let critical = 1.358 / count.sqrt();
    assert!(ks < critical, "KS statistic {ks} above {critical}");
}

#[test]
fn rate_zero_pauses_and_the_resume_owes_nothing() {
    let mut sched = schedule(10.0, Arrival::Constant, hold(100), 1);
    let handle = sched.rate_handle();
    let clock = ManualClock::default();
    let mut starts = run(&mut sched, &clock, 5.0, 0.01);
    assert_eq!(starts.len(), 50);

    handle.set(Objective::SessionsPerSecond(0.0)).unwrap();
    let paused = run(&mut sched, &clock, 15.0, 0.01);
    assert!(paused.is_empty(), "a paused schedule starts nothing");
    assert!(!sched.is_finished(&clock), "a pause is not the end");
    assert_eq!(sched.next_due(&clock, 0), None);

    handle.set(Objective::SessionsPerSecond(10.0)).unwrap();
    starts.extend(run(&mut sched, &clock, 16.0, 0.01));
    // 50 starts before the pause and 10 in the second after it. No start
    // catches up the ten paused seconds.
    assert_eq!(starts.len(), 60);
    assert!(count_in(&starts, 15.0, 15.1) <= 1, "no burst at the resume");
    assert!(times(&starts).iter().all(|&t| !(5.0..15.0).contains(&t)));
}

#[test]
fn a_schedule_can_pause_and_resume_with_poisson_arrivals_and_a_new_rate() {
    let mut sched = schedule(30.0, Arrival::Poisson, hold(1000), 3);
    let handle = sched.rate_handle();
    let clock = ManualClock::default();
    run(&mut sched, &clock, 10.0, 0.01);
    handle.set(Objective::SessionsPerSecond(0.0)).unwrap();
    // The pause applies at the next tick, 50 ms away, so one start that fell
    // due before that tick may still go out and nothing after it.
    let paused = run(&mut sched, &clock, 110.0, 0.05);
    assert!(paused.len() <= 1);
    assert!(times(&paused).iter().all(|&t| t <= 10.06));
    handle.set(Objective::SessionsPerSecond(40.0)).unwrap();
    let after = run(&mut sched, &clock, 410.0, 0.01);
    let got = after.len() as f64 / 300.0;
    assert!((got - 40.0).abs() / 40.0 <= TOLERANCE, "{got}/s");
    assert!(times(&after).iter().all(|&t| t >= 110.0));
}

#[test]
fn a_raised_rate_can_start_arrivals_that_the_old_rate_left_beyond_the_end() {
    // 1 s at 1/s holds one constant start at 0.5 s. The second start, at count
    // 1.5, lies beyond the end. Doubling the rate at 0.6 s brings it inside.
    let mut sched = schedule(1.0, Arrival::Constant, hold(1), 1);
    let handle = sched.rate_handle();
    let clock = ManualClock::default();
    let first = run(&mut sched, &clock, 0.6, 0.1);
    assert_eq!(first.len(), 1);
    assert!(sched.is_finished(&clock), "one start only at the old rate");
    handle.set(Objective::SessionsPerSecond(4.0)).unwrap();
    assert!(!sched.is_finished(&clock), "a waiting change may revive it");
    let rest = run(&mut sched, &clock, 1.0, 0.1);
    assert!(!rest.is_empty());
    assert!(times(&rest).iter().all(|&t| t > 0.6 && t <= 1.0));
}

#[test]
fn a_change_follows_the_load_profile() {
    // A hold at fraction 1 followed by a hold at fraction 0.5. The fraction
    // scales the new rate like the old one.
    let profile = LoadProfile::new(vec![
        Phase::new("a", Duration::from_secs(50), Shape::Hold, 1.0, 1.0),
        Phase::new("b", Duration::from_secs(50), Shape::Hold, 0.5, 0.5),
    ])
    .unwrap();
    let mut sched = schedule(10.0, Arrival::Constant, profile, 1);
    let handle = sched.rate_handle();
    let clock = ManualClock::default();
    let mut starts = run(&mut sched, &clock, 40.0, 0.05);
    handle.set(Objective::SessionsPerSecond(20.0)).unwrap();
    starts.extend(run(&mut sched, &clock, 100.0, 0.05));
    assert!((count_in(&starts, 0.0, 40.0) as f64 - 400.0).abs() <= 1.0);
    // 10 s at 20/s in phase a, then 50 s at 10/s in phase b.
    assert!((count_in(&starts, 40.0, 50.0) as f64 - 200.0).abs() <= 2.0);
    assert!((count_in(&starts, 50.0, 100.0) as f64 - 500.0).abs() <= 2.0);
}

#[test]
fn a_concurrency_target_follows_the_handle() {
    let cfg = ScheduleConfig::new(Objective::ConcurrentSessions(4), hold(100), 1);
    let mut sched = Schedule::new(cfg).unwrap();
    let handle = sched.rate_handle();
    let clock = ManualClock::new(secs(1.0));
    assert_eq!(sched.drain(&clock, 0).count(), 4);

    handle.set(Objective::ConcurrentSessions(10)).unwrap();
    assert_eq!(sched.next_due(&clock, 4), Some(clock.elapsed()));
    assert_eq!(sched.drain(&clock, 4).count(), 6);

    handle.set(Objective::ConcurrentSessions(0)).unwrap();
    clock.set(secs(2.0));
    assert_eq!(sched.drain(&clock, 10).count(), 0);
    assert_eq!(sched.next_due(&clock, 0), None);
    assert!(!sched.is_finished(&clock));

    handle.set(Objective::ConcurrentSessions(3)).unwrap();
    assert_eq!(sched.drain(&clock, 1).count(), 2);
    assert_eq!(
        handle.set(Objective::SessionsPerSecond(5.0)),
        Err(SchedError::KindMismatch)
    );
}

#[test]
fn a_handle_refuses_invalid_rates_and_keeps_its_value() {
    let sched = schedule(10.0, Arrival::Constant, hold(10), 1);
    let handle = sched.rate_handle();
    for bad in [-1.0, f64::NAN, f64::INFINITY] {
        assert!(matches!(
            handle.set(Objective::SessionsPerSecond(bad)),
            Err(SchedError::InvalidRate(_))
        ));
    }
    assert_eq!(handle.get(), Objective::SessionsPerSecond(10.0));
}

#[test]
fn an_unchanged_schedule_is_identical_to_one_without_a_handle_use() {
    // Reading the handle must not disturb the seeded sequence.
    let mut a = schedule(25.0, Arrival::Poisson, hold(60), 99);
    let mut b = schedule(25.0, Arrival::Poisson, hold(60), 99);
    let _unused = b.rate_handle();
    let clock = ManualClock::new(Duration::MAX);
    let xs: Vec<_> = a.drain(&clock, 0).collect();
    let ys: Vec<_> = b.drain(&clock, 0).collect();
    assert_eq!(xs, ys);
    // Setting the current value again changes nothing either.
    let mut c = schedule(25.0, Arrival::Poisson, hold(60), 99);
    c.rate_handle()
        .set(Objective::SessionsPerSecond(25.0))
        .unwrap();
    assert_eq!(c.drain(&clock, 0).collect::<Vec<_>>(), xs);
}

fn registry() -> (RateControl, Vec<RateHandle>) {
    let control = RateControl::new();
    let mut handles = Vec::new();
    for (scenario, rate) in [("web", 10.0), ("web", 30.0), ("dns", 5.0)] {
        let sched = schedule(rate, Arrival::Constant, hold(10), 1);
        let handle = sched.rate_handle();
        control.register(scenario, handle.clone()).unwrap();
        handles.push(handle);
    }
    (control, handles)
}

fn rates(handles: &[RateHandle]) -> Vec<f64> {
    handles
        .iter()
        .map(|h| match h.get() {
            Objective::SessionsPerSecond(r) => r,
            Objective::ConcurrentSessions(n) => f64::from(n),
        })
        .collect()
}

#[test]
fn an_update_divides_the_total_in_proportion_to_the_current_rates() {
    let (control, handles) = registry();
    control
        .update(&[("web", Objective::SessionsPerSecond(80.0))])
        .unwrap();
    assert_eq!(rates(&handles), vec![20.0, 60.0, 5.0]);
    assert_eq!(
        control.total("web"),
        Some(Objective::SessionsPerSecond(80.0))
    );
    // A paused scenario divides equally when it resumes.
    control
        .update(&[("web", Objective::SessionsPerSecond(0.0))])
        .unwrap();
    assert_eq!(rates(&handles), vec![0.0, 0.0, 5.0]);
    control
        .update(&[("web", Objective::SessionsPerSecond(50.0))])
        .unwrap();
    assert_eq!(rates(&handles), vec![25.0, 25.0, 5.0]);
}

#[test]
fn an_update_with_an_unknown_scenario_changes_nothing() {
    let (control, handles) = registry();
    let err = control
        .update(&[
            ("dns", Objective::SessionsPerSecond(99.0)),
            ("nope", Objective::SessionsPerSecond(1.0)),
        ])
        .unwrap_err();
    assert_eq!(err, SchedError::UnknownScenario("nope".to_owned()));
    assert_eq!(rates(&handles), vec![10.0, 30.0, 5.0]);
}

#[test]
fn an_invalid_update_is_applied_completely_or_not_at_all() {
    let (control, handles) = registry();
    let good = ("dns", Objective::SessionsPerSecond(9.0));
    assert!(matches!(
        control.update(&[good, ("web", Objective::SessionsPerSecond(-1.0))]),
        Err(SchedError::InvalidRate(_))
    ));
    assert!(matches!(
        control.update(&[good, ("web", Objective::SessionsPerSecond(f64::NAN))]),
        Err(SchedError::InvalidRate(_))
    ));
    assert_eq!(
        control.update(&[good, ("web", Objective::ConcurrentSessions(3))]),
        Err(SchedError::KindMismatch)
    );
    assert_eq!(
        control.update(&[good, good]),
        Err(SchedError::DuplicateScenario("dns".to_owned()))
    );
    assert_eq!(rates(&handles), vec![10.0, 30.0, 5.0]);
    assert!(control.update(&[]).is_ok());
}

#[test]
fn a_factor_scales_the_registered_rates_and_is_idempotent() {
    let (control, handles) = registry();
    control.scale(&[("web", 0.5), ("dns", 2.0)]).unwrap();
    assert_eq!(rates(&handles), vec![5.0, 15.0, 10.0]);
    control.scale(&[("web", 0.5)]).unwrap();
    assert_eq!(rates(&handles), vec![5.0, 15.0, 10.0]);
    control.scale(&[("web", 1.0), ("dns", 1.0)]).unwrap();
    assert_eq!(rates(&handles), vec![10.0, 30.0, 5.0]);
    control.scale(&[("dns", 0.0)]).unwrap();
    assert_eq!(rates(&handles), vec![10.0, 30.0, 0.0]);
    assert_eq!(
        control.scale(&[("web", 3.0), ("none", 1.0)]),
        Err(SchedError::UnknownScenario("none".to_owned()))
    );
    assert!(matches!(
        control.scale(&[("web", f64::INFINITY)]),
        Err(SchedError::InvalidRate(_))
    ));
    assert_eq!(rates(&handles), vec![10.0, 30.0, 0.0]);
}

#[test]
fn a_concurrency_total_divides_into_whole_sessions() {
    let control = RateControl::new();
    let mut handles = Vec::new();
    for _ in 0..3 {
        let cfg = ScheduleConfig::new(Objective::ConcurrentSessions(2), hold(10), 1);
        let handle = Schedule::new(cfg).unwrap().rate_handle();
        control.register("conc", handle.clone()).unwrap();
        handles.push(handle);
    }
    control
        .update(&[("conc", Objective::ConcurrentSessions(7))])
        .unwrap();
    let got = rates(&handles);
    assert_eq!(got.iter().sum::<f64>(), 7.0);
    assert!(got
        .iter()
        .all(|&v| v == v.floor() && (2.0..=3.0).contains(&v)));
    assert_eq!(
        control.total("conc"),
        Some(Objective::ConcurrentSessions(7))
    );
    let rate_handle = schedule(1.0, Arrival::Constant, hold(10), 1).rate_handle();
    assert_eq!(
        control.register("conc", rate_handle),
        Err(SchedError::KindMismatch)
    );
}

#[test]
fn concurrent_updates_from_other_threads_are_safe() {
    const ALLOWED: [f64; 4] = [5.0, 10.0, 20.0, 40.0];
    let mut sched = schedule(10.0, Arrival::Poisson, hold(100_000), 5);
    let handle = sched.rate_handle();
    let done = AtomicBool::new(false);
    let starts = std::thread::scope(|scope| {
        let updaters: Vec<_> = (0..4)
            .map(|i| {
                let h = handle.clone();
                scope.spawn(move || {
                    for n in 0..20_000usize {
                        h.set(Objective::SessionsPerSecond(ALLOWED[(n + i) % 4]))
                            .unwrap();
                    }
                })
            })
            .collect();
        // The worker owns the schedule and its clock, and ticks while the
        // updaters write. Every rate that it applies must be one that an
        // updater wrote, so a torn read would fail the check.
        let worker = scope.spawn(|| {
            let clock = ManualClock::default();
            let mut all = Vec::new();
            let mut t = 0.0;
            while !done.load(Ordering::Acquire) || t < 200.0 {
                t += 0.01;
                clock.set(secs(t));
                all.extend(sched.drain(&clock, 0));
                assert!(ALLOWED.contains(&rate_of(&sched)) || rate_of(&sched) == 10.0);
            }
            all
        });
        for u in updaters {
            u.join().unwrap();
        }
        done.store(true, Ordering::Release);
        worker.join().unwrap()
    });
    assert!(!starts.is_empty());
    let t = times(&starts);
    assert!(
        t.windows(2).all(|p| p[0] <= p[1]),
        "scheduled times stay ordered"
    );
    assert!(starts
        .iter()
        .enumerate()
        .all(|(i, s)| s.session_id == i as u64));
    assert!(ALLOWED.contains(&match handle.get() {
        Objective::SessionsPerSecond(r) => r,
        Objective::ConcurrentSessions(_) => f64::NAN,
    }));
}

#[test]
fn concurrent_registry_updates_are_all_or_nothing_for_readers() {
    let control = RateControl::new();
    let mut handles = Vec::new();
    for scenario in ["a", "b"] {
        let handle = schedule(1.0, Arrival::Constant, hold(10), 1).rate_handle();
        control.register(scenario, handle.clone()).unwrap();
        handles.push(handle);
    }
    std::thread::scope(|scope| {
        let writers: Vec<_> = (0..4)
            .map(|i| {
                let control = &control;
                scope.spawn(move || {
                    for n in 0..5_000u32 {
                        let v = f64::from(1 + (n + i) % 9);
                        control
                            .update(&[
                                ("a", Objective::SessionsPerSecond(v)),
                                ("b", Objective::SessionsPerSecond(v)),
                            ])
                            .unwrap();
                        // A failing update must not leak a half applied state.
                        assert!(control
                            .update(&[
                                ("a", Objective::SessionsPerSecond(100.0)),
                                ("zzz", Objective::SessionsPerSecond(1.0)),
                            ])
                            .is_err());
                    }
                })
            })
            .collect();
        for _ in 0..5_000 {
            // total() holds the registry lock, so a reader never sees the 100
            // of a failed update, which would show a half applied update.
            let a = control.total("a");
            let b = control.total("b");
            assert!(matches!(a, Some(Objective::SessionsPerSecond(v)) if v != 100.0));
            assert!(matches!(b, Some(Objective::SessionsPerSecond(v)) if v != 100.0));
        }
        for w in writers {
            w.join().unwrap();
        }
    });
    assert_eq!(control.total("a"), control.total("b"));
}

mod properties {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        /// For any two rates and any change time, constant arrivals keep the
        /// continuous expected count and every gap between the two spacings.
        #[test]
        fn a_change_keeps_the_expected_count_and_the_gaps(
            old in 1.0f64..200.0,
            new in 1.0f64..200.0,
            at in 1.0f64..20.0,
        ) {
            let tick = 0.002;
            let mut sched = schedule(old, Arrival::Constant, hold(60), 1);
            let handle = sched.rate_handle();
            let clock = ManualClock::default();
            let mut starts = run(&mut sched, &clock, at, tick);
            let changed_at = clock.elapsed().as_secs_f64();
            handle.set(Objective::SessionsPerSecond(new)).unwrap();
            starts.extend(run(&mut sched, &clock, at + 10.0, tick));
            let end = clock.elapsed().as_secs_f64();
            // The new rate applies at the first tick after the change.
            let expected = old * (changed_at + tick) + new * (end - changed_at - tick);
            prop_assert!((starts.len() as f64 - expected).abs() <= 1.5);
            let (fast, slow) = (1.0 / old.max(new), 1.0 / old.min(new));
            for p in times(&starts).windows(2) {
                let gap = p[1] - p[0];
                prop_assert!(gap >= fast - 1e-6 && gap <= slow + 1e-6, "gap {gap}");
            }
        }
    }
}
