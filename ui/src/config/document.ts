// SPDX-License-Identifier: GPL-3.0-only
//
// The text model of the test configuration editor. The YAML text is the single
// source of truth, and the forms read and edit it through a `yaml` Document.
// The Document keeps comments, key order, quoting, and flow or block style of
// every node that an edit does not touch.
//
// Round trip definition (also in docs/wiki/Web-UI.md):
//   1. Text that the user loaded or typed stays byte for byte as it is until a
//      form field changes it. The editor never reserializes on load.
//   2. A form edit parses the text into a Document, changes only the nodes of
//      that edit, and serializes with `SERIALIZE_OPTIONS`. Every other byte of
//      the text stays, so comments, key order, and style survive. For the
//      example tests of the contract this serialization is byte identical even
//      for an edit that sets a field to its current value.
//   3. A configuration that the controller returns is JSON and has no
//      comments. The editor shows it as YAML in the key order of the schema
//      (`canonicalYaml`), and the YAML parses back to an equal value.

import {
  Document,
  isMap,
  isScalar,
  isSeq,
  parseDocument,
  stringify,
  type ToStringOptions,
  type YAMLSeq,
} from "yaml";

/** A path into the configuration, as keys of mappings and indexes of sequences. */
export type Path = readonly (string | number)[];

/**
 * The serialization options. Line folding is off, because the default width of
 * 80 characters would break the long flow mappings of the examples, and a
 * reserialized document would then differ from its source.
 */
export const SERIALIZE_OPTIONS: ToStringOptions = { lineWidth: 0 };

/** One problem found in the text, by the YAML parser, the schema, or the controller. */
export interface ConfigIssue {
  /** JSON Pointer of the field, `""` for the document. */
  pointer: string;
  message: string;
  severity: "error" | "warning" | "info";
  source: "yaml" | "schema" | "controller";
  /** One based position, for syntax errors. */
  line?: number;
  column?: number;
}

/** The result of reading the text. */
export interface Analysis {
  /** The parsed value, or undefined when the text has a syntax error. */
  value: unknown;
  /** Syntax errors of the YAML text. */
  syntaxIssues: ConfigIssue[];
}

/** Builds a JSON Pointer (RFC 6901) from a path. */
export function pointerOf(path: Path): string {
  return path
    .map((part) => `/${String(part).replaceAll("~", "~0").replaceAll("/", "~1")}`)
    .join("");
}

/** Reads the YAML text. A text with any syntax error has no value. */
export function analyze(text: string): Analysis {
  const doc = parseDocument(text);
  const syntaxIssues: ConfigIssue[] = doc.errors.map((error) => ({
    pointer: "",
    message: error.message.split("\n")[0] ?? error.message,
    severity: "error",
    source: "yaml",
    ...(error.linePos?.[0] ? { line: error.linePos[0].line, column: error.linePos[0].col } : {}),
  }));
  if (syntaxIssues.length === 0 && doc.contents === null) {
    syntaxIssues.push({
      pointer: "",
      message: "The configuration is empty.",
      severity: "error",
      source: "yaml",
    });
  } else if (syntaxIssues.length === 0 && !isMap(doc.contents)) {
    syntaxIssues.push({
      pointer: "",
      message: "The configuration must be a mapping of fields.",
      severity: "error",
      source: "yaml",
    });
  }
  return {
    value: syntaxIssues.length === 0 ? (doc.toJS() as unknown) : undefined,
    syntaxIssues,
  };
}

/**
 * Applies one edit to the Document of the text and serializes it. A text with a
 * syntax error comes back unchanged, because there is no Document to edit.
 */
export function editText(text: string, edit: (doc: Document) => void): string {
  const doc = parseDocument(text);
  if (doc.errors.length > 0 || !isMap(doc.contents)) return text;
  edit(doc);
  return doc.toString(SERIALIZE_OPTIONS);
}

/** Sets the value at the path, or removes the key when the value is undefined. */
export function setValue(doc: Document, path: Path, value: unknown): void {
  if (value === undefined) {
    doc.deleteIn(path);
    return;
  }
  // A field that was emptied is written as "". When it gets text again, let the
  // serializer choose the style instead of keeping the quotes of the empty string.
  const node = doc.getIn(path, true);
  if (isScalar(node) && node.value === "" && typeof value === "string") node.type = undefined;
  doc.setIn(path, value);
}

function itemsAt(doc: Document, path: Path): YAMLSeq | undefined {
  const node = doc.getIn(path, true);
  return isSeq(node) ? node : undefined;
}

