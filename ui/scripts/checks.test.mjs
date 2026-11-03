// SPDX-License-Identifier: GPL-3.0-only
// @vitest-environment node

import { describe, expect, it } from "vitest";
import { initialScripts } from "./check-bundle.mjs";
import { isAllowed } from "./check-licenses.mjs";

describe("isAllowed", () => {
  it.each([
    ["MIT", true],
    ["Apache-2.0", true],
    ["(MIT OR CC0-1.0)", true],
    ["GPL-2.0-only OR MIT", true],
    ["MIT AND Apache-2.0", true],
    ["MIT AND SSPL-1.0", false],
    ["GPL-2.0-only", false],
    ["BUSL-1.1", false],
    ["UNLICENSED", false],
  ])("judges %s as %s", (expression, expected) => {
    expect(isAllowed(expression)).toBe(expected);
  });

  it("rejects a malformed expression", () => {
    expect(() => isAllowed("(MIT OR")).toThrow();
  });
});

describe("initialScripts", () => {
  it("finds module scripts and module preloads in either attribute order", () => {
    const html = `
      <script type="module" crossorigin src="/assets/index-a.js"></script>
      <link rel="modulepreload" crossorigin href="/assets/vendor-b.js">
      <link href="/assets/other-c.js" rel="modulepreload">
      <link rel="stylesheet" href="/assets/index.css">`;
    expect(initialScripts(html)).toEqual([
      "/assets/index-a.js",
      "/assets/vendor-b.js",
      "/assets/other-c.js",
    ]);
  });
});
