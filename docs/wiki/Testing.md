# Testing

Every work package proves its behavior with tests that run without the packages around it. This page defines the test levels, where each level lives, how to write a modular test for a new component, and how regression tests protect fixed defects.

## Test levels

We use eight levels. Each package ships the levels that its workstream requires in the matrix below.

| Level | Purpose | Location | Runs in |
|---|---|---|---|
| Unit | One function or type, no I/O | Next to the code (`#[cfg(test)]`, `_test.go`, `test_*.py`) | Every pull request |
| Property | Invariants over generated inputs | Next to the code, with `proptest` or `hypothesis` | Every pull request |
| Contract | Code agrees with a contract file | `tests/` of the consuming package | Every pull request |
| Component | One package through its public interface with fakes for neighbors | `tests/` of the package | Every pull request |
| Integration | Several real packages in one process or host | `tests/integration/` | Every pull request when the runner allows it |
| End to end | Controller, agent, and harness over real sockets | `tests/e2e/` and `tests/gates/` | Pull requests labeled `e2e`, nightly, and gates |
| Realism | Generated traffic passes the eight realism checks | Module `fixtures/` and `realism/` | Module pull requests and nightly |
| Performance | Throughput, rate, and latency against baselines | `bench/` | Nightly on a fixed host and on request |

## Required levels by workstream

| Workstream | Unit | Property | Contract | Component | Integration | End to end | Realism | Performance |
|---|---|---|---|---|---|---|---|---|
| CON | | | yes | | | | | |
| OPS, QA | yes | | | yes | | | | |
| ENG | yes | yes | yes | yes | | | | yes |
| TRN, HST | yes | | | yes | yes | | | yes |
| AGT | yes | | yes | yes | yes | | | |
| CTL | yes | | yes | yes | yes | | | |
| CLI | yes | | yes | yes | | | | |
| MOD | yes | | yes | yes | | | yes | |
| RLM, CAP | yes | yes | | yes | | | | |
| IMG | | | | yes | | yes | | |
| UI | yes | | yes | yes | | yes | | |

## Commands

