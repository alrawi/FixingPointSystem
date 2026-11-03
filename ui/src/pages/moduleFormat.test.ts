// SPDX-License-Identifier: GPL-3.0-only

import { describe, expect, it } from "vitest";
import {
  closedParameters,
  formatValue,
  functionSignature,
  modulePath,
  parameterRows,
  providesText,
} from "./moduleFormat";

describe("parameterRows", () => {
  it("reads types, defaults, required names, constraints, and descriptions", () => {
    const rows = parameterRows({
      type: "object",
      required: ["host"],
      properties: {
        host: { type: "string", minLength: 1, description: "Server name." },
        port: { type: "integer", minimum: 1, maximum: 65535, default: 80 },
        mode: { enum: ["fast", "slow"], default: "fast" },
        ratio: { type: ["number", "null"], exclusiveMinimum: 0 },
        odd: true,
      },
    } as { type: "object" });
    expect(rows).toEqual([
      {
        name: "host",
        type: "string",
        defaultValue: null,
        required: true,
        constraints: ["at least 1 characters"],
        description: "Server name.",
      },
      {
        name: "port",
        type: "integer",
        defaultValue: "80",
        required: false,
        constraints: ["minimum 1", "maximum 65535"],
        description: null,
      },
      {
        name: "mode",
        type: "enum",
        defaultValue: "fast",
        required: false,
        constraints: ["one of fast, slow"],
        description: null,
      },
      {
        name: "ratio",
        type: "number or null",
        defaultValue: null,
        required: false,
        constraints: ["greater than 0"],
        description: null,
      },
      {
        name: "odd",
        type: "any",
        defaultValue: null,
        required: false,
        constraints: [],
        description: null,
      },
    ]);
  });

  it("returns no rows for a missing or empty schema", () => {
    expect(parameterRows(undefined)).toEqual([]);
    expect(parameterRows({ type: "object" })).toEqual([]);
  });

  it("tells a closed schema from an open one", () => {
    expect(
      closedParameters({ type: "object", additionalProperties: false } as { type: "object" }),
    ).toBe(true);
    expect(closedParameters({ type: "object" })).toBe(false);
    expect(closedParameters(undefined)).toBe(false);
  });
});

describe("formatValue", () => {
  it("keeps plain words bare and quotes the rest", () => {
    expect(formatValue("/index.html")).toBe("/index.html");
    expect(formatValue("two words")).toBe('"two words"');
    expect(formatValue("")).toBe('""');
    expect(formatValue(4096)).toBe("4096");
    expect(formatValue(false)).toBe("false");
    expect(formatValue({ a: 1 })).toBe('{"a":1}');
    expect(formatValue(undefined)).toBe("undefined");
  });
});

describe("functionSignature", () => {
  it("lists the parameters in order with their defaults", () => {
    expect(
      functionSignature("dns", {
        name: "response",
        summary: "Response.",
        params: [
          { name: "query", type: "bytes" },
          { name: "ttl", type: "int", default: 300 },
          { name: "flag", type: "boolean", default: true },
        ],
      }),
    ).toBe("dns.response(query: bytes, ttl: int = 300, flag: boolean = true)");
    expect(functionSignature("m", { name: "f", summary: "F.", params: [] })).toBe("m.f()");
  });
});

describe("providesText and modulePath", () => {
  it("names the transport and the port when one is given", () => {
    expect(
      providesText([
        { protocol: "dns", transport: "udp", port: 53 },
        { protocol: "quic", transport: "udp" },
      ]),
    ).toEqual(["dns over udp port 53", "quic over udp"]);
    expect(providesText(undefined)).toEqual([]);
  });

  it("encodes the version of a detail path", () => {
    expect(modulePath("dns", "1.0.0+build.5")).toBe("/modules/dns/1.0.0%2Bbuild.5");
  });
});
