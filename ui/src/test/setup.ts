// SPDX-License-Identifier: GPL-3.0-only

import { cleanup } from "@testing-library/react";
import { afterEach } from "vitest";

afterEach(() => {
  // Node environment tests, such as the mock contract tests, have no DOM.
  if (typeof window === "undefined") return;
  cleanup();
  window.sessionStorage.clear();
  window.history.replaceState(null, "", "/");
});
