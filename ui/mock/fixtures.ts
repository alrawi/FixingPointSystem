// SPDX-License-Identifier: GPL-3.0-only
//
// Mock data built from the contract examples, so the mock server changes with
// the contract. Module summaries and manifests come from
// contracts/examples/modules, the agent and its ports from the network section
// of contracts/examples/tests/http_smoke.yaml, and the version strings from the
// `examples` of the Capabilities schema in contracts/openapi/fps-api-v1.yaml.

import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { parse } from "yaml";
import type { Agent, Capabilities, ModuleSummary } from "../src/api/client";

/** The contracts directory of the repository. */
export const CONTRACTS_DIR = join(import.meta.dirname, "..", "..", "contracts");

function readYaml(path: string): unknown {
  return parse(readFileSync(path, "utf8"));
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function str(record: Record<string, unknown>, key: string): string {
  const value = record[key];
  if (typeof value !== "string") throw new Error(`fixture field ${key} is not a string`);
  return value;
}

/** A module manifest from the examples, with the path it came from. */
export interface ExampleModule {
  manifest: Record<string, unknown>;
  summary: ModuleSummary;
}

/** Reads every example module manifest and its scenario names. */
export function loadExampleModules(contractsDir = CONTRACTS_DIR): ExampleModule[] {
  const root = join(contractsDir, "examples", "modules");
  return readdirSync(root, { withFileTypes: true })
    .filter((entry) => entry.isDirectory())
    .map((entry) => entry.name)
    .sort()
    .map((dir) => {
      const manifest = readYaml(join(root, dir, "manifest.yaml"));
      if (!isRecord(manifest)) throw new Error(`${dir}/manifest.yaml is not a mapping`);
      const files = Array.isArray(manifest["scenarios"]) ? manifest["scenarios"] : [];
      const scenarios = files.map((file) => {
        const scenario = readYaml(join(root, dir, String(file)));
        return isRecord(scenario) ? str(scenario, "scenario") : String(file);
      });
      const summary: ModuleSummary = {
        name: str(manifest, "name"),
        version: str(manifest, "version"),
        type: str(manifest, "type") as ModuleSummary["type"],
        license: str(manifest, "license"),
        summary: str(manifest, "summary"),
        scenarios,
      };
      return { manifest, summary };
    });
}

/** Builds one online agent that owns the ports of the http_smoke example test. */
export function loadExampleAgents(contractsDir = CONTRACTS_DIR): Agent[] {
  const test = readYaml(join(contractsDir, "examples", "tests", "http_smoke.yaml"));
  const network = isRecord(test) && isRecord(test["network"]) ? test["network"] : {};
  const ports = Array.isArray(network["ports"]) ? network["ports"] : [];
  return [
    {
      id: "agent-local",
      address: "127.0.0.1:50051",
      state: "online",
      version: "0.1.0",
      cores: 2,
      backend: "kernel",
      ports: ports.filter(isRecord).map((port) => {
        const location = str(port, "location");
        return { name: str(port, "name"), driver: location.split("/").pop() ?? location };
      }),
      labels: { site: "local" },
    },
  ];
}

/** The first example value of a property of the Capabilities schema. */
function capabilitiesExample(contractsDir: string, property: string): string {
  const api = readYaml(join(contractsDir, "openapi", "fps-api-v1.yaml"));
  const schemas = isRecord(api) && isRecord(api["components"]) ? api["components"]["schemas"] : {};
  const caps =
    isRecord(schemas) && isRecord(schemas["Capabilities"]) ? schemas["Capabilities"] : {};
  const props = isRecord(caps["properties"]) ? caps["properties"] : {};
  const prop = props[property];
  const examples = isRecord(prop) ? prop["examples"] : undefined;
  const first: unknown = Array.isArray(examples) ? examples[0] : undefined;
  if (typeof first !== "string") throw new Error(`Capabilities.${property} has no example`);
  return first;
}

/** The complete data set that the mock server serves. */
export interface MockData {
  capabilities: Capabilities;
  modules: ExampleModule[];
  agents: Agent[];
}

/** Loads the mock data from the contract examples. */
export function loadMockData(contractsDir = CONTRACTS_DIR): MockData {
  const modules = loadExampleModules(contractsDir);
  const agents = loadExampleAgents(contractsDir);
  return {
    modules,
    agents,
    capabilities: {
      engine_api: capabilitiesExample(contractsDir, "engine_api"),
      api_version: capabilitiesExample(contractsDir, "api_version"),
      backends: ["kernel"],
      agents,
      modules: modules.map((module) => module.summary),
    },
  };
}
