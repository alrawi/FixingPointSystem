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
