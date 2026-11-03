# SPDX-License-Identifier: GPL-3.0-only
#
# QA golden file helpers under tests/golden/.

# The Go and Python golden file helpers under tests/golden/. The Rust crate
# fps-golden is a workspace member, so lint-rust already covers it. Each optional
# tool is skipped with a notice when it is missing, and CI installs all of them.
LINT_TARGETS += lint-qa-golden
lint-qa: lint-qa-golden
lint-qa-golden:
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
TEST_TARGETS += test-qa-golden
test-qa: test-qa-golden
test-qa-golden:
	$(CARGO) test -p fps-golden
	@if command -v $(GO) >/dev/null 2>&1; then \
	  echo "go test -race ./... in tests/golden/go"; \
	  cd tests/golden/go && $(GO) test -race ./...; \
	else \
	  echo "go tests of test-qa skipped because go is not installed"; \
	fi
	$(PYTHON) -m pytest -q tests/golden/python

.PHONY: lint-qa test-qa
