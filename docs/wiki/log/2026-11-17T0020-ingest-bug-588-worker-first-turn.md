## [2026-11-17] ingest | bug-588 A worker takes one turn before it honors stop

We fixed bug #588, the intermittent failure of `worker::tests::an_error_from_a_loop_ends_only_that_worker` in `engine/fps-backend-afxdp`. The cause was in the worker code: `run` checked the stop flag before the first turn, so a stop that came before the thread was first scheduled ended the worker with no turn and no error, and a queue that was broken from the start was reported as stopped. `run` now takes at least one turn before it honors stop. The new test `a_loop_takes_one_turn_even_when_the_stop_came_first` runs `run` with the flag already set and fails on the old code every time, and the 20 ms sleep of `spinning_workers_stop_promptly_and_count_idle_turns` is gone. Before the fix the test failed 69 of 2000 runs, after it 0 of 2000 in plain, pinned, and loaded runs. The evidence is `docs/handoff/bug-588.md` on the branch `bug/588`. The fix changes no contract and adds no dependency.

### Shared page notes

[Transport](../Transport.md), section Workers, after "A loop that returns an error ends only its own worker.": add "Every worker takes at least one turn, even when `stop` comes before its thread starts, so the error of a broken queue is always reported."

Module-Development "Watch out for": none, the change is not visible to module authors.
