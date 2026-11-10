// SPDX-License-Identifier: GPL-3.0-only
//
// Contract tests of the configuration operations of the mock server: every
// response validates against the response schema of its operation in
// fps-api-v1.yaml.
// @vitest-environment node

import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { Ajv2020 } from "ajv/dist/2020.js";
import { afterAll, beforeAll, describe, expect, it } from "vitest";
import { parse } from "yaml";
import { CONTRACTS_DIR, loadMockData } from "./fixtures";
import { MOCK_TOKEN, startMockServer, type RunningMockServer } from "./server";

const API_ID = "https://github.com/alrawi/FixingPointSystem/contracts/openapi/fps-api-v1.yaml";

function buildValidator(): Ajv2020 {
  const ajv = new Ajv2020({ strict: false, validateSchema: false, allErrors: true });
  const schemaDir = join(CONTRACTS_DIR, "schemas");
  for (const file of readdirSync(schemaDir).filter((name) => name.endsWith(".json"))) {
    ajv.addSchema(JSON.parse(readFileSync(join(schemaDir, file), "utf8")) as object);
  }
  const api = parse(readFileSync(join(CONTRACTS_DIR, "openapi", "fps-api-v1.yaml"), "utf8")) as {
    [key: string]: unknown;
  };
  ajv.addSchema({ ...api, $id: API_ID });
  return ajv;
}

function pointer(...parts: string[]): string {
  return parts.map((part) => part.replaceAll("~", "~0").replaceAll("/", "~1")).join("/");
}

function responseSchema(path: string, method: string, status: string): string {
  return pointer(
    "paths",
    path,
    method,
    "responses",
    status,
    "content",
    "application/json",
    "schema",
  );
}

describe("mock configuration store", () => {
  const ajv = buildValidator();
  let mock: RunningMockServer;
  const headers = { Authorization: `Bearer ${MOCK_TOKEN}` };

  beforeAll(async () => {
    mock = await startMockServer();
  });
  afterAll(async () => {
    await mock.close();
  });

  function expectValid(ref: string, body: unknown) {
    const validate = ajv.compile({ $ref: `${API_ID}#/${ref}` });
    expect(validate(body), JSON.stringify(validate.errors, null, 2)).toBe(true);
  }

  it("lists the example tests newest first and serves each as a valid configuration", async () => {
    const list = await fetch(`${mock.baseUrl}/configs`, { headers });
    const body = (await list.json()) as { items: { id: string; name: string }[] };
    expectValid(responseSchema("/configs", "get", "200"), body);
    expect(body.items.map((item) => item.name)).toEqual(
      loadMockData()
        .configs.map((config) => config.name)
        .reverse(),
    );
    for (const item of body.items) {
      const one = await fetch(`${mock.baseUrl}/configs/${item.id}`, { headers });
      expect(one.status).toBe(200);
      expectValid(responseSchema("/configs/{id}", "get", "200"), await one.json());
    }
    expect((await fetch(`${mock.baseUrl}/configs/nope`, { headers })).status).toBe(404);
  });

  it("stores a YAML or JSON body and answers 201 with a ConfigRef", async () => {
    const text = readFileSync(join(CONTRACTS_DIR, "examples", "tests", "dns_zone.yaml"), "utf8");
    const yaml = await fetch(`${mock.baseUrl}/configs`, {
      method: "POST",
      headers: { ...headers, "Content-Type": "application/yaml" },
      body: text,
    });
    expect(yaml.status).toBe(201);
    const ref = (await yaml.json()) as { id: string; name: string };
    expectValid(responseSchema("/configs", "post", "201"), ref);
    expect(ref.name).toBe("dns_zone");
    expect(yaml.headers.get("location")).toBe(`/v1/configs/${ref.id}`);
    const json = await fetch(`${mock.baseUrl}/configs`, {
      method: "POST",
      headers: { ...headers, "Content-Type": "application/json" },
      body: JSON.stringify(parse(text)),
    });
    expect(json.status).toBe(201);
    const again = await fetch(`${mock.baseUrl}/configs/${ref.id}`, { headers });
    expect(((await again.json()) as { metadata: { name: string } }).metadata.name).toBe("dns_zone");
  });

  it("answers a body that breaks the schema with 422 and its errors", async () => {
    const response = await fetch(`${mock.baseUrl}/configs`, {
      method: "POST",
      headers: { ...headers, "Content-Type": "application/yaml" },
      body: "apiVersion: fixingpoint.dev/v1alpha1\nkind: Test\nextra: 1\n",
    });
    expect(response.status).toBe(422);
    const body = (await response.json()) as { errors: { path: string }[] };
    expectValid("components/schemas/Problem", body);
    expect(body.errors.map((error) => error.path)).toEqual(
      expect.arrayContaining(["/metadata", "/extra"]),
    );
    const wrongType = await fetch(`${mock.baseUrl}/configs`, {
      method: "POST",
      headers: { ...headers, "Content-Type": "text/plain" },
      body: "x",
    });
    expect(wrongType.status).toBe(415);
  });

  it("validates a stored configuration against the schema and the installed modules", async () => {
    const reportPath = responseSchema("/configs/{id}:validate", "post", "200");
    const good = await fetch(`${mock.baseUrl}/configs/cfg_http_smoke:validate`, {
      method: "POST",
      headers,
    });
    const goodBody = (await good.json()) as { valid: boolean; findings: unknown[] };
    expectValid(reportPath, goodBody);
    expect(goodBody).toMatchObject({ valid: true, findings: [] });
    // dns_zone names the scenario dns/lookup_a, which the example dns module does not ship.
    const bad = await fetch(`${mock.baseUrl}/configs/cfg_dns_zone:validate`, {
      method: "POST",
      headers,
    });
    const badBody = (await bad.json()) as { valid: boolean; findings: { path: string }[] };
    expectValid(reportPath, badBody);
    expect(badBody.valid).toBe(false);
    expect(badBody.findings.map((finding) => finding.path)).toEqual(["/profile/mix/0/scenario"]);
    expect(
      (await fetch(`${mock.baseUrl}/configs/nope:validate`, { method: "POST", headers })).status,
    ).toBe(404);
  });
});
