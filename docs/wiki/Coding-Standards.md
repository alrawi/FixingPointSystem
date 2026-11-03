# Coding Standards

These standards keep code from many agents consistent enough that any agent can read, review, and resume any package. Linters enforce most rules, and reviewers enforce the rest. A rule that a linter cannot check carries an example on this page.

## Rules for every language

We apply twelve rules in every workstream.

1. **License header.** Every source file starts with an SPDX line, for example `// SPDX-License-Identifier: GPL-3.0-only`.
2. **Formatter output is final.** Code is formatted by the language's standard formatter, and reviewers do not discuss formatting.
3. **No warnings.** Linters run with warnings as errors. A suppression names the rule and the reason on the same line.
4. **Untrusted input never panics.** Bytes from the device under test, from captures, from modules, and from API clients are untrusted. Parsing code returns errors and never indexes without a bounds check.
5. **Determinism.** Logic that affects traffic takes its randomness from a seeded generator and its time from an injected clock. Wall clock reads and unseeded randomness appear only at process edges.
6. **Errors carry context.** An error names the object and the operation, for example the flow id and action index, and wraps its cause.
7. **Structured logs.** Logs are JSON with stable field names. Messages are lowercase sentences without trailing punctuation, and variable data goes into fields.
8. **No hidden work items.** A `TODO` names an issue, as in `TODO(#123)`. A bare `TODO` fails review.
9. **Small public surface.** Every exported item has a documentation comment, and anything that does not need export stays private.
10. **No contract drift.** Code that implements a contract uses generated types or tests that read the contract files directly.
11. **Dependencies are deliberate.** A new dependency needs a GPL-3.0 compatible license, an active upstream, and a sentence in the pull request that explains why the standard library does not suffice.
12. **Names describe the domain.** We use the terms of the specification, such as scenario, flow, session, host pool, and plan, and we do not invent synonyms.

## Rust

The agent, the engine, backends, and native module code use Rust. The toolchain is current stable, 1.87 or newer, with edition 2021.

| Tool | Command | Setting |
|---|---|---|
| Formatter | `cargo fmt --all -- --check` | Default rustfmt |
| Linter | `cargo clippy --workspace --all-targets -- -D warnings` | Workspace lints below |
| Licenses and advisories | `cargo deny check` | `deny.toml` |
| Tests | `cargo test --workspace` | |

Each crate declares these lints in `lib.rs` or `main.rs`. Backend crates that need `unsafe` for AF_XDP or DPDK replace `forbid` with `deny` and justify every block.

```rust
// SPDX-License-Identifier: GPL-3.0-only
//! Flow program interpreter.
#![forbid(unsafe_code)]
#![warn(missing_docs, clippy::unwrap_used, clippy::expect_used, clippy::panic)]
```

Libraries define error enums with `thiserror`. Binaries and tests may use `anyhow`. Library code never calls `unwrap` or `expect`, except for an invariant that the type system cannot express, and then the message starts with `invariant`.

```rust
/// Errors that the interpreter reports for one session.
#[derive(Debug, thiserror::Error)]
pub enum InterpError {
    /// The peer closed the session before an `expect` finished.
    #[error("flow {flow} action {index}: peer closed after {received} bytes")]
    PeerClosed { flow: FlowId, index: usize, received: usize },
    /// An `expect` did not finish within its timeout.
    #[error("flow {flow} action {index}: expect timed out after {timeout_ms} ms")]
    ExpectTimeout { flow: FlowId, index: usize, timeout_ms: u64 },
    /// The transport failed.
    #[error("flow {flow} action {index}: io error")]
    Io { flow: FlowId, index: usize, #[source] source: std::io::Error },
}
```

Hot path code follows four rules. The hot path is every function that runs per session action, per packet, or per byte.

1. No allocation per action. Buffers come from per worker pools or reused vectors.
2. No locks and no shared mutable state across workers. Each worker owns its sessions and its statistics shard.
3. No blocking calls inside async code. File and DNS work outside the data plane runs on a dedicated thread.
4. No lock held across an `.await`.

```rust
// Good: the worker reuses one scratch buffer for every discard.
let n = io.read(&mut self.scratch[..want.min(self.scratch.len())]).await?;

// Bad: this allocates on every expect action.
let mut buf = vec![0u8; want];
let n = io.read(&mut buf).await?;
```

Randomness comes from a generator derived from the run seed and the session id, so a replay reproduces every session.

```rust
/// Returns the generator for one session. The same seed and id always give the same stream.
pub fn session_rng(run_seed: u64, session_id: u64) -> rand_pcg::Pcg64Mcg {
    rand_pcg::Pcg64Mcg::new(u128::from(run_seed) << 64 | u128::from(session_id))
}
```

Logging uses `tracing` with fields.

```rust
tracing::warn!(flow = %flow, action = index, timeout_ms, "expect timed out");
```

Every `unsafe` block carries a `SAFETY` comment that states the invariant and the reason it holds.

```rust
// SAFETY: `frame` lies inside the UMEM region registered in `Umem::new`, and the
// kernel returned it on the receive ring, so no other descriptor aliases it.
let bytes = unsafe { self.umem.frame_slice(frame) };
```

## Go

The controller and the command line client use Go, at the version in `controller/go.mod`.

| Tool | Command |
|---|---|
| Formatter | `gofmt -l .` prints nothing; `goimports` groups imports |
| Linters | `go vet ./...` and `golangci-lint run` with errcheck, staticcheck, revive, gosec, and errorlint |
| Tests | `go test -race ./...` |

Packages live under `internal/` unless another module imports them. A consumer defines the interface it needs. Functions take `context.Context` as the first argument when they do I/O or may block.

