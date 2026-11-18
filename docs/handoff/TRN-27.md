# Handoff TRN-27 The user space datapath on AF_XDP queues

<!-- Copy this file to docs/handoff/<ID>.md when you claim a work package.
     Update it at every checkpoint. A successor reads only this file, the
     issue, and the branch, so write for an agent that knows nothing else. -->

| Field | Value |
|---|---|
| Issue | #628 |
| Branch | `wp/TRN-27` |
| Pull request | #650 |
| Status | in progress |
| Progress | 90% (4 of 5 plan steps) |
| Current holder | claude-7c27a1, lease until 2026-11-18T09:08:48Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

The production datapath: a crate `engine/fps-datapath` with the loop per queue that splits ARP, ND, DHCP, TCP, and UDP frames between fps-hosts and the user space stacks, and offers connect and listen as fps-backend-kernel does. Acceptance: a test over a veth pair in CI completes sessions through the datapath on AF_XDP queues, and the results are committed for the CI runner.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [x] 1. Crate `fps-datapath`: classifier, `Splitter` (FrameIo wrapper: ARP to the fps-hosts Responder, TCP to the stack, hooks for ND/DHCP/UDP), `Datapath` with lanes, `Connector`, `Listener`.
- [x] 2. In-memory tests (`engine/fps-datapath/tests/memory.rs`), no root.
- [x] 3. Veth test on AF_XDP queues (`engine/fps-backend-afxdp/tests/datapath_veth.rs`), sessions per second and sockets held.
- [x] 4. Results of the CI runner and the sandbox under `engine/fps-datapath/results/`, Transport.md section, log entry.
- [ ] 5. CI green on the final commit, review.

## Done and verified

`cargo test -p fps-datapath -p fps-backend-afxdp` with FPS_REQUIRE_AFXDP=1 as root: all pass, the veth test ran on AF_XDP (copy mode) with 1000 held sessions and 4000 short sessions.

## Remaining

CI green on the final commit, then `fps_swarm.py review 628 --pr 650`.

## Next action

Check the rust job of PR 650, then run the review command.

## Blockers and open questions

none

## Decisions taken in this package

- OwnStack owns both links of its pair, so a lane is the client arm queue and the server arm queue; one thread per lane polls `OwnStack::pump` (drive() would wait for a timer and not see frames from the wire).
- The splitter wraps FrameIo, so fps-tcp-own is untouched.
- The veth test lives in fps-backend-afxdp (dev-dependency cycle is allowed) so the existing afxdp job runs it.

## Pitfalls

Record anything that cost time and that a successor should avoid.

## Verify the current state

```bash
# Commands that a successor runs first, with the expected result
```

## Session log

Append one row per session start, checkpoint, release, block, takeover, and finish. Never edit earlier rows.

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|

## Results

Hosted runner (afxdp job, debug): 6,908 sessions per second, 4,001 sockets held. Sandbox: see engine/fps-datapath/results/.
