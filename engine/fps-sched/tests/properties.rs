// SPDX-License-Identifier: GPL-3.0-only
//! Property tests over generated load profiles.

use std::time::Duration;

use fps_sched::{
    Arrival, LoadProfile, ManualClock, Objective, Phase, Schedule, ScheduleConfig, SessionStart,
    Shape,
};
use proptest::prelude::*;

fn shape() -> impl Strategy<Value = Shape> {
    prop_oneof![
        Just(Shape::Hold),
        Just(Shape::Linear),
        (1u32..20).prop_map(|steps| Shape::Step { steps }),
    ]
}

fn phase() -> impl Strategy<Value = Phase> {
    (1u64..5_000, shape(), 0.0f64..2.0, 0.0f64..2.0)
        .prop_map(|(ms, shape, s, e)| Phase::new("p", Duration::from_millis(ms), shape, s, e))
}

fn profile() -> impl Strategy<Value = LoadProfile> {
    prop::collection::vec(phase(), 1..6).prop_map(|p| LoadProfile::new(p).unwrap())
}

fn all_starts(profile: LoadProfile, arrival: Arrival, rate: f64, seed: u64) -> Vec<SessionStart> {
    let mut cfg = ScheduleConfig::new(Objective::SessionsPerSecond(rate), profile, seed);
    cfg.arrival = arrival;
    let mut sched = Schedule::new(cfg).unwrap();
    sched.drain(&ManualClock::new(Duration::MAX), 0).collect()
}

proptest! {
    #[test]
    fn mass_inverse_round_trips(p in profile(), q in 0.0f64..=1.0) {
        let m = q * p.total_mass();
        prop_assume!(m > 0.0);
        let t = p.time_at_mass(m).unwrap();
        prop_assert!(t >= 0.0 && t <= p.total_secs() + 1e-9);
        prop_assert!((p.mass_at(t) - m).abs() <= 1e-9 * p.total_mass().max(1.0));
    }

    #[test]
    fn schedules_are_ordered_bounded_and_reproducible(
        p in profile(),
        rate in 1.0f64..500.0,
        seed in any::<u64>(),
        poisson in any::<bool>(),
    ) {
        let arrival = if poisson { Arrival::Poisson } else { Arrival::Constant };
        let a = all_starts(p.clone(), arrival, rate, seed);
        let b = all_starts(p.clone(), arrival, rate, seed);
        prop_assert_eq!(&a, &b);
        let end = Duration::from_secs_f64(p.total_secs());
        for w in a.windows(2) {
            prop_assert!(w[0].scheduled_at <= w[1].scheduled_at);
            prop_assert!(w[0].phase <= w[1].phase);
            prop_assert_eq!(w[0].session_id + 1, w[1].session_id);
        }
        for s in &a {
            prop_assert!(s.scheduled_at <= end);
            // A start never falls where the target fraction is 0.
            let t = s.scheduled_at.as_secs_f64();
            prop_assert!(p.phases()[s.phase].start_fraction > 0.0
                || p.phases()[s.phase].end_fraction > 0.0, "start at {} in an idle phase", t);
        }
        if !poisson {
            let expected = (rate * p.total_mass() + 0.5).floor() as usize;
            prop_assert!(a.len().abs_diff(expected) <= 1, "{} vs {}", a.len(), expected);
        }
    }
}