Errors wrap their cause with `%w` and use sentinel errors for conditions that callers test.

```go
// SPDX-License-Identifier: GPL-3.0-only

// ErrConfigNotFound reports that no stored configuration has the requested id.
var ErrConfigNotFound = errors.New("config not found")

func (s *Store) Get(ctx context.Context, id string) (Config, error) {
	row := s.db.QueryRowContext(ctx, `SELECT body FROM configs WHERE id = ?`, id)
	var body []byte
	if err := row.Scan(&body); err != nil {
		if errors.Is(err, sql.ErrNoRows) {
			return Config{}, fmt.Errorf("get config %s: %w", id, ErrConfigNotFound)
		}
		return Config{}, fmt.Errorf("get config %s: %w", id, err)
	}
	return decodeConfig(body)
}
```

HTTP handlers translate errors into RFC 9457 problem details in one place.

```go
func writeProblem(w http.ResponseWriter, err error) {
	p := api.Problem{Type: "about:blank", Title: "internal error", Status: http.StatusInternalServerError}
	switch {
	case errors.Is(err, ErrConfigNotFound):
		p.Title, p.Status = "config not found", http.StatusNotFound
	case errors.As(err, new(*ValidationError)):
		p.Title, p.Status = "invalid config", http.StatusUnprocessableEntity
	}
	w.Header().Set("Content-Type", "application/problem+json")
	w.WriteHeader(p.Status)
	_ = json.NewEncoder(w).Encode(p)
}
```

Logging uses `log/slog` with a JSON handler.

```go
logger.Info("run state changed", "run_id", run.ID, "from", from, "to", to)
```

The controller has no package level mutable state. Dependencies arrive through constructors, so tests replace them with fakes.

## Python

Tools, realism checks, the capture compiler, and the SDK use Python 3.10 or newer.

| Tool | Command |
|---|---|
| Formatter and linter | `ruff format --check` and `ruff check` |
| Types | `mypy --strict` for new packages |
| Tests | `python3 -m pytest` |

Command line tools expose `main(argv: list[str] | None = None) -> int` and return an exit code, so tests call them without a subprocess. Paths use `pathlib`. Unit tests never reach the network.

```python
# SPDX-License-Identifier: GPL-3.0-only
"""Score a capture against expected application labels."""
from __future__ import annotations

import argparse
from pathlib import Path


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--pcap", type=Path, required=True)
    parser.add_argument("--labels", type=Path, required=True)
    args = parser.parse_args(argv)
    report = score(args.pcap, args.labels)
    print(report.to_json())
    return 0 if report.passed else 1
```

Python code that reads captures or other downloaded files runs with `python3 -I` in CI, so files next to the input cannot shadow imports.

## TypeScript

The web UI stack is pending decision UI-00. Whatever the decision, we apply four rules. The compiler runs in strict mode. A formatter and a linter run in CI with warnings as errors. Components call the API only through the client generated from the OpenAPI contract. Tests run against a mock server built from the contract examples.

## Shell

Shell scripts use bash and pass shellcheck. Each script starts with the same skeleton.

```bash
#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-only
# Create the two arm namespace harness. Usage: netns.sh up|down|exec NS CMD...
set -euo pipefail

usage() { echo "usage: $0 up|down|exec NS CMD..." >&2; exit 2; }

main() {
  [[ $# -ge 1 ]] || usage
  case "$1" in
    up) up ;;
    down) down ;;
    exec) shift; exec_in "$@" ;;
    *) usage ;;
  esac
}

main "$@"
```

Scripts quote every expansion, use functions for each step, and never call `sudo` internally. A script that needs root checks `EUID` and exits with a message, and the caller decides how to elevate.

## Contracts and YAML

Contract files follow five rules, and the validator enforces the first three.

1. YAML uses two space indentation and plain integers for numbers. We write `10000000000`, because some parsers read `10e9` as a string.
2. Strings that contain `: `, `#`, braces, or a leading special character are quoted.
3. Every example under `contracts/examples/` validates.
4. Each contract file carries its version, and every change adds a file `contracts/changes/<ID>.md` that the coordinator folds into `contracts/CHANGELOG.md` when a version ships.
5. A field is never removed or renamed in a minor version. Deprecation lasts two minor releases.

## Modules

A module directory has the same layout for every protocol.

```text
modules/dns/
  manifest.yaml          name, version, type, license, provides, parameters, provenance
  scenarios/*.yaml       flow programs
  native/                optional Rust crate that implements the extension traits (D10, proposed)
  fixtures/              synthetic reference captures and expected labels
  tests/                 interpreter and transcript tests
  README.md              parameters, scenarios, realism results
```

Module content is synthetic or carries a documented license in the provenance field. Fixtures come from runs in the test harness and never from third party captures.

## Commits and pull requests

Commit subjects use the imperative mood, stay under 72 characters, and name the package. Bodies explain why the change exists. Every commit carries two trailers.

```text
Add expect min_bytes to the interpreter

The DNS module needs to read a header before it knows the message length.

Work-Package: ENG-04
Agent: claude-7f3a2c
```

A pull request contains one work package, uses the title `[<ID>] <title>`, and fills in the template in `.github/pull_request_template.md`. The owner squashes checkpoint commits into one commit at merge.

## Documentation

Pages in this wiki, READMEs, and decision records use one voice. We write in the first person plural, in the active voice, and without contractions. We lead with the result or decision. We do not use em dashes, colons in running prose, or exclamation marks, and we introduce every list with a complete sentence. Tables and code blocks may contain any punctuation.
