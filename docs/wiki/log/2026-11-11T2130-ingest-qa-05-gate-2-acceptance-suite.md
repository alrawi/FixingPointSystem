## [2026-11-11] ingest | QA-05 Gate 2 acceptance suite

QA-05 added the Gate 2 suite under `tests/gates/gate2/`, `mk/qa-gate2.mk`, and the workflow `.github/workflows/qa-gate2.yml`. The [Gates](../Gates.md) page has a Gate 2 section.

What the suite does:
- Module stage: records and scores each of the 14 native modules with `realism/ci/module-realism.sh` and its fixture peer. A second table maps the 15 applications of D7 to their evidence (HTTPS is the encryption check of `http1` and `http2`).
- Mix stage: runs a byte weighted mix of 15 entries (weights add up to 100) through the controller and two agent identities in the QA-01 harness, and scores the capture with the realism runner (`mix_accuracy`, conformance, parser errors) against the byte weights with a 2 point tolerance.
- Image stage: IMG-04's `boot-test.sh --role all-in-one --run` builds, boots, and runs a test unattended.
- `gate2_report.py` writes `report.md` and `summary.json` with every measured value. It exits 0, 1, or 2, and a selected stage without files fails.
- `prove-failure.sh` shows that the gate fails on a raised threshold, a broken module report, and a mix scored at 0 points.

Result of the CI run on pull request 487: all five jobs passed with `PASS WITH WAIVERS`. All 14 modules passed at 100% conformance, the mix stayed within 1.37 points of its weights (nDPI, HTTPS row) with 19,606 sessions and none failed, and the image booted on KVM with 51 checks and an unattended run of 557 sessions.

Findings for the coordinator:
- The controller catalog does not pass the manifest `expected_bytes` to the compiler (`controller/internal/wiring/adapters.go` leaves `compiler.Scenario.BytesPerSession` empty, and `registry.Manifest` has no such field). `profile.basis: bytes` and the `throughput_bps` objective therefore fail with `no bytes per session estimate` on any real controller, and the byte weighted mix control of CTL-12 has no estimates either. CTL-34, CTL-21, and CTL-12 are tested only with fake catalogs. The suite works around it with `FPS_GATE2_BASIS=sessions` (a named waiver). A CTL package should read `expected_bytes` into the catalog, and then the line in the workflow goes.
- AGT-21 (#457) is not a dependency of QA-05, but the full 15 application mix needs it. The suite waives `hls`, `ldap`, `mqtt`, and `ntp` while `engine/fps-agent` lacks their crates, and the waiver lapses by itself when AGT-21 merges. Gate 2 does not hold in full before that, so the owner reads the verdict line, `PASS WITH WAIVERS`, before signing it off.
- Most module manifests have no `expected_bytes` (the plan promises one per scenario). The suite stages manifests with the estimates of `mix.yaml`.
- The run report counts the FTP data connection and the SIP media stream as sessions of their own, so the bytes per counted session of `ftp` and `sip_rtp` are half of the bytes per scenario session. This looks intended, but the metric catalog does not say it.
- The `Run.Calibration` property is not in the OpenAPI schema, so the suite cannot read the multipliers of the mix control from the report.
- The nDPI byte share of the HTTPS row exceeds its weight by 1.37 points, against 0.90 points by the run metrics, because TLS records add bytes. A heavier TLS share needs a wider tolerance or the metrics side only.
- Not verified locally: the controller build (no Go module proxy), Zeek, KVM, QEMU, and Packer. CI ran all of them. The local checks were `make lint-qa-gate2 test-qa-gate2` and the first two negative proofs on synthetic reports.

### Shared page notes

- [Testing](../Testing.md): add the suite under the QA rows: `tests/gates/gate2/`, the workflow `qa-gate2.yml`, `make build-qa-gate2`, `sudo make e2e-qa-gate2`, and `make e2e-qa-gate2-image`, with a pointer to the Gate 2 section of [Gates](../Gates.md).
- [Home](../Home.md): QA-05 status line for the batch, with the verdict `PASS WITH WAIVERS` and the two waivers.
- [Controller](../Controller.md): a note that the scenario catalog does not read `expected_bytes` yet (see the first finding), until a CTL package does.
- [Implementation Plan](../Implementation-Plan.md): the Gate 2 criterion holds in full only after AGT-21 merges and the catalog carries `expected_bytes`.
