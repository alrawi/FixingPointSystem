// SPDX-License-Identifier: GPL-3.0-only
//! Datapath lanes that follow the workers of the plan (AGT-30).
//!
//! The AF_XDP backend runs one own stack per lane, each on its own thread and
//! on queue `k` of both ports. A plan that names several workers wants its
//! load spread over several cores, so the agent starts one lane per worker, as
//! far as the ports have queues. Each lane serves the hosts of its workers:
//! a client host reaches only the server hosts of its own lane (see
//! `fps_datapath::split_pools`), and a worker sends to the server range that
//! it owns, so a session stays on one lane when the lane holds the ranges of
//! the workers that open it.
//!
//! [`plan_lanes`] is a pure function of the plan, so tests need no device.
//! It divides the workers, in the order of their ids, into as many groups as
//! there are lanes, and a lane takes the smallest range of each pool that holds
//! the ranges of its workers. It starts one lane over the whole pools, as the
//! agent did before AGT-30, when the plan does not split cleanly: a worker
//! with a rate and no server range sends to the whole server pool, which
//! several lanes cannot serve, and ranges of different lanes must not overlap.
//! The reason comes back in [`LaneLayout::note`].

use std::path::Path;

use fps_hosts::HostRange;

use crate::plan::{HostPoolSlice, Plan, WorkerPlan};

/// What one lane serves.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaneSpec {
    /// The ids of the plan workers whose hosts the lane serves.
    pub workers: Vec<u32>,
    /// The client hosts of the lane, as indexes of the client pool.
    pub client: HostRange,
    /// The server hosts of the lane, as indexes of the server pool.
    pub server: HostRange,
}

/// The lanes of a run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaneLayout {
    /// One entry per lane, in the order of the queues: lane `k` uses queue
    /// `k` of each port.
    pub lanes: Vec<LaneSpec>,
    /// Why the layout has fewer lanes than the plan has workers with hosts
    /// and the limit allowed, or `None` when it has as many as it could.
    pub note: Option<String>,
}

/// The hosts of one plan worker in the two pools of the run.
struct Owner<'p> {
    worker: &'p WorkerPlan,
    client: Option<HostRange>,
    server: Option<HostRange>,
}

fn range_in(worker: &WorkerPlan, pool: &HostPoolSlice) -> Option<HostRange> {
    worker
        .hosts
        .iter()
        .find(|h| h.pool == pool.name)
        .map(|h| HostRange::new(h.offset, h.count))
}

/// The smallest range that holds every range of `parts`, or `None` when `parts`
/// is empty.
fn hull(parts: impl Iterator<Item = HostRange>) -> Option<HostRange> {
    let mut found: Option<(u32, u64)> = None;
    for r in parts {
        found = Some(match found {
            None => (r.offset, r.end()),
            Some((start, end)) => (start.min(r.offset), end.max(r.end())),
        });
    }
    found.map(|(start, end)| {
        HostRange::new(
            start,
            u32::try_from(end - u64::from(start)).unwrap_or(u32::MAX),
        )
    })
}

/// Reports whether any two of `ranges` share a host.
fn overlap(mut ranges: Vec<HostRange>) -> bool {
    ranges.sort_by_key(|r| r.offset);
    ranges
        .windows(2)
        .any(|w| w[0].end() > u64::from(w[1].offset))
}

