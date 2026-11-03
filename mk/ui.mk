# SPDX-License-Identifier: GPL-3.0-only
#
# UI web UI under ui/. The targets need Node.js 22.22 or later and npm. A
# machine without npm skips them with a notice, and CI runs them in ui.yml.
# The first run installs the locked dependencies with npm ci.

NPM ?= npm
UI_DIR := ui

# Installs the dependencies when node_modules is missing or older than the lock file.
.PHONY: ui-deps
ui-deps:
	@if [ ! -f $(UI_DIR)/node_modules/.package-lock.json ] || \
	    [ $(UI_DIR)/package-lock.json -nt $(UI_DIR)/node_modules/.package-lock.json ]; then \
	  echo "cd $(UI_DIR) && $(NPM) ci"; \
	  cd $(UI_DIR) && $(NPM) ci; \
	fi

# Formatting, lint with warnings as errors, strict type checks, the currency of
# the generated client, and the licenses of every locked package.
LINT_TARGETS += lint-ui
lint-ui:
	@if ! command -v $(NPM) >/dev/null 2>&1; then \
	  echo "lint-ui skipped because npm is not installed"; exit 0; \
	fi; \
	$(MAKE) --no-print-directory ui-deps; \
	cd $(UI_DIR); \
	$(NPM) run --silent format:check; \
	$(NPM) run --silent lint; \
	$(NPM) run --silent typecheck; \
	$(NPM) run --silent check:api; \
	$(NPM) run --silent check:licenses

# Unit, component, and contract tests against the mock server, the production
# build, and the initial JavaScript budget of 250 kB gzip.
TEST_TARGETS += test-ui
test-ui:
	@if ! command -v $(NPM) >/dev/null 2>&1; then \
	  echo "test-ui skipped because npm is not installed"; exit 0; \
	fi; \
	$(MAKE) --no-print-directory ui-deps; \
	cd $(UI_DIR); \
	$(NPM) run --silent test; \
	$(NPM) run --silent build; \
	$(NPM) run --silent check:bundle

# Browser smoke tests with axe-core against the build and the mock server. It
# needs a Playwright browser, which `npx playwright install chromium` fetches,
# so make e2e leaves it out and ui.yml runs it.
.PHONY: e2e-ui
e2e-ui: ui-deps
	cd $(UI_DIR) && $(NPM) run --silent build && $(NPM) run --silent e2e
