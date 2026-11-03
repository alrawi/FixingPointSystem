// SPDX-License-Identifier: GPL-3.0-only
//
// Pure helpers that turn the parts of a module manifest into display rows.
// They take the manifest as the contract types describe it and never trust a
// field the schema leaves open, such as the JSON Schema of the parameters.

import type { ModuleFunction, ModuleManifest } from "../api/client";

/** One user configurable parameter of a module, read from its JSON Schema. */
export interface ParameterRow {
  name: string;
  type: string;
  /** The default as JSON text, or null when the schema names none. */
  defaultValue: string | null;
  required: boolean;
  /** Constraints such as `minimum 1`, in the order of the schema keywords below. */
  constraints: string[];
  description: string | null;
}

/** Keywords that become constraint text, with the words we show for them. */
const CONSTRAINT_KEYWORDS: [keyword: string, text: (value: string) => string][] = [
  ["minimum", (value) => `minimum ${value}`],
  ["exclusiveMinimum", (value) => `greater than ${value}`],
  ["maximum", (value) => `maximum ${value}`],
  ["exclusiveMaximum", (value) => `less than ${value}`],
  ["multipleOf", (value) => `multiple of ${value}`],
  ["minLength", (value) => `at least ${value} characters`],
  ["maxLength", (value) => `at most ${value} characters`],
  ["pattern", (value) => `pattern ${value}`],
  ["format", (value) => `format ${value}`],
  ["minItems", (value) => `at least ${value} items`],
  ["maxItems", (value) => `at most ${value} items`],
];

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

/** Formats a JSON value for display, keeping strings unquoted when they are plain words. */
export function formatValue(value: unknown): string {
  if (typeof value === "string") return /^[\w./:@*-]+$/.test(value) ? value : JSON.stringify(value);
  // JSON.stringify answers undefined for undefined, which its types do not show.
  return value === undefined ? "undefined" : JSON.stringify(value);
}

function schemaType(schema: Record<string, unknown>): string {
  const type = schema["type"];
  if (typeof type === "string") return type;
  if (Array.isArray(type)) return type.map(String).join(" or ");
  if ("const" in schema) return "constant";
  if (Array.isArray(schema["enum"])) return "enum";
  return "any";
}

/** Lists the parameters that the manifest's JSON Schema declares, in schema order. */
export function parameterRows(declared: ModuleManifest["parameters"]): ParameterRow[] {
  // The contract types only the root keyword, so the rest is read as unknown JSON.
  const parameters: unknown = declared;
  if (!isRecord(parameters)) return [];
  const properties = parameters["properties"];
  if (!isRecord(properties)) return [];
  const required = new Set(
    Array.isArray(parameters["required"]) ? parameters["required"].map(String) : [],
  );
  return Object.entries(properties).map(([name, value]) => {
    const schema = isRecord(value) ? value : {};
    const constraints: string[] = [];
    if (Array.isArray(schema["enum"])) {
      constraints.push(`one of ${schema["enum"].map(formatValue).join(", ")}`);
    }
    if ("const" in schema) constraints.push(`equal to ${formatValue(schema["const"])}`);
    for (const [keyword, text] of CONSTRAINT_KEYWORDS) {
      if (keyword in schema) constraints.push(text(formatValue(schema[keyword])));
    }
    const description = schema["description"];
    return {
      name,
      type: schemaType(schema),
      defaultValue: "default" in schema ? formatValue(schema["default"]) : null,
      required: required.has(name),
      constraints,
      description: typeof description === "string" ? description : null,
    };
  });
}

/** True when the parameter schema rejects names that it does not list. */
export function closedParameters(declared: ModuleManifest["parameters"]): boolean {
  const parameters: unknown = declared;
  return isRecord(parameters) && parameters["additionalProperties"] === false;
}

/** The call signature of a template function, such as `http1.request(method: text, accept: text = "*\/*")`. */
export function functionSignature(module: string, fn: ModuleFunction): string {
  const params = fn.params.map((param) =>
    param.default === undefined
      ? `${param.name}: ${param.type}`
      : `${param.name}: ${param.type} = ${JSON.stringify(param.default)}`,
  );
  return `${module}.${fn.name}(${params.join(", ")})`;
}

/** The protocols a module provides, such as `http over tcp port 80`. */
export function providesText(provides: ModuleManifest["provides"]): string[] {
  return (provides ?? []).map(
    (entry) =>
      `${entry.protocol} over ${entry.transport}${entry.port === undefined ? "" : ` port ${entry.port}`}`,
  );
}

/** The path of the detail view of a module. */
export function modulePath(name: string, version: string): string {
  return `/modules/${encodeURIComponent(name)}/${encodeURIComponent(version)}`;
}
