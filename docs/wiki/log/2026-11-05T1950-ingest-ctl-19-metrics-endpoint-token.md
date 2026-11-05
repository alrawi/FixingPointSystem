## [2026-11-05] ingest | CTL-19 Optional token for the metrics endpoint

We applied decision D12 item 2. The controller configuration gained `metrics_token_file` and the flag `-metrics-token-file`. Without the setting `GET /metrics` stays open, and with it the endpoint requires `Authorization: Bearer <token>` and answers 401 otherwise. The setting takes a file so that the secret never lands in the configuration, the command line, or the logs. The check compares SHA-256 digests with `crypto/subtle.ConstantTimeCompare`. The new section Metrics endpoint token of the [Controller](Controller.md) page documents the setting, and the sentence in Metrics in the binary that said the endpoint needs no token now points to it. Tests cover an open endpoint, a required token, a wrong token, near miss tokens, the file loader, the flag layering, and exit code 2 for an unreadable file. The evidence is `docs/handoff/CTL-19.md` on the branch `wp/CTL-19` and the CI run of its pull request. The local sandbox could not download Go modules, so `make lint-ctl test-ctl` ran in CI.

## Shared page notes

The coordinator may apply two notes.

1. [Testing](Testing.md), the CTL rows. `make test-ctl` now also runs `TestMetricsEndpointRequiresConfiguredToken` and `TestMetricsEndpointIsOpenByDefault` in `controller/cmd/fps-controller`, which start the controller in process and call `GET /metrics` with and without a token.
2. [Home](Home.md) can mark CTL-19 as ready for review.
