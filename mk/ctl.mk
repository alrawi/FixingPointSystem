# SPDX-License-Identifier: GPL-3.0-only
#
# CTL controller.


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

# CTL-21 throughput rate calibration. The package uses only the standard
# library, and the targets above cover it too, so these two give a fast loop for
# the package alone.
.PHONY: lint-ctl-calibrate test-ctl-calibrate
lint-ctl-calibrate:
	@out="$$(cd controller && gofmt -l internal/runs/calibrate)"; \
	if [ -n "$$out" ]; then echo "gofmt needed on:"; echo "$$out"; exit 1; fi
	cd controller && $(GO) vet ./internal/runs/calibrate/

test-ctl-calibrate:
	cd controller && $(GO) test -race -cover ./internal/runs/calibrate/

# CTL-15 mutual TLS. The package uses only the standard library, and the
# targets above cover it too, so these two give a fast loop for the package
# alone. The tests open real TLS connections on the loopback interface.
.PHONY: lint-ctl-pki test-ctl-pki
lint-ctl-pki:
	@out="$$(cd controller && gofmt -l internal/pki)"; \
	if [ -n "$$out" ]; then echo "gofmt needed on:"; echo "$$out"; exit 1; fi
	cd controller && $(GO) vet ./internal/pki/

test-ctl-pki:
	cd controller && $(GO) test -race -cover ./internal/pki/

# CTL-13 HTML report. The package renders with html/template and draws its charts
# as inline SVG, so these two give a fast loop for the package alone. Run
# `cd controller && go test ./internal/report/html/ -update` after a change of
# the template, then review the diff of the golden pages in testdata/.
.PHONY: lint-ctl-report-html test-ctl-report-html
lint-ctl-report-html:
	@out="$$(cd controller && gofmt -l internal/report/html)"; \
	if [ -n "$$out" ]; then echo "gofmt needed on:"; echo "$$out"; exit 1; fi
	cd controller && $(GO) vet ./internal/report/html/

test-ctl-report-html:
	cd controller && $(GO) test -race -cover ./internal/report/html/

# CTL-25 scenario contract v0.3 in the module registry. The targets above cover
# the package too, so these two give a fast loop for it alone.
.PHONY: lint-ctl-registry test-ctl-registry
lint-ctl-registry:
	@out="$$(cd controller && gofmt -l internal/registry)"; \
	if [ -n "$$out" ]; then echo "gofmt needed on:"; echo "$$out"; exit 1; fi
	cd controller && $(GO) vet ./internal/registry/

test-ctl-registry:
	cd controller && $(GO) test -race -cover ./internal/registry/

# CTL-27 mutual TLS in the controller. The targets above cover the packages
# too, so these two give a fast loop for the TLS wiring alone: the settings, the
# session end and expiry of the agent manager, the wiring of the authority, the
# enrollment listener and the admin socket, and the binary with its token and
# revoke subcommands. The tests open real TLS connections and a unix socket on
# the local machine, and the end to end test builds the binary.
.PHONY: lint-ctl-tls test-ctl-tls
lint-ctl-tls:
	@out="$$(cd controller && gofmt -l internal/app internal/agents internal/wiring cmd)"; \
	if [ -n "$$out" ]; then echo "gofmt needed on:"; echo "$$out"; exit 1; fi
	cd controller && $(GO) vet ./internal/app/ ./internal/agents/ ./internal/wiring/ ./cmd/...

test-ctl-tls:
	cd controller && $(GO) test -race ./internal/app/ ./internal/agents/ ./internal/wiring/ ./cmd/... -run 'TLS|Tls|Admin|Expired|EndSession|AutoEnroll|PlainMode|Secure|Config|ServerNames|PKI|ExitCodes|SIGTERM|EndToEnd'

# CTL-11 module signing and the signed registry index. The package uses only the
# standard library, and the targets above cover it too, so these two give a fast
# loop for the package alone.
.PHONY: lint-ctl-signing test-ctl-signing
lint-ctl-signing:
	@out="$$(cd controller && gofmt -l internal/registry/signing)"; \
	if [ -n "$$out" ]; then echo "gofmt needed on:"; echo "$$out"; exit 1; fi
	cd controller && $(GO) vet ./internal/registry/signing/

test-ctl-signing:
	cd controller && $(GO) test -race -cover ./internal/registry/signing/

# CTL-24 throughput calibration in the run loop. The targets above cover the
# package too, so these two give a fast loop for the run orchestrator alone. The
# end to end tests simulate agents and run for a few seconds.
.PHONY: lint-ctl-runs test-ctl-runs
lint-ctl-runs:
	@out="$$(cd controller && gofmt -l internal/runs)"; \
	if [ -n "$$out" ]; then echo "gofmt needed on:"; echo "$$out"; exit 1; fi
	cd controller && $(GO) vet ./internal/runs/...

test-ctl-runs:
	cd controller && $(GO) test -race -cover ./internal/runs/...

# CTL-28 signature checks at install and in plans. The targets above cover these
# packages too, so this one gives a fast loop for the registry, the compiler, and
# the signing settings of the app configuration.
.PHONY: test-ctl-trust
test-ctl-trust:
	cd controller && $(GO) test -race ./internal/registry/... ./internal/compiler/ ./internal/app/ ./internal/wiring/

# CTL-14 captures API. The targets above cover these packages too, so these two
# give a fast loop for the capture service, the upload path of the agent
# manager, and the fake agent. The tests move files through in memory gRPC
# connections and take a few seconds.
.PHONY: lint-ctl-captures test-ctl-captures
lint-ctl-captures:
	@out="$$(cd controller && gofmt -l internal/captures internal/agents internal/fakeagent)"; \
	if [ -n "$$out" ]; then echo "gofmt needed on:"; echo "$$out"; exit 1; fi
	cd controller && $(GO) vet ./internal/captures/ ./internal/agents/ ./internal/fakeagent/

test-ctl-captures:
	cd controller && $(GO) test -race -cover ./internal/captures/ ./internal/agents/ ./internal/fakeagent/

# CTL-12 byte weighted mix control. The package mixcontrol uses only the
# standard library, and the targets above cover it too, so the first two give a
# fast loop for the package alone. The last target runs the end to end tests of
# the run loop with the fake agent, which take a few seconds.
.PHONY: lint-ctl-mixcontrol test-ctl-mixcontrol test-ctl-bytemix
lint-ctl-mixcontrol:
	@out="$$(cd controller && gofmt -l internal/mixcontrol)"; \
	if [ -n "$$out" ]; then echo "gofmt needed on:"; echo "$$out"; exit 1; fi
	cd controller && $(GO) vet ./internal/mixcontrol/

test-ctl-mixcontrol:
	cd controller && $(GO) test -race -cover ./internal/mixcontrol/

test-ctl-bytemix:
	cd controller && $(GO) test -race -run 'ByteMix|Calibration' ./internal/runs/ ./internal/fakeagent/
