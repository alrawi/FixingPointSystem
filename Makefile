# SPDX-License-Identifier: GPL-3.0-only
#
# Root entry points for every check in the repository.
#
#   make lint          every formatter and linter
#   make test          the fast test levels of every workstream
#   make test-<ws>     the fast tests of one workstream, for example make test-con
#   make lint-<ws>     the linters of one workstream, when it has its own
#   make e2e           the end to end levels, which need root
#
# Each workstream owns one block below and registers its targets by appending to
# LINT_TARGETS and TEST_TARGETS on lines of its own. A package that adds a
# workstream adds a new block above the aggregate targets and never edits a shared
# line, so parallel packages do not conflict here. A block delegates to the
# workstream's own commands or scripts and keeps logic out of this file.

SHELL := bash
.SHELLFLAGS := -euo pipefail -c
.DEFAULT_GOAL := help

PYTHON ?= python3
CARGO ?= cargo

LINT_TARGETS :=
TEST_TARGETS :=
E2E_TARGETS :=

# ---------------------------------------------------------------------------
# Repository wide linters

# Rust formatting and clippy cover every crate in the Cargo workspace.
LINT_TARGETS += lint-rust
lint-rust:
	$(CARGO) fmt --all -- --check
	$(CARGO) clippy --workspace --all-targets -- -D warnings

# Every tracked shell script passes shellcheck.
LINT_TARGETS += lint-shell
lint-shell:
	git ls-files -z '*.sh' | xargs -0 -r shellcheck

# Licenses and advisories of Rust dependencies. CI runs this check in the
# licenses job, so a machine without cargo-deny skips it with a notice.
LINT_TARGETS += lint-deny
lint-deny:
	@if command -v cargo-deny >/dev/null 2>&1; then \
	  echo "$(CARGO) deny check advisories bans licenses sources"; \
	  $(CARGO) deny check advisories bans licenses sources; \
	else \
	  echo "lint-deny skipped because cargo-deny is not installed"; \
	fi

# ---------------------------------------------------------------------------
# CON contracts

TEST_TARGETS += test-con
test-con:
	$(PYTHON) tools/contracts/validate.py
	$(PYTHON) -m pytest -q tools/contracts

# ---------------------------------------------------------------------------
# OPS repository and swarm tooling

# The work package catalog and the wiki pages generated from it.
LINT_TARGETS += lint-ops
lint-ops:
	$(PYTHON) tools/plan/plan.py check
	$(PYTHON) tools/plan/plan.py render --check
	$(PYTHON) tools/plan/plan.py lint

# The plan tool tests, the workspace probe, and the Go workspace file.
TEST_TARGETS += test-ops
test-ops: test-ops-plan test-ops-workspace test-ops-gowork

test-ops-plan:
	$(PYTHON) -m pytest -q tools/plan

# Proves that a new empty crate under engine/ joins the Cargo workspace and builds
# without an edit to the root Cargo.toml. The probe runs in a temporary copy of
# the workspace root, so it never touches Cargo.lock or the source tree.
test-ops-workspace:
	tmp="$$(mktemp -d)"; \
	trap 'rm -rf "$$tmp"' EXIT; \
	cp Cargo.toml Cargo.lock "$$tmp/"; \
	cp -R spikes "$$tmp/spikes"; \
	mkdir -p "$$tmp/tests/golden" && cp -R tests/golden/rust "$$tmp/tests/golden/rust"; \
	mkdir -p "$$tmp/engine/fps-probe/src"; \
	printf '[package]\nname = "fps-probe"\nversion = "0.0.1"\nedition = "2021"\n' \
	  > "$$tmp/engine/fps-probe/Cargo.toml"; \
	printf '//! Workspace probe.\n' > "$$tmp/engine/fps-probe/src/lib.rs"; \
	CARGO_TARGET_DIR="$$tmp/target" $(CARGO) build --quiet \
	  --manifest-path "$$tmp/Cargo.toml" -p fps-probe; \
	echo "test-ops-workspace passed, engine/fps-probe built without editing Cargo.toml"

# go.work lists no modules until controller/ and cli/ exist. The check parses it
# when Go is installed and skips with a notice otherwise.
test-ops-gowork:
	@if command -v go >/dev/null 2>&1; then \
	  echo "go work edit -json"; \
	  go work edit -json >/dev/null; \
	else \
	  echo "test-ops-gowork skipped because go is not installed"; \
	fi

# ---------------------------------------------------------------------------
# SPK Phase 0 spikes

TEST_TARGETS += test-spk
test-spk:
	$(CARGO) test -p 'fps-spike-*'
	$(PYTHON) -m pytest -q spikes/s4-trex-astf

# ---------------------------------------------------------------------------
# QA test helpers

GO ?= go

