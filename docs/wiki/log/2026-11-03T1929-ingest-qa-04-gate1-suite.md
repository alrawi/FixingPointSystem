## [2026-11-03] ingest | QA-04 Gate 1 acceptance suite

We added the Gate 1 acceptance suite under `tests/gates/gate1/` and the new component page [Gates](Gates.md). The script `run.sh` brings up the QA-01 harness, starts the real `fps-controller` in the client namespace, and starts two identities of the real agent with `fps-agent connect`, one with `--side client` in the client namespace and one with `--side server` in the server namespace, which dials over a management link that the capture does not see. It installs `modules/http1` and `modules/dns`, submits the mix of `single_fetch`, `keepalive_browse`, and `lookup_a` through the REST API, records the router's client arm, fetches the run report and the metrics, scores the capture with nDPI, tshark, and Zeek, and writes `report.md` with the measured values. `prove-threshold.sh` scores a passing capture again with a threshold above the measured ratios and expects the gate to fail. A local run measured 100% conformance for 1,382 `http1` and 982 `dns` sessions, and the capture held every session that the agent attempted. The new files `mk/qa-gate1.mk` and `.github/workflows/qa-gate1.yml` run the lint, the unit tests, and the suite as root on `ubuntu-24.04`. The package added no Rust dependency. The evidence is `docs/handoff/QA-04.md` on the branch `wp/QA-04`.

## Shared page notes

The coordinator may apply the notes below.

1. [Home](Home.md) and the sidebar may link the new page [Gates](Gates.md).
2. [Testing](Testing.md) may add to its section Gate suites that the Gate 1 suite runs with `make build-qa-gate1` and `sudo make e2e-qa-gate1`, and that a suite writes `summary.json` next to `report.md`.
3. [Implementation Plan](Implementation-Plan.md) may mark QA-04 as done after the merge, which closes the critical path of Phase 1.
4. A CTL follow up may give run ids a prefix of their own, because `runs.Config.NewID` defaults to `store.RandomID`, so run ids start with `cfg_` like configuration ids.
5. A follow up may let one agent place the client side and the server side of a plan in two namespaces, or let the compiler put the two sides on different agents, so a gate needs one agent identity instead of two.
