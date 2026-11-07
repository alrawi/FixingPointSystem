// SPDX-License-Identifier: GPL-3.0-only
//! Run time rate adjustment.
//!
//! Every [`Schedule`](crate::Schedule) owns one [`RateHandle`], a cheap clone
//! of one atomic cell that holds the current target. Any thread may store a
//! new target through a clone, and the worker that owns the schedule loads the
//! cell at its next scheduling tick, which is its next call of
//! [`Schedule::drain`](crate::Schedule::drain). The worker takes no lock and
//! never restarts.
//!
//! A [`RateControl`] is the registry of an agent. It maps a scenario id to the
//! handles of the worker rates that serve it, so one update names the new
//! target of the whole agent and the registry divides it among the workers.
//! An update is checked completely before it changes anything, so an unknown
//! scenario id or an invalid value leaves every rate as it was.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use crate::schedule::Objective;
use crate::SchedError;

/// The shared cell that holds one target, and the kind of the target.
#[derive(Debug)]
struct Cell {
    /// The bits of the target as an `f64`. A concurrency target is an integer
    /// below 2^32, which an `f64` holds exactly.
    bits: AtomicU64,
    concurrent: bool,
}

/// A thread safe handle on the target of one schedule.
///
/// The handle keeps the kind of the objective that created it. A rate handle
/// takes sessions per second and a concurrency handle takes open sessions, and
/// [`RateHandle::set`] refuses the other kind. A target of 0 pauses the
/// schedule, see [`Schedule::rate_handle`](crate::Schedule::rate_handle).
#[derive(Debug, Clone)]
pub struct RateHandle {
    cell: Arc<Cell>,
}

impl RateHandle {
    /// Returns a handle that holds `objective` as its first target.
    pub(crate) fn new(objective: Objective) -> Self {
        let (value, concurrent) = match objective {
            Objective::SessionsPerSecond(r) => (r, false),
            Objective::ConcurrentSessions(n) => (f64::from(n), true),
        };
        Self {
            cell: Arc::new(Cell {
                bits: AtomicU64::new(value.to_bits()),
                concurrent,
            }),
        }
    }

    /// Returns the raw bits that the schedule compares with the bits it applied.
    pub(crate) fn bits(&self) -> u64 {
        self.cell.bits.load(Ordering::Acquire)
    }

    /// Returns the target that the handle holds now, which the schedule may not
    /// have applied yet.
    pub fn get(&self) -> Objective {
        let value = f64::from_bits(self.bits());
        if self.cell.concurrent {
            Objective::ConcurrentSessions(value as u32)
        } else {
            Objective::SessionsPerSecond(value)
        }
    }

    /// Returns true when the handle belongs to a concurrency objective.
    pub fn is_concurrent(&self) -> bool {
        self.cell.concurrent
    }

    /// Stores a new target. The schedule applies it at its next tick.
    ///
    /// A rate must be finite and not negative, and 0 pauses the schedule. A
    /// kind other than the kind of the handle fails with
    /// [`SchedError::KindMismatch`].
    pub fn set(&self, target: Objective) -> Result<(), SchedError> {
        let value = match (target, self.cell.concurrent) {
            (Objective::SessionsPerSecond(r), false) => {
                if !(r.is_finite() && r >= 0.0) {
                    return Err(SchedError::InvalidRate(r));
                }
                r
            }
            (Objective::ConcurrentSessions(n), true) => f64::from(n),
            _ => return Err(SchedError::KindMismatch),
        };
        self.store_value(value);
        Ok(())
    }

    fn value(&self) -> f64 {
        f64::from_bits(self.bits())
    }

    fn store_value(&self, value: f64) {
        self.cell.bits.store(value.to_bits(), Ordering::Release);
    }
}

/// One registered worker rate.
#[derive(Debug)]
struct Entry {
    handle: RateHandle,
    /// The target at registration, which a factor scales.
    base: f64,
}

/// The registry of the worker rates of one agent, by scenario id.
///
/// The scenario id is the name of the mix entry. Several workers serve one
/// scenario, so a scenario holds several handles. Updates take a lock among
/// themselves only. The workers read their atomic cells and never touch it.
/// Each worker applies its share at its own next tick, so the shares of one
/// update can take effect on slightly different ticks.
#[derive(Debug, Default)]
pub struct RateControl {
    scenarios: Mutex<BTreeMap<String, Vec<Entry>>>,
}

impl RateControl {
    /// Returns an empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers the handle of one worker rate under a scenario id.
    ///
    /// All handles of a scenario must share one kind, or the call fails with
    /// [`SchedError::KindMismatch`].
    pub fn register(&self, scenario: &str, handle: RateHandle) -> Result<(), SchedError> {
        let mut map = self.lock();
        let list = map.entry(scenario.to_owned()).or_default();
        if list
            .iter()
            .any(|e| e.handle.is_concurrent() != handle.is_concurrent())
        {
            return Err(SchedError::KindMismatch);
        }
        let base = handle.value();
        list.push(Entry { handle, base });
        Ok(())
    }

    /// Returns the scenario ids in order.
    pub fn scenarios(&self) -> Vec<String> {
        self.lock().keys().cloned().collect()
    }

    /// Returns the current total target of a scenario, or `None` when the id
    /// is unknown. A rate scenario sums the rates and a concurrency scenario
    /// sums the open session targets.
    pub fn total(&self, scenario: &str) -> Option<Objective> {
        let map = self.lock();
        let list = map.get(scenario)?;
        let sum: f64 = list.iter().map(|e| e.handle.value()).sum();
        Some(objective_of(list, sum))
    }