/**
 * Appends an item to the sequence at the path and creates the sequence when it
 * is missing. A new mapping takes the flow or block style of its siblings, so
 * a list written as `- { name: a, ... }` stays in that style.
 */
export function addItem(doc: Document, path: Path, item: unknown): void {
  const node = doc.createNode(item);
  const seq = itemsAt(doc, path);
  if (seq === undefined) {
    doc.setIn(path, doc.createNode([item]));
    return;
  }
  const sibling = seq.items[0];
  if (isMap(node) && isMap(sibling) && sibling.flow === true) node.flow = true;
  seq.add(node);
}

/** Removes the item at the index from the sequence at the path. */
export function removeItem(doc: Document, path: Path, index: number): void {
  doc.deleteIn([...path, index]);
}

/** Moves an item inside the sequence at the path. Comments stay with the item. */
export function moveItem(doc: Document, path: Path, from: number, to: number): void {
  const seq = itemsAt(doc, path);
  if (seq === undefined || from === to) return;
  if (from < 0 || from >= seq.items.length || to < 0 || to >= seq.items.length) return;
  const [item] = seq.items.splice(from, 1);
  if (item !== undefined) seq.items.splice(to, 0, item);
}

/** Reads the value at the path of a parsed value, for the forms. */
export function readPath(root: unknown, path: Path): unknown {
  let node = root;
  for (const part of path) {
    if (Array.isArray(node)) node = node[Number(part)] as unknown;
    else if (typeof node === "object" && node !== null) {
      node = (node as Record<string, unknown>)[String(part)];
    } else return undefined;
  }
  return node;
}

interface SchemaNode {
  $ref?: string;
  properties?: Record<string, SchemaNode>;
  items?: SchemaNode;
  additionalProperties?: SchemaNode | boolean;
  $defs?: Record<string, SchemaNode>;
}

function resolve(node: SchemaNode | undefined, root: SchemaNode): SchemaNode | undefined {
  if (node?.$ref?.startsWith("#/$defs/") !== true) return node;
  return root.$defs?.[node.$ref.slice("#/$defs/".length)];
}

/** Reorders mapping keys like the `properties` of the schema. Unknown keys follow in their order. */
function orderLike(value: unknown, node: SchemaNode | undefined, root: SchemaNode): unknown {
  const schema = resolve(node, root);
  if (Array.isArray(value)) {
    return value.map((item: unknown) => orderLike(item, schema?.items, root));
  }
  if (typeof value !== "object" || value === null) return value;
  const record = value as Record<string, unknown>;
  const known = Object.keys(schema?.properties ?? {});
  const keys = [
    ...known.filter((key) => key in record),
    ...Object.keys(record).filter((key) => !known.includes(key)),
  ];
  const out: Record<string, unknown> = {};
  for (const key of keys) {
    const child =
      schema?.properties?.[key] ??
      (typeof schema?.additionalProperties === "object" ? schema.additionalProperties : undefined);
    out[key] = orderLike(record[key], child, root);
  }
  return out;
}

/**
 * Serializes a configuration that the controller returned as JSON. The keys
 * follow the order of the schema, and the YAML has no comments because the
 * JSON carries none.
 */
export function canonicalYaml(value: unknown, schema: object): string {
  const ordered = orderLike(value, schema, schema);
  return stringify(ordered, SERIALIZE_OPTIONS);
}

/** The text of a new configuration. It passes the schema, so the forms start without errors. */
export const NEW_CONFIG_TEXT = `# A new test configuration. Edit the forms or this text; both stay in step.
apiVersion: fixingpoint.dev/v1alpha1
kind: Test
metadata:
  name: new_test
  description: Describe what the test shows.
topology: app_sim
network:
  ports:
    - { name: client0, location: local/lo }
    - { name: server0, location: local/lo }
  host_pools:
    - { name: clients, port: client0, role: client, ipv4: 16.0.0.0/24, count: 254 }
    - { name: servers, port: server0, role: server, ipv4: 48.0.0.0/24, count: 1 }
profile:
  basis: sessions
  mix:
    - { scenario: http1/web_get, weight: 1 }
load:
  objective: { type: sessions_per_second, value: 100 }
  phases:
    - { name: steady, duration_s: 30, shape: hold }
timeline: { seed: 1 }
criteria:
  - { metric: sessions.failed_ratio, op: "<", value: 0.01 }
`;
