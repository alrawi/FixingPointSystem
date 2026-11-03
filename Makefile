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
# Nobody edits this file to add checks. Each workstream area owns one file under
# mk/, for example mk/ctl.mk or mk/qa-harness.mk, and registers its targets there
# by appending to LINT_TARGETS, TEST_TARGETS, or E2E_TARGETS. Parallel packages
# therefore never change the same file here. See docs/wiki/Swarm-Protocol.md.

SHELL := bash
.SHELLFLAGS := -euo pipefail -c
.DEFAULT_GOAL := help

PYTHON ?= python3
CARGO ?= cargo
GO ?= go
GOLANGCI_LINT ?= golangci-lint

# Each Go module builds on its own, so a new module needs no shared go.work.
export GOWORK := off

LINT_TARGETS :=
TEST_TARGETS :=
E2E_TARGETS :=

include $(sort $(wildcard mk/*.mk))

.PHONY: help lint test e2e $(LINT_TARGETS) $(TEST_TARGETS) $(E2E_TARGETS)

help:
	@echo "make lint                  run every linter: $(LINT_TARGETS)"
	@echo "make test                  run every fast test: $(TEST_TARGETS)"
	@echo "make e2e                   run every end to end level as root: $(E2E_TARGETS)"
	@echo "mk/*.mk                    one file per workstream area defines these targets"

lint: $(LINT_TARGETS)

test: $(TEST_TARGETS)

e2e: $(E2E_TARGETS)