    /// Sets the total target of each named scenario, as `UpdateRates` of the
    /// agent protocol does.
    ///
    /// The registry divides each total among the workers of the scenario in
    /// proportion to their current targets, or equally when they are all 0. A
    /// concurrency total is divided in whole sessions that add up to the total.
    /// A scenario that the update does not name keeps its targets. The call
    /// changes nothing and returns an error when any id is unknown or repeated,
    /// any kind differs from the kind of its scenario, or any value is invalid.
    pub fn update(&self, updates: &[(&str, Objective)]) -> Result<(), SchedError> {
        let map = self.lock();
        let mut plan: Plan<'_> = Vec::with_capacity(updates.len());
        for (id, target) in updates {
            let list = check_id(&map, &plan, id)?;
            let total = total_value(list, *target)?;
            let weights: Vec<f64> = list.iter().map(|e| e.handle.value()).collect();
            let concurrent = list[0].handle.is_concurrent();
            plan.push((*id, list, distribute(total, &weights, concurrent)));
        }
        apply(&plan);
        Ok(())
    }

    /// Scales the registered target of each named scenario by a factor, as the
    /// run calibration of the controller produces one factor per mix entry.
    ///
    /// The factor applies to the target at registration and not to the current
    /// target, so a repeated factor is idempotent and a factor of 1 restores the
    /// compiled rate. A factor of 0 pauses the scenario. The same all or nothing
    /// rules as [`RateControl::update`] apply, and a factor must be finite and
    /// not negative.
    pub fn scale(&self, factors: &[(&str, f64)]) -> Result<(), SchedError> {
        let map = self.lock();
        let mut plan: Plan<'_> = Vec::with_capacity(factors.len());
        for (id, factor) in factors {
            let list = check_id(&map, &plan, id)?;
            if !(factor.is_finite() && *factor >= 0.0) {
                return Err(SchedError::InvalidRate(*factor));
            }
            let weights: Vec<f64> = list.iter().map(|e| e.base).collect();
            let total: f64 = weights.iter().sum::<f64>() * factor;
            let concurrent = list[0].handle.is_concurrent();
            if !total.is_finite() || (concurrent && total.round() > f64::from(u32::MAX)) {
                return Err(SchedError::InvalidRate(total));
            }
            plan.push((*id, list, distribute(total, &weights, concurrent)));
        }
        apply(&plan);
        Ok(())
    }

    fn lock(&self) -> MutexGuard<'_, BTreeMap<String, Vec<Entry>>> {
        // Every write happens after all checks, so a panic while the lock was
        // held cannot leave the map half written and a poisoned lock is safe.
        self.scenarios
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }
}

/// The checked shares of an update, by scenario.
type Plan<'a> = Vec<(&'a str, &'a [Entry], Vec<f64>)>;

fn objective_of(list: &[Entry], value: f64) -> Objective {
    if list.first().is_some_and(|e| e.handle.is_concurrent()) {
        Objective::ConcurrentSessions(value.round() as u32)
    } else {
        Objective::SessionsPerSecond(value)
    }
}

/// Finds the scenario and rejects an id that the update repeats.
fn check_id<'a>(
    map: &'a BTreeMap<String, Vec<Entry>>,
    plan: &Plan<'_>,
    id: &str,
) -> Result<&'a [Entry], SchedError> {
    let list = map
        .get(id)
        .filter(|l| !l.is_empty())
        .ok_or_else(|| SchedError::UnknownScenario(id.to_owned()))?;
    if plan.iter().any(|(seen, _, _)| *seen == id) {
        return Err(SchedError::DuplicateScenario(id.to_owned()));
    }
    Ok(list)
}

/// Checks the kind and the value of one total.
fn total_value(list: &[Entry], target: Objective) -> Result<f64, SchedError> {
    match (target, list[0].handle.is_concurrent()) {
        (Objective::SessionsPerSecond(r), false) if r.is_finite() && r >= 0.0 => Ok(r),
        (Objective::SessionsPerSecond(r), false) => Err(SchedError::InvalidRate(r)),
        (Objective::ConcurrentSessions(n), true) => Ok(f64::from(n)),
        _ => Err(SchedError::KindMismatch),
    }
}

fn apply(plan: &Plan<'_>) {
    for (_, list, shares) in plan {
        for (entry, share) in list.iter().zip(shares) {
            entry.handle.store_value(*share);
        }
    }
}

/// Divides `total` in proportion to `weights`, or equally when they sum to 0.
/// A concurrency total divides in whole numbers that add up to the total, and
/// the largest remainders take the leftover sessions, first index first.
fn distribute(total: f64, weights: &[f64], concurrent: bool) -> Vec<f64> {
    let n = weights.len();
    let sum: f64 = weights.iter().sum();
    let ideal: Vec<f64> = if sum > 0.0 {
        weights.iter().map(|w| total * w / sum).collect()
    } else {
        vec![total / n as f64; n]
    };
    if !concurrent {
        return ideal;
    }
    let total = total.round();
    let mut shares: Vec<f64> = ideal.iter().map(|v| v.floor()).collect();
    let mut left = (total - shares.iter().sum::<f64>()).max(0.0) as usize;
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|&a, &b| {
        let ra = ideal[a] - shares[a];
        let rb = ideal[b] - shares[b];
        rb.total_cmp(&ra).then(a.cmp(&b))
    });
    for i in order {
        if left == 0 {
            break;
        }
        shares[i] += 1.0;
        left -= 1;
    }
    shares
}
