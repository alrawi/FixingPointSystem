// SPDX-License-Identifier: GPL-3.0-only

import react from "@vitejs/plugin-react";
import { defineConfig } from "vitest/config";

// The dev server and the preview server forward API calls to a controller, by
// default the mock server that `npm run mock` starts.
const apiTarget = process.env["FPS_API_TARGET"] ?? "http://127.0.0.1:8090";
const proxy = { "/v1": { target: apiTarget, changeOrigin: true } };

export default defineConfig({
  plugins: [react()],
  server: { proxy },
  preview: { proxy },
  build: {
    target: "es2022",
    sourcemap: true,
  },
  test: {
    environment: "jsdom",
    include: ["src/**/*.test.{ts,tsx}", "mock/**/*.test.ts", "scripts/**/*.test.mjs"],
    setupFiles: ["src/test/setup.ts"],
    restoreMocks: true,
  },
});
