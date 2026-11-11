## [2026-11-10] ingest | MOD-15 HLS video module

MOD-15 added the `hls` module under [`modules/hls`](https://github.com/alrawi/FixingPointSystem/tree/main/modules/hls). The crate `fps-module-hls` has eleven template functions: the master and media playlists (RFC 8216), MPEG-TS segments of the size that the bit rate and the segment duration give, the readers of a player (`playlist_uri`, `playlist_count`, `resolve`, `content_length`), the ladder walk (`rendition`, `next`), and `pace`. The module requires `http1` and takes the HTTP framing from `http1.request` and `http1.response`. The three scenarios are `vod_ladder` (one keepalive connection, port 8090), `vod_segments` (a playlist connection on 8091 that spawns one connection per segment on 8092), and `live_refresh` (port 8093).

Acceptance. The temporal check of the realism runner accepts the segment cadence: the 32 gaps between the segment sessions of `vod_segments` lie between 398.2 and 401.8 ms against a constant 400 ms with a tolerance of 40 ms. The runner compares session starts only, so gaps inside a keepalive connection and bytes need a check of their own, `fixtures/hls_check.py` (the `rtp_check.py` precedent). It reports a mean gap of 400.1 to 400.3 ms in all 7 playbacks and a bit rate within 0.5% of the configured one (375327 against 373333 bit/s for `vod_ladder`, 321440 against 320000 for `live_refresh`). The tests of the check build synthetic captures and need no tshark.

Findings.

- The first tshark run reported "missing TS frames" for the segments, because every segment restarted its continuity counters, and the PAT and the PMT shared one counter. The counters now continue from segment to segment: video and audio take a multiple of 16 packets, so they start at 0 in every segment, and the PAT and the PMT start at the segment number modulo 16. The capture has no parser error.
- The accept timeout of the runtime is 5 s, so a multi flow scenario with spawns waits for the k-th server spawn of the k-th client spawn. A flow has no arithmetic or condition, so a ladder is stages times segments per stage in the scenario variables.
- nDPI 4.2.0 classifies 40 of 40 sessions as HTTP. nDPI 4.14 in CI is not confirmed yet. Zeek did not run locally.
- The agent runs `vod_ladder` and `live_refresh` after the registry entry below. `vod_segments` is a multi flow scenario, which AGT-16 allows for TCP, but this package verified it through the interpreter and `fixture_peer` only, not through the agent.

### Shared page notes

- `mk/mod-hls.mk` (coordinator file), exact content:

```make
# SPDX-License-Identifier: GPL-3.0-only
#
# MOD hls module under modules/hls/. The Rust tests also run in the rust job of
# ci.yml through cargo test --workspace, and lint-rust covers the crate.

# Scratch files go under the Cargo target directory, which Git ignores.
MOD_HLS_OUT := $(or $(CARGO_TARGET_DIR),target)/mod-hls

# validate.py treats each directory under --modules as a module, so the target
# validates the module through a parent directory that holds links to the
# module and to http1, which it requires.
LINT_TARGETS += lint-mod-hls
lint-mod-hls:
	mkdir -p $(MOD_HLS_OUT)/modules
	ln -sfn $(CURDIR)/modules/hls $(MOD_HLS_OUT)/modules/hls
	ln -sfn $(CURDIR)/modules/http1 $(MOD_HLS_OUT)/modules/http1
	$(PYTHON) tools/contracts/validate.py --modules $(MOD_HLS_OUT)/modules --tests --no-openapi

# The crate tests, the tests of the playback check, then the realism score of
# the committed fixture and the playback check, which need ndpiReader and
# tshark.
TEST_TARGETS += test-mod-hls
test-mod-hls:
	$(CARGO) test -p fps-module-hls
	PYTHONDONTWRITEBYTECODE=1 $(PYTHON) -m unittest discover -s modules/hls/fixtures
	@if command -v ndpiReader >/dev/null 2>&1 && command -v tshark >/dev/null 2>&1; then \
	  mkdir -p $(MOD_HLS_OUT); \
	  echo "$(PYTHON) -m realism score --pcap modules/hls/fixtures/hls.pcapng --labels modules/hls/fixtures/expected.yaml --parsers tshark"; \
	  $(PYTHON) -m realism score --pcap modules/hls/fixtures/hls.pcapng \
	    --labels modules/hls/fixtures/expected.yaml --parsers tshark \
	    --output $(MOD_HLS_OUT)/fixture.report.json; \
	  echo "modules/hls/fixtures/hls_check.py modules/hls/fixtures/hls.pcapng"; \
	  $(PYTHON) -I modules/hls/fixtures/hls_check.py modules/hls/fixtures/hls.pcapng; \
	else \
	  echo "test-mod-hls skipped the fixture score and the playback check because ndpiReader or tshark is not installed"; \
	fi

# Records a fresh capture in the QA-01 harness, scores it, and runs the session
# count check and the playback check. It needs root, so make test leaves it
# out, and it never overwrites the committed fixture.
E2E_TARGETS += e2e-mod-hls
e2e-mod-hls:
	modules/hls/fixtures/capture.sh $(MOD_HLS_OUT)/capture

.PHONY: lint-mod-hls test-mod-hls e2e-mod-hls
```

- `.github/workflows/mod-hls.yml` (coordinator file), exact content. It installs tools through `realism/ci/install-tools.sh` only. The workflow scores with tshark only, because Zeek did not run on this capture locally; add `,zeek` to `--parsers` once a CI run shows it clean.

```yaml
# SPDX-License-Identifier: GPL-3.0-only
#
# Workflow of the hls module under modules/hls/. The rust job of ci.yml runs the
# Rust tests. The module job validates the manifest and scenarios, scores the
# committed fixture with nDPI and tshark, and runs the playback check, which
# reads the segment cadence inside a connection and the byte share of the
# rungs. The job runs on a GitHub hosted runner, never on a self hosted one.
name: mod-hls

on:
  push:
    branches: [main]
    paths:
      - modules/hls/**
      - modules/http1/**
      - engine/fps-flow/**
      - Cargo.lock
      - realism/**
      - tools/contracts/**
      - contracts/schemas/**
      - mk/mod-hls.mk
      - .github/workflows/mod-hls.yml
  pull_request:
    paths:
      - modules/hls/**
      - modules/http1/**
      - engine/fps-flow/**
      - Cargo.lock
      - realism/**
      - tools/contracts/**
      - contracts/schemas/**
      - mk/mod-hls.mk
      - .github/workflows/mod-hls.yml

permissions:
  contents: read

env:
  GOWORK: "off"

jobs:
  module:
    runs-on: ubuntu-24.04
    timeout-minutes: 30
    steps:
      - uses: actions/checkout@v4
      - uses: actions/setup-python@v5
        with:
          python-version: "3.12"
      - name: Install nDPI and tshark, which the checks call out of process
        run: realism/ci/install-tools.sh
      - run: python -m pip install -r tools/contracts/requirements.txt
      - run: make lint-mod-hls PYTHON=python
      - name: Unit tests of the playback check
        run: PYTHONDONTWRITEBYTECODE=1 python -m unittest discover -s modules/hls/fixtures
      - name: Score the committed fixture
        run: |
          python -m realism score --pcap modules/hls/fixtures/hls.pcapng \
            --labels modules/hls/fixtures/expected.yaml --parsers tshark --output report.json
          python -c 'import json; r = json.load(open("report.json")); print(json.dumps([c.get("applications", c["check"]) for c in r["checks"]], indent=2))'
      - name: Check the segment cadence and the byte share of the committed fixture
        run: python -I modules/hls/fixtures/hls_check.py modules/hls/fixtures/hls.pcapng
```

- `engine/fps-agent/src/modules.rs` and `engine/fps-agent/Cargo.toml` (AGT follow up): the module has state, so it needs a full registration, like `http2`, after the `Smb2` line, and the dependency `fps-module-hls = { path = "../../modules/hls/native" }` in the dependencies of the agent crate. Add the name `hls` to the list in the registry test of `modules.rs` (the one that lists `"http1", "dns", ...`).

```rust
    // Hls holds the segment counter and the pace of one flow side, so every
    // session side gets its own copy, as with Http2.
    Registration {
        name: fps_module_hls::MODULE_NAME,
        catalog: || catalog_of!(fps_module_hls),
        build: |ctx| {
            if ctx.given.is_some() {
                return Err(BuildError::Settings(
                    "the plan gives settings, but its native functions take none".to_owned(),
                ));
            }
            Ok(Box::new(fps_module_hls::Hls::default()))
        },
    },
```

- `Cargo.lock`: gained the entry of `fps-module-hls` (16 lines), which this package committed, because the workspace glob picks the crate up.
- [Modules](Modules.md): updated, a table row and a section. [Home](Home.md), [Testing](Testing.md), [Realism](Realism.md), [Implementation Plan](Implementation-Plan.md): none.
- [Realism](Realism.md), RLM follow up: the arrival check compares session starts only, so a keepalive connection with paced requests has no property. A per request inter arrival property would replace the part of `hls_check.py` that reads cadence, as for `rtp_check.py`.
