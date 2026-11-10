// SPDX-License-Identifier: GPL-3.0-only
//
// Checks a configuration against the JSON Schema of the contract in the
// browser. The schema is the file that the controller validates with, bundled
// at build time, so the browser and the controller cannot disagree on shape.
// The controller still checks module references and agent capacity, which the
// schema cannot express.

import { Ajv2020, type ErrorObject, type ValidateFunction } from "ajv/dist/2020.js";
import testConfigSchema from "../../../contracts/schemas/test-config.schema.json";
import type { ConfigIssue } from "./document";

/** The schema of the contract, for the key order of generated YAML. */
export const TEST_CONFIG_SCHEMA: object = testConfigSchema;

let compiled: ValidateFunction | undefined;

function validator(): ValidateFunction {
  // Compile on first use, so views that never edit a configuration pay nothing.
  compiled ??= new Ajv2020({
    allErrors: true,
    // verbose keeps the schema of each failure, which the anyOf message reads.
    verbose: true,
    strict: false,
  }).compile(TEST_CONFIG_SCHEMA);
  return compiled;
}

function describe(error: ErrorObject): { pointer: string; message: string } {
  const params = error.params as Record<string, unknown>;
  const at = (key: string) => `${error.instancePath}/${String(params[key]).replaceAll("/", "~1")}`;
  switch (error.keyword) {
    case "required":
      return { pointer: at("missingProperty"), message: "is required" };
    case "additionalProperties":
      return { pointer: at("additionalProperty"), message: "is not a field of this section" };
    case "enum":
      return {
        pointer: error.instancePath,
        message: `must be one of ${(params["allowedValues"] as unknown[]).map(String).join(", ")}`,
      };
    case "const":
      return { pointer: error.instancePath, message: `must be ${String(params["allowedValue"])}` };
    case "type":
      return { pointer: error.instancePath, message: `must be ${String(params["type"])}` };
    case "pattern":
      return {
        pointer: error.instancePath,
        message: `has the wrong format, which the pattern ${String(params["pattern"])} describes`,
      };
    case "minItems":
      return {
        pointer: error.instancePath,
        message: `needs at least ${String(params["limit"])} item${params["limit"] === 1 ? "" : "s"}`,
      };
    case "anyOf": {
      const branches = Array.isArray(error.schema)
        ? (error.schema as { required?: string[] }[])
        : [];
      const names = branches.flatMap((branch) => branch.required ?? []);
      return {
        pointer: error.instancePath,
        message:
          names.length > 0
            ? `needs at least one of ${names.join(", ")}`
            : "matches no allowed shape",
      };
    }
    default:
      return { pointer: error.instancePath, message: error.message ?? "is not valid" };
  }
}

/** Checks a parsed configuration against the schema and returns one issue per problem. */
export function checkConfig(value: unknown): ConfigIssue[] {
  const validate = validator();
  if (validate(value)) return [];
  const seen = new Set<string>();
  const issues: ConfigIssue[] = [];
  for (const error of validate.errors ?? []) {
    // The branches of an anyOf report their own failures. The anyOf error says it better.
    if (error.schemaPath.includes("/anyOf/")) continue;
    const { pointer, message } = describe(error);
    const key = `${pointer}\u0000${message}`;
    if (seen.has(key)) continue;
    seen.add(key);
    issues.push({ pointer, message, severity: "error", source: "schema" });
  }
  return issues;
}
