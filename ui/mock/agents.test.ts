// SPDX-License-Identifier: GPL-3.0-only
//
// Contract tests of the agent operations of the mock server: every response validates against
// the response schema of its operation in fps-api-v1.yaml, errors are RFC 9457 problems, and a
// reservation changes the state that the agent list and the capabilities report.
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
  expect(validate(body), JSON.stringify(validate.errors, null, 2)).toBe(true);
}

const headers = { Authorization: `Bearer ${MOCK_TOKEN}` };

describe("mock agent reservations", () => {
  let mock: RunningMockServer;
  const ajv = buildValidator();

  beforeAll(async () => {
    mock = await startMockServer();
  });
  afterAll(async () => {
    await mock.close();
  });

  async function post(path: string) {
    const response = await fetch(`${mock.baseUrl}${path}`, { method: "POST", headers });
    return { response, body: (await response.json()) as unknown };
  }

  async function stateOf(id: string): Promise<string | undefined> {
    const list = (await (await fetch(`${mock.baseUrl}/agents`, { headers })).json()) as {
      id: string;
      state: string;
    }[];
    return list.find((agent) => agent.id === id)?.state;
  }

  it("reserves and releases an agent with answers that match the contract", async () => {
    const reserved = await post("/agents/agent-local:reserve");
    expect(reserved.response.status).toBe(200);
    expectValid(
      ajv,
      pointer(
        "paths",
        "/agents/{id}:reserve",
        "post",
        "responses",
        "200",
        "content",
        "application/json",
        "schema",
      ),
      reserved.body,
    );
    expect((reserved.body as { state: string }).state).toBe("reserved");
    expect(await stateOf("agent-local")).toBe("reserved");
    const capabilities = (await (
      await fetch(`${mock.baseUrl}/capabilities`, { headers })
    ).json()) as {
      agents: { id: string; state: string }[];
    };
    expect(capabilities.agents[0]?.state).toBe("reserved");

    const released = await post("/agents/agent-local:release");
    expect(released.response.status).toBe(200);
    expectValid(
      ajv,
      pointer(
        "paths",
        "/agents/{id}:release",
        "post",
        "responses",
        "200",
        "content",
        "application/json",
        "schema",
      ),
      released.body,
    );
    expect((released.body as { state: string }).state).toBe("online");
  });

  it("reports a conflict as a problem when the agent is offline or busy", async () => {
    const own = await startMockServer(0, "127.0.0.1", {
      data: {
        ...loadMockData(),
        agents: [
          { id: "agent-down", state: "offline" },
          { id: "agent-busy", address: "10.0.0.8:50051", state: "busy" },
        ],
      },
    });
    try {
      const expected = {
        "agent-down": "reserve agent agent-down: agent offline",
        "agent-busy": "reserve agent agent-busy: plan loaded: agent busy",
      };
      for (const [id, detail] of Object.entries(expected)) {
        const response = await fetch(`${own.baseUrl}/agents/${id}:reserve`, {
          method: "POST",
          headers,
        });
        const body: unknown = await response.json();
        expect(response.status).toBe(409);
        expect(response.headers.get("content-type")).toBe("application/problem+json");
        expectValid(ajv, "components/schemas/Problem", body);
        expect((body as { detail: string }).detail).toBe(detail);
      }
    } finally {
      await own.close();
    }
  });

  it("records the operator as the holder and clears it on release, with valid agents", async () => {
    const own = await startMockServer(0, "127.0.0.1");
    try {
      const call = async (action: string) => {
        const response = await fetch(`${own.baseUrl}/agents/agent-local:${action}`, {
          method: "POST",
          headers,
        });
        return { status: response.status, body: (await response.json()) as unknown };
      };
      const list = async () =>
        (await (await fetch(`${own.baseUrl}/agents`, { headers })).json()) as unknown;
      const agentSchema = "components/schemas/Agent";

      expect(((await list()) as { reserved_by?: string }[])[0]?.reserved_by).toBeUndefined();
      const reserved = await call("reserve");
      expectValid(ajv, agentSchema, reserved.body);
      expect((reserved.body as { reserved_by?: string }).reserved_by).toBe("operator");
      // The operator reserves again for 200, and the holder stays the operator.
      const again = await call("reserve");
      expect(again.status).toBe(200);
      expect((again.body as { reserved_by?: string }).reserved_by).toBe("operator");
      expectValid(ajv, agentSchema, ((await list()) as unknown[])[0]);

      const released = await call("release");
      expectValid(ajv, agentSchema, released.body);
      expect((released.body as { reserved_by?: string }).reserved_by).toBeUndefined();
    } finally {
      await own.close();
    }
  });

  it("refuses with 409 in the documented order: another holder, offline, busy", async () => {
    const own = await startMockServer(0, "127.0.0.1", {
      data: {
        ...loadMockData(),
        agents: [
          { id: "held-run", state: "reserved", reserved_by: "run_mock0007" },
          { id: "held-run-busy", state: "busy", reserved_by: "run_mock0008" },
          { id: "held-run-down", state: "offline", reserved_by: "run_mock0009" },
          { id: "held-op-down", state: "offline", reserved_by: "operator" },
          { id: "held-op-busy", state: "busy", reserved_by: "operator" },
        ],
      },
    });
    try {
      const expected: Record<string, [number, string]> = {
        "held-run": [
          409,
          "reserve agent held-run for operator: held by run_mock0007: agent reserved by another holder",
        ],
        "held-run-busy": [
          409,
          "reserve agent held-run-busy for operator: held by run_mock0008: agent reserved by another holder",
        ],
        "held-run-down": [
          409,
          "reserve agent held-run-down for operator: held by run_mock0009: agent reserved by another holder",
        ],
        "held-op-down": [409, "reserve agent held-op-down: agent offline"],
        "held-op-busy": [409, "reserve agent held-op-busy: plan loaded: agent busy"],
      };
      for (const [id, [status, detail]] of Object.entries(expected)) {
        const response = await fetch(`${own.baseUrl}/agents/${id}:reserve`, {
          method: "POST",
          headers,
        });
        const body: unknown = await response.json();
        expect(response.status).toBe(status);
        expect(response.headers.get("content-type")).toBe("application/problem+json");
        expectValid(ajv, "components/schemas/Problem", body);
        expect((body as { detail: string }).detail).toBe(detail);
      }
      // A refusal leaves the agent as it was.
      const list = (await (await fetch(`${own.baseUrl}/agents`, { headers })).json()) as {
        id: string;
        state: string;
        reserved_by?: string;
      }[];
      expect(list.map((agent) => [agent.id, agent.state, agent.reserved_by])).toEqual([
        ["held-run", "reserved", "run_mock0007"],
        ["held-run-busy", "busy", "run_mock0008"],
        ["held-run-down", "offline", "run_mock0009"],
        ["held-op-down", "offline", "operator"],
        ["held-op-busy", "busy", "operator"],
      ]);
      for (const agent of list) expectValid(ajv, "components/schemas/Agent", agent);
    } finally {
      await own.close();
    }
  });

  it("reserves a reserved agent again and releases an agent that is not reserved", async () => {
    const own = await startMockServer(0, "127.0.0.1");
    try {
      const call = async (action: string) => {
        const response = await fetch(`${own.baseUrl}/agents/agent-local:${action}`, {
          method: "POST",
          headers,
        });
        return { status: response.status, body: (await response.json()) as { state: string } };
      };
      expect((await call("release")).body.state).toBe("online");
      expect((await call("reserve")).body.state).toBe("reserved");
      expect((await call("reserve")).body.state).toBe("reserved");
      expect((await call("release")).body.state).toBe("online");
      for (const agent of own.agents) agent.state = "busy";
      const busy = await call("release");
      expect(busy.status).toBe(200);
      expect(busy.body.state).toBe("busy");
    } finally {
      await own.close();
    }
  });

  it("answers an unknown agent with 404", async () => {
    for (const operation of ["reserve", "release"]) {
      const { response, body } = await post(`/agents/agent-none:${operation}`);
      expect(response.status).toBe(404);
      expectValid(ajv, "components/schemas/Problem", body);
      expect((body as { detail: string }).detail).toBe(
        `${operation} agent agent-none: unknown agent`,
      );
    }
  });

  it("keeps the data that it was given unchanged", async () => {
    const data = loadMockData();
    const own = await startMockServer(0, "127.0.0.1", { data });
    try {
      const response = await fetch(`${own.baseUrl}/agents/agent-local:reserve`, {
        method: "POST",
        headers,
      });
      expect(response.status).toBe(200);
      expect(data.agents[0]?.state).toBe("online");
    } finally {
      await own.close();
    }
  });
});
