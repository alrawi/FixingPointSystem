## [2026-11-09] ingest | CAP-01 Capture session extraction

CAP-01 adds the `capture/` Python package and the command `python3 -m capture sessions --pcap FILE [--out FILE]`. It reads pcap and pcapng with the standard library, decodes Ethernet, IPv4, IPv6, TCP, and UDP, reassembles each TCP direction (retransmissions, overlaps, out of order buffering, holes), detects handshakes and closes, handles sessions without a handshake, groups UDP flows by 5-tuple with an idle timeout, and writes session JSON that `capture/sessions.schema.json` describes. 44 tests with synthetic captures cover the acceptance items. We created the [Capture](Capture.md) page, which documents the rules and the JSON fields that CAP-02 to CAP-04 read.

### Shared page notes

- `docs/wiki/_Sidebar.md`: add `[Capture](Capture)` next to the other workstream component pages.
- `docs/wiki/Home.md` and `docs/wiki/Implementation-Plan.md`: link the new Capture page where the CAP workstream appears.
- `docs/wiki/Testing.md`: the table row `test-capture` belongs to the engine capture. Add a row for `lint-cap test-cap` (ruff, mypy strict, pytest, no root, no external tools) and the workflow `cap` once it exists.
- CI: no workflow covers `capture/`. A job `cap` on `ubuntu-24.04` with `actions/checkout@v4`, `actions/setup-python@v5` (3.12), `python -m pip install -r capture/requirements-dev.txt`, and `make lint-cap test-cap PYTHON=python`, triggered by changes to `capture/**`, `mk/cap.mk`, `Makefile`, and `.github/**`, covers it. `.github/workflows/ci.yml` is a shared file, so the coordinator adds the job or a new `cap.yml`.
