# SPDX-License-Identifier: GPL-3.0-only
#
# CLI command line client. The Go client in cli/ (CLI-02) uses only the standard
# library. The Python SDK of CLI-01 in sdk/python/ has no target here yet.


# gofmt, go vet, and golangci-lint over the cli module. CI runs golangci-lint
# through its pinned action, so a machine without the binary skips it with a
# notice.
LINT_TARGETS += lint-cli
lint-cli:
	@out="$$(cd cli && gofmt -l .)"; \
	if [ -n "$$out" ]; then echo "gofmt needed on:"; echo "$$out"; exit 1; fi
	cd cli && $(GO) vet ./...
	@if command -v $(GOLANGCI_LINT) >/dev/null 2>&1; then \
	  echo "cd cli && $(GOLANGCI_LINT) run"; \
	  cd cli && $(GOLANGCI_LINT) run; \
	else \
	  echo "golangci-lint skipped because it is not installed"; \
	fi

# The unit tests run against a stub server. The end to end tests in cli/e2e
# build fps-controller and fps-fakeagent from controller/ and skip with a
# message when that build fails, for example without network access to the Go
# module proxy. Pass -short to skip them on purpose.
TEST_TARGETS += test-cli
test-cli:
	cd cli && $(GO) test -race ./...

# Only the end to end tests, which need the controller to build.
.PHONY: test-cli-e2e
test-cli-e2e:
	cd cli && $(GO) test -race -count=1 -v ./e2e/
