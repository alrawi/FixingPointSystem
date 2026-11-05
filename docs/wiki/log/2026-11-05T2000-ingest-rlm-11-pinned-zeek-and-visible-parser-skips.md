## [2026-11-05] ingest | RLM-11 pinned Zeek and visible parser skips

We applied decision D12 items 5 and 23 in `realism/` and the two realism workflows. The file `realism/ci/zeek.version` now holds the only Zeek version, `9.0.1-0`, which `realism/ci/install-tools.sh` installs from the openSUSE Build Service repository `security:zeek`, and both the `rlm` and the `realism` workflow call that script. The `parser_errors` check gained `skipped_parsers`, a list of each parser that `--parsers auto` skipped with its reason, and the report format moved to 0.3.0. The CLI, the module job summary, and the runner README show the skipped parsers too, and the [Realism](Realism.md) page has a new section. The evidence is `docs/handoff/RLM-11.md` and the tests in `realism/tests/`, including a run with only tshark on the search path that reports Zeek as skipped.

## Shared page notes

- [Testing](Testing.md): the `rlm` workflow no longer installs Zeek inline, it calls `realism/ci/install-tools.sh`, and the Zeek pin lives in `realism/ci/zeek.version`.
- [Implementation Plan](Implementation-Plan.md): the realism report seam is at format 0.3.0.
