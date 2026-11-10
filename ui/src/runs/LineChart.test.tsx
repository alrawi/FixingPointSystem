// SPDX-License-Identifier: GPL-3.0-only

import { render, screen, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import {
  formatBitsPerSecond,
  formatBytes,
  formatClock,
  formatCount,
  formatDecimal,
  formatMicros,
  formatPercent,
} from "./format";
import { LineChart, plot } from "./LineChart";

const T0 = Date.UTC(2026, 10, 1, 10, 0, 0);
const points = [10, 20, 40].map((v, i) => ({ t: T0 + i * 1000, v }));

describe("plot", () => {
  it("starts the y axis at zero, rounds its end up, and spans the time range", () => {
    const { lines, vMax, tMin, tMax } = plot([{ id: "a", label: "a", points }], 300);
    expect(vMax).toBe(50);
    expect(tMax - tMin).toBe(2000);
    // The plot area spans x 76 to 628 and y 10 to 174, and y grows downward.
    expect(lines[0]?.path).toBe("76,141.2 352,108.4 628,42.8");
  });
});

describe("LineChart", () => {
  it("draws one polyline per series with its values and names the latest values", () => {
    render(
      <LineChart
        title="Rate"
        series={[
          { id: "a", label: "first", points },
          { id: "b", label: "second", points: points.map((p) => ({ ...p, v: p.v / 2 })) },
        ]}
        format={formatDecimal}
      />,
    );
    const svg = screen.getByRole("img", { name: /Rate/ });
    expect(svg.querySelectorAll("polyline")).toHaveLength(2);
    expect(svg.querySelector('polyline[data-series="a"]')?.getAttribute("data-values")).toBe(
      "10,20,40",
    );
    expect(svg.querySelector("desc")?.textContent).toBe(
      "Line chart of the last 3 seconds. Latest: first 40, second 20.",
    );
    const table = screen.getByRole("table", { name: "Newest values of Rate", hidden: true });
    const rows = within(table).getAllByRole("row", { hidden: true });
    expect(rows.map((row) => row.textContent)).toEqual([
      "Time (UTC)firstsecond",
      "10:00:024020",
      "10:00:012010",
      "10:00:00105",
    ]);
  });

  it("says that it waits when no series has a point", () => {
    render(
      <LineChart title="Rate" series={[{ id: "a", label: "a", points: [] }]} format={String} />,
    );
    expect(screen.getByText("Waiting for the first sample.")).toBeDefined();
    expect(screen.queryByRole("img")).toBeNull();
  });

  it("draws a single point", () => {
    render(
      <LineChart
        title="One"
        series={[{ id: "a", label: "a", points: points.slice(0, 1) }]}
        format={String}
      />,
    );
    expect(document.querySelectorAll("circle")).toHaveLength(1);
  });
});

describe("formats", () => {
  it("formats counts, decimals, and ratios in a fixed locale", () => {
    expect(formatCount(1234567)).toBe("1,234,567");
    expect(formatCount(undefined)).toBe("-");
    expect(formatDecimal(12.34)).toBe("12.3");
    expect(formatPercent(0.0167)).toBe("1.67 %");
  });

  it("formats latencies, rates, and sizes with decimal prefixes", () => {
    expect(formatMicros(850)).toBe("850 µs");
    expect(formatMicros(13000)).toBe("13.0 ms");
    expect(formatBitsPerSecond(999)).toBe("999 bit/s");
    expect(formatBitsPerSecond(3_276_800)).toBe("3.28 Mbit/s");
    expect(formatBitsPerSecond(1e10)).toBe("10 Gbit/s");
    expect(formatBytes(1_638_400)).toBe("1.64 MB");
    expect(formatClock(T0 + 5000)).toBe("10:00:05");
  });
});
