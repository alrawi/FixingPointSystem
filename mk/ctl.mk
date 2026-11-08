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
