// SPDX-License-Identifier: GPL-3.0-only
//
// The acceptance test of UI-03: every example test of the contract passes
// through the editor model and serializer without edits and comes out equal.
// The round trip definition is in document.ts. Text is byte identical, and a
// configuration that came from the controller as JSON is equal as a value.
// @vitest-environment node

import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { parse, parseDocument } from "yaml";
import { describe, expect, it } from "vitest";
import { CONTRACTS_DIR } from "../../mock/fixtures";
import {
  addItem,
  analyze,
  canonicalYaml,
  editText,
  moveItem,
  pointerOf,
  removeItem,
  setValue,
  SERIALIZE_OPTIONS,
  type Path,
} from "./document";
import { checkConfig, TEST_CONFIG_SCHEMA } from "./schema";

const EXAMPLES_DIR = join(CONTRACTS_DIR, "examples", "tests");
const files = readdirSync(EXAMPLES_DIR)
  .filter((name) => name.endsWith(".yaml"))
  .sort();
const examples = files.map((name) => ({
  name,
  text: readFileSync(join(EXAMPLES_DIR, name), "utf8"),
}));

/** Every path to a scalar of a parsed value. */
function leafPaths(value: unknown, prefix: Path = []): Path[] {
  if (Array.isArray(value)) {
    return value.flatMap((item, index) => leafPaths(item, [...prefix, index]));
  }
  if (typeof value === "object" && value !== null) {
    return Object.entries(value).flatMap(([key, child]) => leafPaths(child, [...prefix, key]));
  }
  return [prefix];
}

function valueAt(root: unknown, path: Path): unknown {
  return path.reduce<unknown>(
    (node, part) => (node as Record<string | number, unknown>)[part],
    root,
  );
}

describe("example tests of the contract", () => {
  it("finds the examples", () => {
    expect(files.length).toBeGreaterThanOrEqual(6);
  });

  describe.each(examples)("$name", ({ text }) => {
    it("parses and passes the schema in the browser", () => {
      const analysis = analyze(text);
      expect(analysis.syntaxIssues).toEqual([]);
      expect(checkConfig(analysis.value)).toEqual([]);
    });

    it("reads as the same value as a plain YAML parse", () => {
      expect(analyze(text).value).toEqual(parse(text));
    });

    it("serializes byte for byte after a parse without edits", () => {
      expect(parseDocument(text).toString(SERIALIZE_OPTIONS)).toBe(text);
    });

    it("keeps every byte when an edit sets every field to its current value", () => {
      const value = parse(text) as unknown;
      let current = text;
      for (const path of leafPaths(value)) {
        current = editText(current, (doc) => {
          setValue(doc, path, valueAt(value, path));
        });
      }
      expect(current).toBe(text);
    });

    it("keeps comments and the rest of the text when one field changes", () => {
      const edited = editText(text, (doc) => {
        setValue(doc, ["metadata", "name"], "renamed_by_test");
      });
      const before = text.split("\n");
      const after = edited.split("\n");
      expect(after).toHaveLength(before.length);
      const changed = after.flatMap((line, index) => (line === before[index] ? [] : [index]));
      expect(changed).toHaveLength(1);
      expect(after[changed[0] ?? 0]).toMatch(/^ {2}name: renamed_by_test$/);
      // Comment lines are among the unchanged lines.
      expect(after.filter((line) => line.startsWith("#"))).toEqual(
        before.filter((line) => line.startsWith("#")),
      );
    });

    it("round trips as a value through the YAML that a stored JSON copy produces", () => {
      // The controller stores the configuration as JSON and returns it as JSON.
      const json = JSON.parse(JSON.stringify(parse(text))) as unknown;
      const shown = canonicalYaml(json, TEST_CONFIG_SCHEMA);
      expect(parse(shown)).toEqual(json);
      expect(checkConfig(parse(shown))).toEqual([]);
      // And the canonical text is stable, so loading it again changes nothing.
      expect(parseDocument(shown).toString(SERIALIZE_OPTIONS)).toBe(shown);
    });
  });
});

describe("document edits", () => {
  const base = `# keep me
apiVersion: fixingpoint.dev/v1alpha1
kind: Test
metadata:
  name: t # trailing comment
topology: app_sim
network:
  ports:
    - { name: a, location: x/y }
    - { name: b, location: x/z }
  host_pools: []
`;

  it("removes an optional key without touching its neighbours", () => {
    const withMtu = editText(base, (doc) => {
      setValue(doc, ["network", "ports", 0, "mtu"], 1500);
    });
    expect(withMtu).toContain("{ name: a, location: x/y, mtu: 1500 }");
    const without = editText(withMtu, (doc) => {
      setValue(doc, ["network", "ports", 0, "mtu"], undefined);
    });
    expect(without).toBe(base);
  });

  it("adds an item in the style of its siblings and creates a missing list", () => {
    const added = editText(base, (doc) => {
      addItem(doc, ["network", "ports"], { name: "c", location: "x/w" });
      addItem(doc, ["criteria"], { metric: "sessions.failed_ratio", op: ">", value: 1 });
    });
    expect(added).toContain("    - { name: c, location: x/w }\n");
    expect(added).toContain('op: ">"');
    expect(analyze(added).syntaxIssues).toEqual([]);
    expect((analyze(added).value as { criteria: unknown[] }).criteria).toHaveLength(1);
  });

  it("removes and moves items, and a comment moves with its item", () => {
    const text = "list:\n  - a # first\n  - b\n  - c\n";
    const moved = editText(text, (doc) => {
      moveItem(doc, ["list"], 0, 2);
    });
    expect(moved).toBe("list:\n  - b\n  - c\n  - a # first\n");
    const removed = editText(moved, (doc) => {
      removeItem(doc, ["list"], 1);
    });
    expect(removed).toBe("list:\n  - b\n  - a # first\n");
  });

  it("leaves a text with a syntax error unchanged", () => {
    const broken = "a: [1, 2\n";
    expect(
      editText(broken, (doc) => {
        setValue(doc, ["a"], 1);
      }),
    ).toBe(broken);
    expect(analyze(broken).syntaxIssues.length).toBeGreaterThan(0);
    expect(analyze(broken).value).toBeUndefined();
  });

  it("reports an empty text, a list at the top, a duplicate key, and two documents", () => {
    for (const text of ["", "- a\n", "a: 1\na: 2\n", "a: 1\n---\nb: 2\n"]) {
      expect(analyze(text).syntaxIssues.length, JSON.stringify(text)).toBeGreaterThan(0);
    }
  });

  it("builds JSON Pointers with the escapes of RFC 6901", () => {
    expect(pointerOf(["a/b", "c~d", 3])).toBe("/a~1b/c~0d/3");
    expect(pointerOf([])).toBe("");
  });

  it("puts keys of a generated document in the order of the schema", () => {
    const shown = canonicalYaml(
      { load: { phases: [], objective: { value: 1, type: "throughput_bps" } }, kind: "Test" },
      TEST_CONFIG_SCHEMA,
    );
    expect(shown.indexOf("kind:")).toBeLessThan(shown.indexOf("load:"));
    expect(shown.indexOf("objective:")).toBeLessThan(shown.indexOf("phases:"));
    expect(shown.indexOf("type:")).toBeLessThan(shown.indexOf("value:"));
  });
});
