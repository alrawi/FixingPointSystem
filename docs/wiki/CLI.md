# CLI

The command `fps` is the Go command line client of the controller REST API in `contracts/openapi/fps-api-v1.yaml`. CLI-02 delivered it in [`cli/`](https://github.com/alrawi/FixingPointSystem/tree/main/cli), a Go module `github.com/alrawi/FixingPointSystem/cli` that uses only the standard library (`flag`, `net/http`, `encoding/json`, `text/tabwriter`). The Python client is a separate package, described in [SDK](SDK.md).

```bash
cd cli && go build -o fps ./cmd/fps
export FPS_URL=https://controller.example:8443 FPS_TOKEN_FILE=$HOME/.config/fps/token FPS_CA_FILE=ca.pem
fps config upload smoke.yaml          # prints the configuration id
fps run start cfg_1 --agent a1 --watch
fps report run_1 --format html -o report.html
```

## Layout

| Path | Purpose |
|---|---|
| `cmd/fps/` | The binary, which handles SIGINT and SIGTERM and calls `fpscli.Execute` |
| `internal/fpscli/` | The commands, the HTTP client, the output, and their tests against a stub server |
| `e2e/` | End to end tests against a real controller with fake agents |
| `.golangci.yml` | The linter configuration of the controller, unchanged |

## Commands

| Command | API operation | Output |
|---|---|---|
| `fps config upload FILE\|-` | `createConfig` | The configuration id on standard output |
| `fps config validate [--id ID] [FILE\|-]` | `createConfig`, then `validateConfig` | The report with its findings. The API has no dry run, so a file is stored first |
| `fps config show ID` | `getConfig` | The stored configuration as JSON |
| `fps config list` | none | The configurations that this machine uploaded, see below |
| `fps run start CONFIG_ID [--agent ID]... [--label K=V]... [--watch]` | `startRun` | The run id. With `--watch` the command follows the run and exits by its outcome |
| `fps run list [--state S] [--page-size N] [--page-token T] [--all]` | `listRuns` | One line per run, newest first |
| `fps run show ID` | `getRun` | State, reason, totals, and the criteria of the verdict |
| `fps run stop ID [--force]` | `stopRun` | The new state |
| `fps watch RUN_ID` | `getRun`, `streamRunMetrics` or `getRunMetrics` | Live metrics, then the final state and verdict |
| `fps report RUN_ID [--format text\|json\|csv\|html] [-o FILE]` | `getRunReport` | `text` is a summary that the client renders from the JSON report |

Flags and arguments mix freely, so `fps run stop r1 --force` and `fps run stop --force r1` both work. `fps help` prints the usage, and `fps <command> -h` prints the flags of one command.

The contract has no operation that lists configurations, so `fps config list` shows the uploads of this machine, from `$XDG_STATE_HOME/fps/configs.json` or `~/.local/state/fps/configs.json` (mode 0600, no secret). A configuration stored by someone else does not appear. A contract change that adds `GET /configs` would let the command ask the controller. The controller serves `listRuns` since CTL-35, so `fps run list` exits with 0 and the end to end test checks that it shows the runs that it started. Against a controller that still answers 501 the command exits with 5 and says that the controller does not implement the operation.

## Global flags and environment

Each global flag works before and after the command name.

| Flag | Environment | Meaning |
|---|---|---|
| `--url URL` | `FPS_URL` | Controller base URL, default `https://localhost:8443`. The client adds `/v1` |
| `--token-file FILE` | `FPS_TOKEN_FILE` | File that holds the API token. The client warns when other users can read it |
| none | `FPS_TOKEN` | The token itself, for a CI secret |
| `--ca-file FILE` | `FPS_CA_FILE` | PEM CA certificates that the client trusts for HTTPS, added to the system roots |
| `--json` | `FPS_JSON` | Print the answer of the controller as JSON |
| `--timeout D` | none | Timeout of one request, default 30s. It does not apply to a metrics stream |
| `--insecure-http` | `FPS_INSECURE_HTTP` | Allow the token over plain `http` to a host that is not loopback |
| `--history-file FILE` | `FPS_HISTORY_FILE` | The list of uploaded configurations |

The token comes from `--token-file`, then `FPS_TOKEN_FILE`, then `FPS_TOKEN`. There is deliberately no `--token` flag, because a command line shows in the process list. The client sends it as `Authorization: Bearer <token>`, as the contract's `bearerToken` scheme says. No output, error, or log line shows it, and the client replaces it with `[redacted]` in any text that a server sends back. The client never follows a redirect, so the token reaches the configured host only. It refuses to send the token over `http` to a host other than loopback unless `--insecure-http` is set.

## Output

Without `--json`, results are text: tables for lists, `Key: value` blocks for one object. Data goes to standard output and messages such as "stored configuration" go to standard error, so `id=$(fps config upload smoke.yaml)` works. With `--json` the client prints the controller's answer unchanged, indented. `fps config list --json` and `fps run list --json` print an object with `items`. Errors always go to standard error as text.

## Watching a run

`fps watch` reads the Server Sent Events stream of `GET /runs/{id}/metrics:stream`, which sends one event per second per view. It follows the views `application`, `scenario`, and `port` unless `--view` names others, and `--side server` counts the server side.

- On a terminal it redraws a table. Elsewhere, or with `--lines`, it prints one line per sample: `ts view key name=value ...`.
- It shows the headline metrics (session counts and rate, bytes, throughput, port drops). `--metric NAME` shows the named metrics and their statistics, and `--all-metrics` shows everything.
- With `--json` it prints one object per line, `{"event":"sample","sample":{...}}` for each sample and a last `{"event":"end","run":{...}}`.
- When the stream drops before its `end` event and the run still lives, the client reconnects, with a pause of one to five seconds, and gives up after ten failures in a row.
- When the controller answers 501 to the stream, or with `--poll`, the client polls `GET /runs/{id}/metrics` and the run every `--interval` (default 1s).
- When the run already ended, the command prints the final state at once.

A watch ends with the final run state, the totals, and the criteria of the verdict, and its exit code follows the outcome. Ctrl-C stops the watch and leaves the run running.

## Exit codes

| Code | Meaning |
|---|---|
| 0 | Success. A command that waits for a run exits 0 when the run completed and its verdict did not fail |
| 1 | Negative result: an invalid configuration, or a run that failed, was aborted, or missed its criteria |
| 2 | Usage error |
| 3 | HTTP 401 or 403. The message names the missing scope when the controller does |
| 4 | HTTP 404 |
| 5 | Any other error response, including 422 on an upload and 501 |
| 6 | The controller is unreachable, TLS verification failed, or the request timed out |
| 130 | Interrupted by SIGINT or SIGTERM |

`fps run show`, `fps report`, and `fps run list` print what they find and exit 0 even for a failed run, because they describe a run rather than wait for it.

## Tests

| Level | What it checks |
|---|---|
| Unit tests in `internal/fpscli/` | Each command against a stub server that records its requests: the token sources and their precedence, that no output shows a token, TLS with a CA file, every exit code, the upload content types, the pages of `run list`, the event stream with reconnection and the polling fallback, the JSON lines, the report formats, and an interrupt |
| End to end test `TestEndToEnd` in `e2e/` | The real `fps-controller` in plain mode on loopback with a temporary data directory and three tokens, and fake agents. It validates, uploads, lists, and shows a configuration, starts a run, watches it to completion, reads the run and its report in text, JSON, CSV, and HTML, starts a run whose criterion fails and expects exit 1, stops a run, and checks the 401, 403, 404, and unreachable codes |

`TestMain` builds `fps`, `fps-controller`, and `fps-fakeagent` into a temporary directory. When `../controller` cannot build, for example without access to the Go module proxy, the end to end test skips with the build error, and `-short` skips it on purpose.

### The fake agent helper

`controller/cmd/fps-fakeagent` is a thin wrapper around `controller/internal/fakeagent` that connects fake agents to a controller, because `cli/` cannot import `controller/internal/`. It is a test helper and no product.

```bash
fps-fakeagent -controller 127.0.0.1:8444 -agents local,other -ports lo -finish-after 3s
```

The agents speak plain gRPC, so the controller needs `insecure_plain_grpc: true`. They simulate traffic at the session rate of their plan. With `-finish-after` each agent ends its plan after that time, as an agent does after its last load phase, so a run completes on its own. The command prints `fake agents ready` once every agent registered, and it exits with 0 on SIGINT or SIGTERM.

## Checks

| Command | Runs |
|---|---|
| `make lint-cli` | `gofmt -l`, `go vet ./...`, and `golangci-lint run` in `cli/`, which it skips with a notice when the binary is missing |
| `make test-cli` | `go test -race ./...` in `cli/`, including the end to end test |
| `make test-cli-e2e` | Only the end to end test, verbose |

All of them build with `GOWORK=off`, as the root Makefile exports it.
