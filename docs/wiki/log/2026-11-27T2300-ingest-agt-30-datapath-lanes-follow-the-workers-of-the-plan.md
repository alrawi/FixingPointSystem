## [2026-11-27] ingest | AGT-30 Datapath lanes follow the workers of the plan

AGT-30 (issue #680) starts one datapath lane per plan worker, up to the queue count of each port, where the agent started one lane on queue 0 whatever the plan asked for. A forced AF_XDP run with four workers on a four queue port now uses four cores. The Agent page describes the rule in the section Lanes.

Decisions.

- `AfXdpSettings::lanes` is the most lanes, and 0 (the default) lets the plan decide. The queue count comes from `/sys/class/net/<interface>/queues`, the smaller of rx and tx, and the smaller of the two ports.
- A lane takes the smallest range of each pool that holds the ranges of its workers. Workers are grouped in the order of their ids. A plan that does not divide cleanly (a worker with a rate and no server range, a lane without hosts in a pool, overlapping ranges) runs on one lane over the whole pools, as before.
- The lane layout is logged and available from `AfXdpBackend::lane_layout`. No protocol field carries it, so no contract changed.

Findings.

- A veth pair made with `numrxqueues 4 numtxqueues 4` binds four native copy mode AF_XDP queues on each end, and a frame sent from queue `k` arrives on queue `k` of the peer, so a lane keeps its sessions.
- The workers of a plan with fewer server hosts than workers cannot get a lane each, because a lane needs a server host.

### Shared page notes

- Agent page: done in this package (section Lanes).
- Implementation Plan and Testing: the veth test `four_workers_on_a_veth_pair_with_four_queues_run_on_four_lanes` needs root and `ip`, as the other AF_XDP tests of `engine/fps-agent`.
- Module Development: no change.
