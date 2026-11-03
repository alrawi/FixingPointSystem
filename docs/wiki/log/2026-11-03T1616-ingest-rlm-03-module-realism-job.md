## [2026-11-03] ingest | RLM-03 Module realism CI job

We added the `realism` workflow, which records a fresh capture of every native module that a pull request touches in the QA-01 namespace harness, scores it with nDPI, tshark, and Zeek, fails on a failed conformance or parser error check, and uploads the capture and the realism report as a workflow artifact. A negative control job breaks the http1 request line and requires the run to fail. We added the module selection as `python3 -m realism modules` and described the job in a new section of [Realism](Realism.md). The evidence is `docs/handoff/RLM-03.md`, the tests in `realism/tests/test_modules.py`, and the workflow runs of the pull request.

### Shared page notes

- [Testing](Testing.md) can list the new end to end target `e2e-rlm-modules`, which records and scores every native module as root.
- The plan entry of RLM-03 names `.github/workflows/realism.yml`, and the rlm area already has `rlm.yml`. We kept the module job in `realism.yml`, because its triggers cover `modules/**` and the job is slower than the unit tests of `rlm.yml`.
- A new native module joins the job when it ships `native/Cargo.toml`, `fixtures/capture.sh`, and `fixtures/expected.yaml`, and its capture script follows the interface of the http1 and dns scripts. The module template page can say so.