/// Divides `plan`'s workers into at most `limit` lanes.
///
/// `client` and `server` are the pools of the run. `limit` is the most lanes
/// that the caller can start, the smaller queue count of the two ports. The
/// result always holds at least one lane, and a layout of one lane serves the
/// whole slice of each pool.
#[must_use]
pub fn plan_lanes(
    plan: &Plan,
    client: &HostPoolSlice,
    server: &HostPoolSlice,
    limit: usize,
) -> LaneLayout {
    let whole = |note: Option<String>| LaneLayout {
        lanes: vec![LaneSpec {
            workers: plan.workers.iter().map(|w| w.worker_id).collect(),
            client: HostRange::new(client.offset, client.count),
            server: HostRange::new(server.offset, server.count),
        }],
        note,
    };
    let mut owners: Vec<Owner<'_>> = plan
        .workers
        .iter()
        .map(|worker| Owner {
            worker,
            client: range_in(worker, client),
            server: range_in(worker, server),
        })
        .filter(|o| o.client.is_some() || o.server.is_some())
        .collect();
    owners.sort_by_key(|o| o.worker.worker_id);

    let hosts = client.count.min(server.count);
    let want = owners
        .len()
        .min(limit)
        .min(usize::try_from(hosts).unwrap_or(usize::MAX));
    if want <= 1 {
        return whole(None);
    }
    if let Some(o) = owners
        .iter()
        .find(|o| !o.worker.rates.is_empty() && o.server.is_none())
    {
        return whole(Some(format!(
            "worker {} sends to the whole server pool {}, which one lane serves",
            o.worker.worker_id, server.name
        )));
    }

    // Consecutive workers share a lane, and the first groups take the
    // remainder.
    let (size, extra) = (owners.len() / want, owners.len() % want);
    let mut lanes = Vec::with_capacity(want);
    let mut rest = owners.as_slice();
    for k in 0..want {
        let n = size + usize::from(k < extra);
        let (group, tail) = rest.split_at(n);
        rest = tail;
        let (Some(c), Some(s)) = (
            hull(group.iter().filter_map(|o| o.client)),
            hull(group.iter().filter_map(|o| o.server)),
        ) else {
            return whole(Some(format!(
                "the workers of lane {k} own no hosts in one of the pools {} and {}",
                client.name, server.name
            )));
        };
        lanes.push(LaneSpec {
            workers: group.iter().map(|o| o.worker.worker_id).collect(),
            client: c,
            server: s,
        });
    }
    if overlap(lanes.iter().map(|l| l.client).collect())
        || overlap(lanes.iter().map(|l| l.server).collect())
    {
        return whole(Some(
            "the host ranges of the workers do not divide into lanes without overlap".into(),
        ));
    }
    LaneLayout { lanes, note: None }
}

