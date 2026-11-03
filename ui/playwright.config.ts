// SPDX-License-Identifier: GPL-3.0-only
//
// Browser smoke tests against the built UI and the mock server. `npm run build`
// must run first. UI-07 adds the suite against a real controller.

import { defineConfig, devices } from "@playwright/test";

const mockPort = 8090;
const previewPort = 4173;

export default defineConfig({
  testDir: "e2e",
  forbidOnly: process.env["CI"] !== undefined,
  retries: 0,
  reporter: process.env["CI"] === undefined ? "list" : [["list"], ["html", { open: "never" }]],
  use: {
    baseURL: `http://127.0.0.1:${previewPort}`,
    trace: "retain-on-failure",
  },
  projects: [{ name: "chromium", use: { ...devices["Desktop Chrome"] } }],
  webServer: [
    {
      command: "npm run mock",
      env: { FPS_MOCK_PORT: String(mockPort) },
      url: `http://127.0.0.1:${mockPort}/v1/capabilities`,
      // The mock answers 401 without a token, which Playwright counts as ready.
      reuseExistingServer: false,
    },
    {
      command: `npx vite preview --host 127.0.0.1 --port ${previewPort} --strictPort`,
      env: { FPS_API_TARGET: `http://127.0.0.1:${mockPort}` },
      url: `http://127.0.0.1:${previewPort}`,
      reuseExistingServer: false,
    },
  ],
});
