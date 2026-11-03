// SPDX-License-Identifier: GPL-3.0-only
//
// Contract tests of the mock server: every response it serves validates
// against the response schema of the operation in fps-api-v1.yaml.
// @vitest-environment node

import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { Ajv2020 } from "ajv/dist/2020.js";
import addFormatsModule from "ajv-formats";
import { afterAll, beforeAll, describe, expect, it } from "vitest";
import { parse } from "yaml";
import { CONTRACTS_DIR, loadMockData } from "./fixtures";
import { MOCK_TOKEN, startMockServer, type RunningMockServer } from "./server";

const API_ID = "https://github.com/alrawi/FixingPointSystem/contracts/openapi/fps-api-v1.yaml";

// ajv-formats ships CommonJS, so the default import may carry the function on .default.
const addFormats = ((addFormatsModule as unknown as { default?: unknown }).default ??
  addFormatsModule) as unknown as (ajv: Ajv2020) => void;

function buildValidator(): Ajv2020 {
  const ajv = new Ajv2020({ strict: false, validateSchema: false, allErrors: true });
  addFormats(ajv);
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

function expectValid(ajv: Ajv2020, ref: string, body: unknown) {
  const validate = ajv.compile({ $ref: `${API_ID}#/${ref}` });
  const ok = validate(body);
  expect(ok, JSON.stringify(validate.errors, null, 2)).toBe(true);
}

describe("mock server", () => {
  let mock: RunningMockServer;
  const ajv = buildValidator();

  beforeAll(async () => {
    mock = await startMockServer();
  });
  afterAll(async () => {
    await mock.close();
  });

  async function get(path: string, token: string | null = MOCK_TOKEN) {
    const headers: Record<string, string> =
      token === null ? {} : { Authorization: `Bearer ${token}` };
    const response = await fetch(`${mock.baseUrl}${path}`, { headers });
    return { response, body: (await response.json()) as unknown };
  }

  it.each([
    ["/capabilities", "/capabilities"],
    ["/modules", "/modules"],
    ["/modules?type=app", "/modules"],
    ["/modules/http1/0.1.0", "/modules/{name}/{version}"],
    ["/modules/dns/0.1.0", "/modules/{name}/{version}"],
    ["/agents", "/agents"],
    ["/runs", "/runs"],
  ])("serves %s as the contract describes", async (url, path) => {
    const { response, body } = await get(url);
    expect(response.status).toBe(200);
    expect(response.headers.get("content-type")).toBe("application/json");
    expectValid(
      ajv,
      pointer("paths", path, "get", "responses", "200", "content", "application/json", "schema"),
      body,
    );
  });

  it("rejects a missing or unknown token with a problem", async () => {
    for (const token of [null, "wrong"]) {
      const { response, body } = await get("/capabilities", token);
      expect(response.status).toBe(401);
      expect(response.headers.get("content-type")).toBe("application/problem+json");
      // Every operation answers errors with components/responses/Problem.
      expectValid(ajv, "components/schemas/Problem", body);
    }
  });

  it("answers an unknown path with 404 and an unserved method with 405", async () => {
    expect((await get("/nothing")).response.status).toBe(404);
    expect((await get("/modules/nope/1.0.0")).response.status).toBe(404);
    const response = await fetch(`${mock.baseUrl}/capabilities`, {
      method: "POST",
      headers: { Authorization: `Bearer ${MOCK_TOKEN}` },
    });
    expect(response.status).toBe(405);
  });
});

describe("mock data", () => {
  it("lists every example module with its scenarios", () => {
    const data = loadMockData();
    const dirs = readdirSync(join(CONTRACTS_DIR, "examples", "modules")).sort();
    expect(data.capabilities.modules.map((module) => module.name)).toEqual(dirs);
    const http1 = data.capabilities.modules.find((module) => module.name === "http1");
    expect(http1?.scenarios).toContain("web_get");
  });

  it("gives the agent the ports of the http_smoke example", () => {
    const ports = loadMockData().agents[0]?.ports?.map((port) => port.name);
    expect(ports).toEqual(["client0", "server0"]);
  });
});
