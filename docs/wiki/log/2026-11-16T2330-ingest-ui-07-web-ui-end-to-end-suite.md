## [2026-11-16] ingest | UI-07 web UI end to end suite

UI-07 adds a Playwright suite that runs the built UI in Chromium against the real `fps-controller` and the fake agents of `fps-fakeagent`, where the earlier browser tests ran against the mock server. The suite is in `ui/e2e-controller/` and runs with `npm run e2e:controller` after `npm run build`. A global setup builds or finds the Go binaries, starts the controller on free ports with a temporary data directory and three tokens, enrolls the agents `local` and `spare`, installs the example modules `http1` and `dns`, starts the fake agents with `-finish-after 10s`, and serves the build with `vite preview`, which forwards `/v1` to the controller. Four specs cover the configuration flow (create, validate on the controller, save, list, reopen, a controller finding on a field), the run flow (start from the form, live counters, completion, a stop, a run opened in the middle, a missing run), the report flow (a passing and a failing run, the links between list, live view, and report, and the comparison), and the agents view (the registered agents and ports, reserve and release with a reload). Every page runs axe-core with the color contrast rule. The [Web UI](../Web-UI.md#end-to-end-suite-ui-07) page describes the stack, the specs, and the differences from the mock.

The worker sandbox cannot download Go modules, so the controller and the fake agent never built there. The setup and the specs ran locally against a stand-in that only logs the two start records and accepts the setup calls, which proved the process handling, the module archives, the preview proxy, and the browser launch. The first run against the real controller is the CI job below.

### Shared page notes

- Workflow `.github/workflows/ui.yml` (coordinator file, hosted runner only, no self hosted runner): add `controller/**` to the `paths` of both `push` and `pull_request`, so a controller change runs the suite, and add this job after `browser`. The workflow already sets `permissions: contents: read` for every job, and the new job needs nothing more.

  ```yaml
    controller-browser:
      runs-on: ubuntu-24.04
      timeout-minutes: 30
      env:
        GOWORK: "off"
      steps:
        - uses: actions/checkout@v4
        - uses: actions/setup-node@v4
          with:
            node-version: "22"
            cache: npm
            cache-dependency-path: ui/package-lock.json
        - uses: actions/setup-go@v5
          with:
            go-version-file: controller/go.mod
            cache-dependency-path: controller/go.sum
        - run: make ui-deps
        - name: Install the Playwright browser
          working-directory: ui
          run: npx playwright install --with-deps chromium-headless-shell
        - name: Build the UI
          working-directory: ui
          run: npm run build
        - name: Run the browser tests against a real controller
          working-directory: ui
          run: npm run e2e:controller
        - name: Keep the Playwright report and the logs of the stack
          if: failure()
          uses: actions/upload-artifact@v4
          with:
            name: ui-controller-playwright
            path: |
              ui/playwright-report
              ui/test-results
            if-no-files-found: ignore
  ```

  The browser step uses `--with-deps` exactly as the existing `browser` job does, so it adds no bare `apt-get` line of its own. The job takes the Go binaries from `go build` inside the setup, so it needs no extra build step. The header comment of `ui.yml` should add one sentence: "The controller-browser job runs the Playwright suite of UI-07 against a real controller with fake agents."
- Makefile area `mk/ui.mk` (shared): add a target that mirrors the job, outside `make e2e` and `make test` because it needs a browser and the Go module proxy:

  ```make
  # Browser suite against a real controller with fake agents (UI-07). It needs a
  # Playwright browser and Go modules, so ui.yml runs it.
  .PHONY: e2e-ui-controller
  e2e-ui-controller: ui-deps
  	cd $(UI_DIR) && $(NPM) run --silent build && $(NPM) run --silent e2e:controller
  ```

- [Testing](../Testing.md), table row `e2e-ui`: append "; `e2e-ui-controller` runs the same browser tests against a real controller with fake agents (UI-07), in the job `controller-browser` of the workflow `ui`".
- [Testing](../Testing.md), required levels: no change. The UI row already lists the browser level.
- [Module Development](../Module-Development.md), Watch out for: no entry. The package changed nothing that a module author meets.