# The Go and Python golden file helpers under tests/golden/. The Rust crate
# fps-golden is a workspace member, so lint-rust already covers it. Each optional
# tool is skipped with a notice when it is missing, and CI installs all of them.
LINT_TARGETS += lint-qa
lint-qa:
	@if command -v $(GO) >/dev/null 2>&1; then \
	  echo "gofmt -l and go vet in tests/golden/go"; \
	  cd tests/golden/go && test -z "$$(gofmt -l . | tee /dev/stderr)" && $(GO) vet ./...; \
	else \
	  echo "go checks of lint-qa skipped because go is not installed"; \
	fi
	@if command -v golangci-lint >/dev/null 2>&1; then \
	  echo "golangci-lint run in tests/golden/go"; \
	  cd tests/golden/go && golangci-lint run ./...; \
	else \
	  echo "golangci-lint of lint-qa skipped because it is not installed"; \
	fi
	@if command -v ruff >/dev/null 2>&1; then \
	  echo "ruff format --check and ruff check tests/golden/python"; \
	  ruff format --check tests/golden/python && ruff check tests/golden/python; \
	else \
	  echo "ruff of lint-qa skipped because it is not installed"; \
	fi
	@if command -v mypy >/dev/null 2>&1; then \
	  echo "mypy --strict tests/golden/python"; \
	  mypy --strict --cache-dir=/dev/null --python-executable "$$(command -v $(PYTHON))" \
	    tests/golden/python; \
	else \
	  echo "mypy of lint-qa skipped because it is not installed"; \
	fi

# Match, mismatch, and update tests of the three golden file helpers.
TEST_TARGETS += test-qa
test-qa:
	$(CARGO) test -p fps-golden
	@if command -v $(GO) >/dev/null 2>&1; then \
	  echo "go test -race ./... in tests/golden/go"; \
	  cd tests/golden/go && $(GO) test -race ./...; \
	else \
	  echo "go tests of test-qa skipped because go is not installed"; \
	fi
	$(PYTHON) -m pytest -q tests/golden/python
# QA test harness

# shellcheck runs on the harness scripts, and ruff and mypy on its Python helper
# when they are installed. CI installs them from tests/harness/requirements.txt.
LINT_TARGETS += lint-qa
lint-qa:
	shellcheck tests/harness/*.sh
	@if command -v ruff >/dev/null 2>&1 && command -v mypy >/dev/null 2>&1; then \
	  echo "ruff format --check tests/harness && ruff check tests/harness && mypy --strict tests/harness/pcapng.py"; \
	  ruff format --check tests/harness && ruff check tests/harness && mypy --strict tests/harness/pcapng.py; \
	else \
	  echo "lint-qa skipped ruff and mypy because they are not installed"; \
	fi

# The unit tests of the pcapng helper, which need no root.
TEST_TARGETS += test-qa
test-qa:
	$(PYTHON) -m pytest -q tests/harness

# The namespace harness self test needs root, so make test leaves it out.
E2E_TARGETS += e2e-qa
e2e-qa:
	tests/harness/selftest.sh
# AGT agent

# Capability detection against fake system roots and the agent.proto mirror test.
TEST_TARGETS += test-agt
test-agt:
	$(CARGO) test -p fps-agent
# CTL controller

GO ?= go
GOLANGCI_LINT ?= golangci-lint

# gofmt, go vet, and golangci-lint over the controller module. CI runs
# golangci-lint through its pinned action, so a machine without the binary skips
# it with a notice.
LINT_TARGETS += lint-ctl
lint-ctl:
	@out="$$(cd controller && gofmt -l .)"; \
	if [ -n "$$out" ]; then echo "gofmt needed on:"; echo "$$out"; exit 1; fi
	cd controller && $(GO) vet ./...
	@if command -v $(GOLANGCI_LINT) >/dev/null 2>&1; then \
	  echo "cd controller && $(GOLANGCI_LINT) run"; \
	  cd controller && $(GOLANGCI_LINT) run; \
	else \
	  echo "golangci-lint skipped because it is not installed"; \
	fi

TEST_TARGETS += test-ctl
test-ctl:
	cd controller && $(GO) test -race ./...
# ENG agent engine

# Every crate under engine/ is tested by package name, which equals its
# directory name, so a new engine crate joins this target without an edit here.
# lint-rust already covers the engine crates.
ENG_CRATES := $(notdir $(patsubst %/,%,$(dir $(wildcard engine/*/Cargo.toml))))

TEST_TARGETS += test-eng
test-eng:
	$(CARGO) test $(addprefix -p ,$(ENG_CRATES))
# RLM realism validation. realism/requirements-dev.txt lists the tools, and the
# optional ndpiReader test skips when nDPI is not installed.

LINT_TARGETS += lint-rlm
lint-rlm:
	$(PYTHON) -m ruff format --check realism
	$(PYTHON) -m ruff check realism
	$(PYTHON) -m mypy --config-file realism/pyproject.toml realism

TEST_TARGETS += test-rlm
test-rlm:
	$(PYTHON) -m pytest -q realism

# ---------------------------------------------------------------------------
# Aggregate targets. New workstream blocks go above this line, because make
# expands the prerequisite lists below when it reads them.

.PHONY: help lint test $(LINT_TARGETS) $(TEST_TARGETS) \
	test-ops-plan test-ops-workspace test-ops-gowork
.PHONY: e2e $(E2E_TARGETS)

help:
	@echo "make lint                  run every linter: $(LINT_TARGETS)"
	@echo "make test                  run every fast test: $(TEST_TARGETS)"
	@echo "make test-<workstream>     run the fast tests of one workstream"
	@echo "make e2e                   run every end to end level as root: $(E2E_TARGETS)"

lint: $(LINT_TARGETS)

test: $(TEST_TARGETS)

e2e: $(E2E_TARGETS)
