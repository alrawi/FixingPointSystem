// SPDX-License-Identifier: GPL-3.0-only

import { describe, expect, it } from "vitest";
import { formatLabels, formatLinkSpeed, holderOf } from "./agentFormat";

describe("holderOf", () => {
  it.each([
    [{ state: "online" }, { kind: "none" }],
    [{ state: "reserved" }, { kind: "unknown" }],
    [{ state: "reserved", reserved_by: "operator" }, { kind: "operator" }],
    [
      { state: "reserved", reserved_by: "run_mock0007" },
      { kind: "run", id: "run_mock0007" },
    ],
    [
      { state: "busy", reserved_by: "run_mock0008" },
      { kind: "run", id: "run_mock0008" },
    ],
    [{ state: "offline", reserved_by: "operator" }, { kind: "operator" }],
    [{ state: "busy" }, { kind: "none" }],
  ])("reads %j as %j", (agent, holder) => {
    expect(holderOf(agent)).toEqual(holder);
  });
});

describe("formatLinkSpeed", () => {
  it.each([
    [undefined, "not reported"],
    [100, "100 Mbit/s"],
    [999, "999 Mbit/s"],
    [1000, "1 Gbit/s"],
    [2500, "2.5 Gbit/s"],
    [25000, "25 Gbit/s"],
    [100000, "100 Gbit/s"],
  ])("writes %s as %s", (speed, text) => {
    expect(formatLinkSpeed(speed)).toBe(text);
  });
});

describe("formatLabels", () => {
  it("sorts the pairs by key", () => {
    expect(formatLabels({ site: "lab", rack: "r2" })).toBe("rack=r2, site=lab");
  });
  it("is empty without labels", () => {
    expect(formatLabels(undefined)).toBe("");
    expect(formatLabels({})).toBe("");
  });
});