/// The number of queues that a network interface has: the smaller of its
/// receive and transmit queue counts under `sys_class_net`, which is
/// `/sys/class/net` on a host. An interface that cannot be read counts one
/// queue, which is queue 0 that every interface has.
#[must_use]
pub fn queue_count(sys_class_net: &Path, interface: &str) -> usize {
    let count = |prefix: &str| {
        std::fs::read_dir(sys_class_net.join(interface).join("queues"))
            .map(|dir| {
                dir.filter_map(Result::ok)
                    .filter(|e| e.file_name().to_string_lossy().starts_with(prefix))
                    .count()
            })
            .unwrap_or(0)
    };
    count("rx-").min(count("tx-")).max(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan::{HostRangePlan, HostRole, WorkerRate};

    fn pool(name: &str, role: HostRole, offset: u32, count: u32) -> HostPoolSlice {
        HostPoolSlice {
            name: name.into(),
            role,
            offset,
            count,
            pool_count: offset + count,
            ..HostPoolSlice::default()
        }
    }

    fn worker(id: u32, c: Option<(u32, u32)>, s: Option<(u32, u32)>, rated: bool) -> WorkerPlan {
        let range = |pool: &str, r: Option<(u32, u32)>| {
            r.map(|(offset, count)| HostRangePlan {
                pool: pool.into(),
                offset,
                count,
            })
        };
        WorkerPlan {
            worker_id: id,
            hosts: [range("c", c), range("s", s)]
                .into_iter()
                .flatten()
                .collect(),
            rates: if rated {
                vec![WorkerRate::default()]
            } else {
                Vec::new()
            },
            ..WorkerPlan::default()
        }
    }

    fn layout(workers: Vec<WorkerPlan>, limit: usize) -> LaneLayout {
        let plan = Plan {
            workers,
            ..Plan::default()
        };
        plan_lanes(
            &plan,
            &pool("c", HostRole::Client, 10, 8),
            &pool("s", HostRole::Server, 100, 8),
            limit,
        )
    }

    fn four_workers() -> Vec<WorkerPlan> {
        (0..4)
            .map(|i| worker(i, Some((10 + 2 * i, 2)), Some((100 + 2 * i, 2)), true))
            .collect()
    }

    #[test]
    fn one_lane_per_worker_each_with_the_hosts_of_its_worker() {
        let l = layout(four_workers(), 4);
        assert_eq!(l.note, None);
        assert_eq!(l.lanes.len(), 4);
        for (k, lane) in l.lanes.iter().enumerate() {
            let k = u32::try_from(k).unwrap();
            assert_eq!(lane.workers, vec![k]);
            assert_eq!(lane.client, HostRange::new(10 + 2 * k, 2));
            assert_eq!(lane.server, HostRange::new(100 + 2 * k, 2));
        }
    }

    #[test]
    fn the_queues_of_the_ports_limit_the_lanes_and_workers_share_a_lane() {
        let l = layout(four_workers(), 3);
        assert_eq!(l.lanes.len(), 3);
        assert_eq!(
            l.lanes[0].workers,
            vec![0, 1],
            "the first lane takes the remainder"
        );
        assert_eq!(l.lanes[0].client, HostRange::new(10, 4));
        assert_eq!(l.lanes[0].server, HostRange::new(100, 4));
        assert_eq!(l.lanes[1].workers, vec![2]);
        assert_eq!(l.lanes[2].workers, vec![3]);

        let l = layout(four_workers(), 2);
        assert_eq!(l.lanes.len(), 2);
        assert_eq!(l.lanes[1].workers, vec![2, 3]);
        assert_eq!(l.lanes[1].client, HostRange::new(14, 4));
    }

    #[test]
    fn a_limit_of_one_or_one_worker_keeps_the_whole_pools_on_one_lane() {
        for l in [
            layout(four_workers(), 1),
            layout(four_workers()[..1].to_vec(), 4),
        ] {
            assert_eq!(l.note, None);
            assert_eq!(l.lanes.len(), 1);
            assert_eq!(l.lanes[0].client, HostRange::new(10, 8));
            assert_eq!(l.lanes[0].server, HostRange::new(100, 8));
        }
    }

    #[test]
    fn workers_are_ordered_by_id_and_a_worker_without_hosts_gets_no_lane() {
        let mut w = four_workers();
        w.reverse();
        w.push(worker(9, None, None, false));
        let l = layout(w, 8);
        assert_eq!(l.lanes.len(), 4);
        assert_eq!(l.lanes[0].workers, vec![0]);
        assert_eq!(l.lanes[3].workers, vec![3]);
    }

    #[test]
    fn a_server_only_worker_joins_a_lane_without_a_rate() {
        let w = vec![
            worker(0, Some((10, 4)), Some((100, 2)), true),
            worker(1, None, Some((102, 2)), false),
            worker(2, Some((14, 4)), Some((104, 4)), true),
        ];
        let l = layout(w, 2);
        assert_eq!(l.note, None);
        assert_eq!(l.lanes.len(), 2);
        assert_eq!(l.lanes[0].workers, vec![0, 1]);
        assert_eq!(l.lanes[0].server, HostRange::new(100, 4));
        assert_eq!(l.lanes[1].workers, vec![2]);
    }

    #[test]
    fn a_client_worker_without_a_server_range_keeps_one_lane_over_the_whole_pools() {
        let mut w = four_workers();
        w[2] = worker(2, Some((14, 2)), None, true);
        let l = layout(w, 4);
        assert_eq!(l.lanes.len(), 1);
        assert_eq!(l.lanes[0].server, HostRange::new(100, 8));
        let note = l.note.expect("a note");
        assert!(
            note.contains("worker 2") && note.contains("pool s"),
            "{note}"
        );
    }

    #[test]
    fn overlapping_ranges_keep_one_lane() {
        let mut w = four_workers();
        w[1] = worker(1, Some((11, 2)), Some((102, 2)), true);
        let l = layout(w, 4);
        assert_eq!(l.lanes.len(), 1);
        assert!(l.note.expect("a note").contains("overlap"));
    }

    #[test]
    fn a_lane_that_owns_no_server_host_keeps_one_lane() {
        let w = vec![
            worker(0, Some((10, 4)), None, false),
            worker(1, Some((14, 4)), Some((100, 8)), false),
        ];
        let l = layout(w, 2);
        assert_eq!(l.lanes.len(), 1);
        assert!(l.note.expect("a note").contains("lane 0"));
    }

    #[test]
    fn a_pool_with_fewer_hosts_than_lanes_limits_the_lanes() {
        let plan = Plan {
            workers: four_workers(),
            ..Plan::default()
        };
        let l = plan_lanes(
            &plan,
            &pool("c", HostRole::Client, 10, 8),
            &pool("s", HostRole::Server, 100, 1),
            4,
        );
        assert_eq!(l.lanes.len(), 1);
    }

    #[test]
    fn the_queue_count_is_the_smaller_of_receive_and_transmit_queues() {
        let dir = std::env::temp_dir().join(format!("fps-lanes-{}", std::process::id()));
        let queues = dir.join("dpa").join("queues");
        std::fs::create_dir_all(&queues).expect("make the sysfs tree");
        for q in ["rx-0", "rx-1", "rx-2", "rx-3", "tx-0", "tx-1", "tx-2"] {
            std::fs::create_dir_all(queues.join(q)).expect("make a queue");
        }
        assert_eq!(queue_count(&dir, "dpa"), 3);
        assert_eq!(
            queue_count(&dir, "missing"),
            1,
            "an unreadable interface has queue 0"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