The root [`Makefile`](https://github.com/alrawi/FixingPointSystem/blob/main/Makefile) runs every check with one command and each workstream's checks with another. We run `make lint test` before every pull request.

```bash
make lint                 # every formatter and linter
make test                 # every fast level of every workstream
make test-con             # one workstream, here the contracts
make lint-ops test-ops    # the linters and tests of one workstream
make help                 # the registered targets

# Equivalent commands per language
cargo test -p fps-interp
(cd controller && go test -race ./...)
python3 -m pytest realism
```

The targets that exist today are listed below.

| Target | Runs |
|---|---|
| `lint-rust` | `cargo fmt --check` and `cargo clippy -D warnings` over the whole Cargo workspace |
| `lint-shell` | shellcheck on every tracked `*.sh` file |
| `lint-deny` | `cargo deny check`, skipped with a notice when cargo-deny is missing |
| `lint-ops` | `tools/plan/plan.py check`, `render --check`, and `lint` |
| `lint-ctl` | `gofmt -l`, `go vet`, and `golangci-lint run` over the controller, skipping golangci-lint with a notice when it is missing |
| `test-con` | the contract validator and its tests |
| `test-ctl` | `go test -race ./...` over the controller |
| `test-ops` | the plan and wiki tool tests and the Cargo workspace probe |
| `test-spk` | the Rust spike crates and the S4 translator tests |
| `lint-qa` | gofmt, go vet, and golangci-lint on the Go golden file helper, and ruff and `mypy --strict` on the Python one, each skipped with a notice when its tool is missing |
| `test-qa` | the match, mismatch, and update tests of the three [golden file helpers](#golden-files) |
| `lint-qa` | shellcheck on the harness scripts, and ruff and mypy on its Python helper when they are installed |
| `test-qa` | the unit tests of the harness pcapng helper |
| `e2e-qa` | the namespace harness self test, which needs root |
| `test-agt` | `cargo test -p fps-agent`, the capability detection tests against fake system roots and the `agent.proto` mirror test |
| `test-eng` | every crate under `engine/`, see [Engine](Engine.md) |
| `lint-rlm` | `ruff format --check`, `ruff check`, and `mypy --strict` over `realism/` |
| `test-rlm` | the realism runner tests, including the ndpiReader test when nDPI is installed |

The root `Makefile` never changes. It includes every file under `mk/`, and each workstream area owns one of them, for example `mk/ctl.mk` or `mk/qa-harness.mk`. A file appends its targets to `LINT_TARGETS` and `TEST_TARGETS` and delegates to the workstream's own commands, so parallel packages never edit the same file. The Rust workspace takes its members from the globs `spikes/*`, `engine/*`, and `modules/*/native`, and the crate `modules/_template/native` keeps the module glob matching, so a new crate needs no edit to the root `Cargo.toml`. The `test-ops-workspace` target proves this on every run. Every Go module builds on its own because the `Makefile` and the workflows set `GOWORK=off`, so a new module needs no edit to `go.work`. End to end targets append to `E2E_TARGETS` in the same way, and `make e2e` runs them. These targets need root, so `make test` leaves them out and an agent runs `sudo make e2e`.

```make
# CTL controller
LINT_TARGETS += lint-ctl
lint-ctl:
	cd controller && go vet ./... && golangci-lint run

TEST_TARGETS += test-ctl
test-ctl:
	cd controller && go test -race ./...
```


Fast levels finish within five minutes on a CI runner. A test that needs longer moves to the end to end or performance level.

## Writing a modular test

A modular test checks one package through its public interface and replaces every neighbor with a fake that the neighbor's own package publishes. We follow five steps.

1. **Name the seam.** Find the trait, interface, or contract file that sits between the package and its neighbor in the seam table of the [Implementation Plan](Implementation-Plan.md).
2. **Use the published fake.** Each seam owner ships a fake next to the interface, for example the in memory duplex in `engine/fps-io` and the fake agent in `controller/internal/fakeagent`. Do not write a private fake when a published one exists.
3. **Fix the inputs.** Use a fixed seed, an injected clock, and fixture files under `testdata/`.
4. **Assert observable behavior.** Check outputs, counters, bytes on the wire, and error variants. Do not assert private fields.
5. **Name the test after the behavior.** For example, `expect_pattern_consumes_through_first_match`.

The Rust example tests the interpreter against a scripted server over the in memory duplex.

```rust
#[tokio::test]
async fn expect_pattern_consumes_through_first_match() {
    let (client_io, mut server_io) = fps_io::memory::duplex(64 * 1024);
    let program = compile_client(r#"
        - send: "GET / HTTP/1.1\r\n\r\n"
        - expect: { pattern: "\r\n\r\n", timeout_ms: 1000 }
    "#);
    let server = tokio::spawn(async move {
        read_until(&mut server_io, b"\r\n\r\n").await;
        server_io.write_all(b"HTTP/1.1 200 OK\r\n\r\nextra").await.unwrap();
    });
    let outcome = run_session(&program, client_io, test_context(42)).await;
    assert_eq!(outcome.result, Ok(()));
    assert_eq!(outcome.consumed, 19); // through the first "\r\n\r\n", not "extra"
    server.await.unwrap();
}
```

The Go example tests the orchestrator against fake agents with a table of state transitions.

```go
func TestRunStateTransitions(t *testing.T) {
	tests := []struct {
		name   string
		agents []fakeagent.Behavior
		want   run.State
	}{
		{"all agents finish", []fakeagent.Behavior{fakeagent.Finish, fakeagent.Finish}, run.Completed},
		{"one agent crashes", []fakeagent.Behavior{fakeagent.Finish, fakeagent.CrashAfter(2)}, run.Failed},
	}
	for _, tc := range tests {
		t.Run(tc.name, func(t *testing.T) {
			o := newTestOrchestrator(t, tc.agents...)
			id := o.Start(context.Background(), testConfig(t))
			if got := o.Wait(id); got != tc.want {
				t.Fatalf("state = %v, want %v", got, tc.want)
			}
		})
	}
}
```

## Testing a new module

A module package proves four things with tests inside its own directory.

1. **The module validates.** `python3 tools/contracts/validate.py --modules modules/<name>` passes.
2. **Client and server agree.** An interpreter test runs every scenario with both sides over the in memory duplex and checks success counters.
3. **Bytes stay stable.** A transcript test renders each scenario with seed 42 and compares the bytes in both directions with golden files in `tests/testdata/`.
4. **Traffic looks real.** The fixture capture from a harness run passes every realism check that applies to the module, and `fixtures/expected.yaml` records the expected labels.

```text
modules/dns/tests/
  scenarios.rs                   client and server over the in memory duplex
  transcript.rs                  golden byte transcripts
  testdata/lookup.seed42.c2s.bin
  testdata/lookup.seed42.s2c.bin
modules/dns/fixtures/
  lookup.pcapng                  produced by tests/harness, synthetic only
  expected.yaml                  application label and realism thresholds
```

## Regression tests

A regression test pins a defect that once escaped. We apply five rules.

1. **Test first.** The fix pull request adds a test that fails on the old code. The pull request description shows the failing output.
2. **Name the issue.** The test name or a comment cites the issue number, as in `regression_142_chunked_last_byte`.
3. **Keep it at the lowest level.** A defect found in an end to end run gets a unit or component test that reproduces it, when such a test can reproduce it.
4. **Never delete without a reason.** Removing a regression test requires a sentence in the pull request that explains why the behavior no longer applies.
5. **Guard performance too.** A fixed performance defect adds or tightens a baseline under `bench/baselines/`.

Golden files form the second regression layer. Compiler output, plan output, report output, and module transcripts are compared with files under `testdata/`. A change to a golden file must be intended.

```bash
UPDATE_GOLDEN=1 cargo test -p fps-flow   # rewrite golden files
git diff --stat -- '*testdata*'          # review every changed file
```

A pull request that changes golden files explains each change in its description, and the reviewer checks the diff. The [golden file helpers](#golden-files) give every language the same switch and the same failure message.

Benchmark baselines form the third layer. `bench/compare.py` fails a run that regresses beyond the tolerance of each metric, which is 10% by default. Baselines change only in a pull request that names the host and explains the change.

## Golden files

We compare rendered output with golden files through one helper per language under `tests/golden/`. The three helpers follow four rules.

1. **One switch.** A helper rewrites a golden file only when `UPDATE_GOLDEN` equals `1`. Any other value, including an empty one, compares and never writes.
2. **Write only on change.** In update mode a helper writes a file only when its content differs, and it creates missing directories. An unchanged file keeps its modification time, so `git status` shows only real changes.
3. **Fail with a readable diff.** A mismatch fails the test with the path of the golden file, the hint to rerun with `UPDATE_GOLDEN=1`, and a unified diff from the golden file to the output. Content that is not UTF-8 gets the byte lengths, the first differing offset, and 16 bytes of hex from each side.
4. **A missing file fails.** Compare mode reports a missing golden file as a failure, so a test cannot pass before someone records its output.

The Rust crate `fps-golden` is a workspace member. A crate adds it as a development dependency and calls the macro, which resolves the name under the crate's `tests/testdata/`.

```toml
[dev-dependencies]
fps-golden = { path = "../../tests/golden/rust" }
```

```rust
#[test]
fn plan_renders_stable_json() {
    let json = render_plan(&test_plan(42));
    fps_golden::assert_golden!("plan.seed42.json", json);
}
```

The function `fps_golden::assert_golden(path, output)` takes any path, and `fps_golden::check(path, output, mode)` returns a `Result` for tests that inspect the failure.

A Go module that imports `github.com/alrawi/FixingPointSystem/tests/golden/go` adds a `replace` line that points at `../tests/golden/go`, because builds run with `GOWORK=off`. `golden.Assert` resolves the name under the package's `testdata/`, which is the working directory of `go test`.

```go
import golden "github.com/alrawi/FixingPointSystem/tests/golden/go"

func TestPlanRendersStableJSON(t *testing.T) {
	golden.Assert(t, "plan.seed42.json", renderPlan(testPlan(42)))
}
```

`golden.AssertString` takes a string, `golden.AssertFile` takes any path, and `golden.Check` returns errors that match `golden.ErrMismatch` and `golden.ErrMissing` with `errors.Is`.

The Python helper is the `golden` fixture in `tests/golden/python/fps_golden.py`. A test directory imports the fixture in its `conftest.py`, because pytest rejects `pytest_plugins` in a conftest below the rootdir. The fixture resolves the name under `testdata/` next to the test file and accepts `str` or `bytes`.

```python
# realism/conftest.py, one level below the repository root
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "tests" / "golden" / "python"))
from fps_golden import golden

__all__ = ["golden"]
```

```python
def test_plan_renders_stable_json(golden):
    golden.check("plan.seed42.json", render_plan(test_plan(42)))
```

We record or refresh golden files with the switch and then review every changed file before we commit.

```bash
UPDATE_GOLDEN=1 cargo test -p fps-flow
UPDATE_GOLDEN=1 go test ./...
UPDATE_GOLDEN=1 python3 -m pytest realism
git diff --stat -- '*testdata*'
```

## Determinism and flaky tests

Tests use fixed seeds, injected clocks, and loopback or namespace networks. A test that depends on real time uses generous limits and asserts ranges.

A flaky test is a defect. The agent that observes it opens a `type:bug` issue with the label `flaky`, marks the test as ignored with a reference to the issue, and continues. The quarantine lasts at most one week before the test is fixed or removed with the owner's approval.

## Coverage

We measure line coverage on changed packages with `cargo llvm-cov`, `go test -cover`, and `pytest --cov`. New library code should reach 80% line coverage. Coverage is a review signal, and a reviewer may accept less when the uncovered code is error handling that tests cannot reach without fault injection.

## Test data rules

Test data is synthetic, small, and licensed with the repository. Captures that contain personal data, credentials, or third party content never enter the repository. Fixture captures under `modules/*/fixtures/` stay below 1 MB each.

## CI workflows

Each workstream owns one workflow file, `.github/workflows/<workstream>.yml`, so packages add CI jobs without editing a shared file. The file [`ops.yml`](https://github.com/alrawi/FixingPointSystem/blob/main/.github/workflows/ops.yml) is the first instance and the template for the others. The repository wide jobs for contracts, Rust, licenses, and shell scripts stay in `ci.yml`.

A workstream workflow follows six rules.

1. **Name it after the workstream.** The workflow `name` is the workstream, as in `ctl`, and the job id names the level, as in `test`. The file usually carries the same name, as `ops.yml` does. The controller workflow is `controller.yml` at the owner's request.
2. **Run the make targets.** The job installs its toolchains and then runs `make lint-<ws> test-<ws>`, so CI and agents run the same commands.
3. **Filter by paths.** The `push` and `pull_request` triggers list the workstream's catalog paths and the area's own `mk/` file and workflow file, and `Cargo.toml` and `Cargo.lock` for Rust areas. The workflow file is named after the area, for example `qa-golden.yml` or `qa-harness.yml`, so two packages never create the same file.
4. **Pin the toolchain.** Go and Python versions are explicit, and Rust uses the stable toolchain as in `ci.yml`.
5. **Grant least privilege.** The workflow sets `permissions` to `contents: read` unless a job must write.
6. **Keep slow levels apart.** End to end, realism, and performance jobs live in their own workflow files with the triggers that the levels table names, so the fast workflow stays within five minutes.

A path filtered workflow does not run on pull requests that miss its paths, and GitHub then reports a required check as pending. The owner therefore marks only `ci.yml` jobs as required checks, or marks a workstream job as required only after removing its path filter.

## Namespace harness

End to end tests run on the two arm namespace harness in [`tests/harness/`](https://github.com/alrawi/FixingPointSystem/tree/main/tests/harness), which QA-01 built. The harness connects a client namespace and a server namespace through a routing namespace that stands in for the device under test, so real sockets cross a forwarding hop on any Linux runner without virtual machines. The routing namespace can apply an MTU, an 802.1Q tag on each arm, and NAT, and it records pcapng captures on either arm.

```bash
sudo tests/harness/netns.sh up                       # FPS_MTU, FPS_NAT, FPS_CLIENT_VLAN, and more configure it
sudo tests/harness/netns.sh capture start out.pcapng client
sudo tests/harness/netns.sh exec client curl "http://$(tests/harness/netns.sh info | sed -n 's/^SERVER_ADDR=//p'):8080/"
sudo tests/harness/netns.sh capture stop
sudo tests/harness/netns.sh down                     # removes every namespace, link, and route
```

A test reads addresses from `netns.sh info` and sets `FPS_NETNS` to a name of its own, so several harnesses run side by side. The harness turns off IPv6 and the veth segmentation offloads by default, so captures hold only test traffic in frames no larger than the MTU. The [harness README](https://github.com/alrawi/FixingPointSystem/blob/main/tests/harness/README.md) lists every command and setting. The `harness` job of `.github/workflows/qa-harness.yml` runs the self test as root on `ubuntu-24.04`, including the VLAN case that needs the `8021q` kernel module.

## Gate suites

Each gate suite lives under `tests/gates/gate<N>/` and has one entry point, `run.sh`, which writes `report.md` with measured values and a pass or fail verdict per criterion. A gate suite uses only public interfaces, which are the REST API, the agent binary, the harness, and the realism runner. The owner signs off a gate after reading its report.
