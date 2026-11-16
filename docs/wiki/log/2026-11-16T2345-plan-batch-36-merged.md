## [2026-11-16] plan | Batch 36, the first batch under D13

Batch 36 is the first batch that the coordinator ran under [D13](../../decisions/0013-autonomous-coordination.md). The vetting agent recommended the batch, and the owner's standing instruction to fix found bugs added RLM-24. The coordinator merged five pull requests, each after green CI and the Jev gate:

- OPS-08 in PR #580: `tools/swarm/check_self_hosted.py`.
  - The check finds one workflow on `main`. `qa-bench.yml` holds the self hosted nightly job and also starts on push and pull request.
  - The split it asks for touches a file that the owner asked on 2026-11-10 to keep untouched, so it waits for the owner. [Operations](../Operations.md#self-hosted-runner-check) says so.
- CON-33 in PR #581 fixes bug #482. Before merge the coordinator updated two test counts outside the package paths, because the store and UI tests count every example test.
- RLM-24 in PR #583: a reused TCP client port starts a new session. Before merge the coordinator applied its two "Watch out for" entries to [Module Development](../Module-Development.md).
- CTL-16 in PR #582: multi agent runs. It found that the controller has no production source of clock offsets, so the coordinator added CTL-51.
- TRN-11 in PR #585: AF_XDP packet input and output. The coordinator added the `afxdp` job to `trn.yml`, with `FPS_REQUIRE_AFXDP=1`, and the [Testing](../Testing.md) and [Implementation Plan](../Implementation-Plan.md) rows from its notes.

Jev scored every merged diff below 0.2 on the secret, self hosted runner, and protected host rules, and below 0.5 on the others.

Two packages are still open:

- UI-07 in PR #584.
  - The coordinator added the `controller-browser` job to `ui.yml`. Its first run passed 10 of 14 browser tests against the real controller, and a worker is fixing the other four.
  - Jev scored the diff 0.47 on the secret rule, for the fixed tokens of the test controller, so under D13 the vetting agent reviews it before merge.
- TRN-17 in PR #586 waits for the owner. `cargo deny` rejects `smoltcp` and `managed`, which are both licensed `0BSD`, and D13 keeps any loosening of `deny.toml` with the owner. Its measurements are useful already: smoltcp reaches 1,000,584 sockets at 1.59 KB each, and its idle poll costs from 15 ns per socket at 16,016 sockets to 52 ns at 1,000,584.

The owner's relay ran its first jobs on fat-nuc on 2026-11-16:

- It rejected an inline `python3 -c` job, as it should.
- It passed a checkout with `tools/plan/plan.py check`.
- It found that its SSH account has no passwordless sudo and no `cargo` on the non interactive `PATH`, so host tests that build Rust or need root wait for the owner.

### Shared page notes

None.
