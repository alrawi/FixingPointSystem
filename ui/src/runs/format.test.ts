// SPDX-License-Identifier: GPL-3.0-only

import { describe, expect, it } from "vitest";
import { formatDuration, formatMetric } from "./format";

describe("formatMetric", () => {
  it("keeps six significant digits", () => {
    expect(formatMetric(0.02)).toBe("0.02");
    expect(formatMetric(0.0053333333)).toBe("0.00533333");
    expect(formatMetric(1234567)).toBe("1,234,570");
    expect(formatMetric(undefined)).toBe("-");
  });
});

describe("formatDuration", () => {
  it("writes seconds, and minutes with seconds", () => {
    expect(formatDuration(45_000)).toBe("45 s");
    expect(formatDuration(65_000)).toBe("1 min 5 s");
    expect(formatDuration(undefined)).toBe("-");
  });
});
