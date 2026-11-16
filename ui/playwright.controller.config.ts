// SPDX-License-Identifier: GPL-3.0-only
//
// Browser tests of the UI against a real fps-controller with fake agents.
// `npm run build` must run first. The global setup builds or finds the Go
// binaries, starts the stack, and ends it after the run. `npm run e2e:controller`
// runs it, and the ui workflow runs it in CI.

import { defineConfig, devices } from "@playwright/test";
import { PREVIEW_PORT } from "./e2e-controller/constants";

// A machine whose Playwright browser has another revision than this package
// expects can point at the one it has. CI installs the right one.
const executablePath = process.env["FPS_E2E_CHROMIUM"];

export default defineConfig({
  testDir: "e2e-controller",
  testMatch: "*.spec.ts",
  globalSetup: "./e2e-controller/global-setup.ts",
  // The specs share one controller and one pair of fake agents, so they run one at a time.
  fullyParallel: false,
  workers: 1,
  forbidOnly: process.env["CI"] !== undefined,
  retries: 0,
  timeout: 90_000,
  expect: { timeout: 15_000 },
  reporter: process.env["CI"] === undefined ? "list" : [["list"], ["html", { open: "never" }]],
  use: {
    baseURL: `http://127.0.0.1:${String(PREVIEW_PORT)}`,
    trace: "retain-on-failure",
    ...(executablePath === undefined ? {} : { launchOptions: { executablePath } }),
  },
  projects: [{ name: "chromium", use: { ...devices["Desktop Chrome"] } }],
});
