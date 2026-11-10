// SPDX-License-Identifier: GPL-3.0-only
//
// Tests of the browser check against the JSON Schema of the contract: it reports
// each kind of schema failure at the JSON Pointer of the field.
// @vitest-environment node

import { readFileSync } from "node:fs";
import { join } from "node:path";
import { parse } from "yaml";
import { describe, expect, it } from "vitest";
import { CONTRACTS_DIR } from "../../mock/fixtures";
import { checkConfig } from "./schema";

const base = parse(
  readFileSync(join(CONTRACTS_DIR, "examples", "tests", "enterprise_mix_10g.yaml"), "utf8"),
) as Record<string, unknown>;

function at(config: Record<string, unknown>, section: string, list: string, index: number) {
  const items = (config[section] as Record<string, Record<string, unknown>[]>)[list];
  const item = items?.[index];
  if (item === undefined) throw new Error(`no ${section}.${list}[${String(index)}]`);
  return item;
}

function mutate(change: (config: Record<string, unknown>) => void): Record<string, unknown> {
  const copy = structuredClone(base);
  change(copy);
  return copy;
}

function messages(config: unknown): Record<string, string[]> {
  const out: Record<string, string[]> = {};
  for (const issue of checkConfig(config)) (out[issue.pointer] ??= []).push(issue.message);
  return out;
}

describe("checkConfig", () => {
  it("accepts a valid configuration", () => {
    expect(checkConfig(base)).toEqual([]);
  });

  it("reports a missing required field at the field", () => {
    const issues = messages(
      mutate((config) => {
        delete (config["load"] as Record<string, unknown>)["phases"];
      }),
    );
    expect(issues).toEqual({ "/load/phases": ["is required"] });
  });

  it("reports an unknown field, a wrong type, and a value outside an enum", () => {
    const issues = messages(
      mutate((config) => {
        config["surprise"] = true;
        at(config, "network", "ports", 0)["mtu"] = "big";
        (config["profile"] as Record<string, unknown>)["basis"] = "packets";
      }),
    );
    expect(issues["/surprise"]).toEqual(["is not a field of this section"]);
    expect(issues["/network/ports/0/mtu"]).toEqual(["must be integer"]);
    expect(issues["/profile/basis"]).toEqual(["must be one of sessions, bytes"]);
  });

  it("reports a pattern, a range, and an empty list", () => {
    const issues = messages(
      mutate((config) => {
        (config["metadata"] as Record<string, unknown>)["name"] = "1bad name";
        (config["timeline"] as Record<string, unknown>)["seed"] = 0;
        (config["profile"] as Record<string, unknown>)["mix"] = [];
      }),
    );
    expect(issues["/metadata/name"]?.[0]).toContain("wrong format");
    expect(issues["/timeline/seed"]).toEqual(["must be >= 1"]);
    expect(issues["/profile/mix"]).toEqual(["needs at least 1 item"]);
  });

  it("asks for an address when a host pool has neither IPv4 nor IPv6, once", () => {
    const issues = messages(
      mutate((config) => {
        delete at(config, "network", "host_pools", 0)["ipv4"];
      }),
    );
    expect(issues).toEqual({ "/network/host_pools/0": ["needs at least one of ipv4, ipv6"] });
  });

  it("reports a value that is not an object at the root", () => {
    expect(messages([])[""]).toEqual(["must be object"]);
    expect(messages(null)[""]).toEqual(["must be object"]);
  });
});
