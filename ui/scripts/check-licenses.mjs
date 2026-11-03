// SPDX-License-Identifier: GPL-3.0-only
//
// Fails when a package in package-lock.json carries a license that is not on
// the allow list below. Every listed license is compatible with GPL-3.0. The
// check reads the license field that npm records in the lock file and, when
// node_modules exists, compares it with the installed package.json.
//
//   node scripts/check-licenses.mjs           check
//   node scripts/check-licenses.mjs --list    also print the count per license

import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";

/** SPDX identifiers that we accept, with the reason for the less common ones. */
export const ALLOWED = new Map([
  ["MIT", "FSF lists Expat as compatible"],
  ["MIT-0", "MIT without the attribution condition"],
  ["ISC", "FSF lists ISC as compatible"],
  ["0BSD", "Public domain equivalent permissive license"],
  ["BSD-2-Clause", "FSF lists FreeBSD as compatible"],
  ["BSD-3-Clause", "FSF lists Modified BSD as compatible"],
  ["Apache-2.0", "FSF lists Apache 2.0 as compatible with GPL-3.0"],
  ["MPL-2.0", "FSF lists MPL 2.0 as compatible through its secondary license clause"],
  ["BlueOak-1.0.0", "Permissive license that its authors state is GPL compatible"],
  ["CC0-1.0", "FSF lists CC0 as compatible"],
  ["CC-BY-4.0", "FSF lists CC BY 4.0 as compatible with GPL-3.0; used only by caniuse-lite data"],
  ["Python-2.0", "FSF lists the Python license 2.0.1 and later as compatible"],
]);

/**
 * Evaluates an SPDX expression such as `MIT`, `(MIT OR CC0-1.0)`, or
 * `Apache-2.0 AND MIT` against the allow list.
 */
export function isAllowed(expression) {
  const tokens = expression
    .replace(/[()]/g, (paren) => ` ${paren} `)
    .trim()
    .split(/\s+/);
  let pos = 0;
  function primary() {
    const token = tokens[pos++];
    if (token === "(") {
      const value = or();
      if (tokens[pos++] !== ")") throw new Error(`unbalanced license expression: ${expression}`);
      return value;
    }
    if (token === undefined) throw new Error(`incomplete license expression: ${expression}`);
    return ALLOWED.has(token.replace(/\+$/, ""));
  }
  function and() {
    let value = primary();
    while (tokens[pos] === "AND") {
      pos++;
      value = primary() && value;
    }
    return value;
  }
  function or() {
    let value = and();
    while (tokens[pos] === "OR") {
      pos++;
      value = and() || value;
    }
    return value;
  }
  const result = or();
  if (pos !== tokens.length)
    throw new Error(`unexpected token in license expression: ${expression}`);
  return result;
}

function main() {
  const root = join(import.meta.dirname, "..");
  const lock = JSON.parse(readFileSync(join(root, "package-lock.json"), "utf8"));
  const problems = [];
  const counts = new Map();
  for (const [path, entry] of Object.entries(lock.packages ?? {})) {
    if (path === "" || entry.link) continue;
    let license = entry.license;
    const manifest = join(root, path, "package.json");
    if (existsSync(manifest)) {
      const installed = JSON.parse(readFileSync(manifest, "utf8")).license;
      if (typeof installed === "string" && installed !== license) {
        problems.push(`${path}: lock file says ${license}, package.json says ${installed}`);
        license = installed;
      }
    }
    if (typeof license !== "string" || license === "") {
      problems.push(`${path}: no license recorded`);
      continue;
    }
    counts.set(license, (counts.get(license) ?? 0) + 1);
    let ok;
    try {
      ok = isAllowed(license);
    } catch (error) {
      problems.push(`${path}: ${error.message}`);
      continue;
    }
    if (!ok) problems.push(`${path}: ${license} is not on the GPL-3.0 compatible allow list`);
  }
  if (process.argv.includes("--list")) {
    for (const [license, count] of [...counts].sort((a, b) => b[1] - a[1])) {
      console.log(`${String(count).padStart(5)}  ${license}`);
    }
  }
  if (problems.length > 0) {
    console.error(problems.join("\n"));
    process.exit(1);
  }
  const total = [...counts.values()].reduce((sum, count) => sum + count, 0);
  console.log(`${total} packages carry a GPL-3.0 compatible license.`);
}

if (process.argv[1] === import.meta.filename) main();
