// SPDX-License-Identifier: GPL-3.0-only
//
// Fails when the initial JavaScript of the build passes the budget of decision
// D11. Initial JavaScript is every script and module preload that dist/index.html
// loads, measured after gzip at level 9. Run it after `npm run build`.

import { readFileSync } from "node:fs";
import { join } from "node:path";
import { gzipSync } from "node:zlib";

export const BUDGET_BYTES = 250 * 1000;

/** Returns the JavaScript files that an HTML page loads before any user action. */
export function initialScripts(html) {
  const files = new Set();
  const pattern =
    /<(?:script[^>]*\ssrc|link[^>]*\srel="modulepreload"[^>]*\shref)="([^"]+\.js)"|<link[^>]*\shref="([^"]+\.js)"[^>]*\srel="modulepreload"/g;
  for (const match of html.matchAll(pattern)) files.add(match[1] ?? match[2]);
  return [...files];
}

function main() {
  const dist = join(import.meta.dirname, "..", "dist");
  const html = readFileSync(join(dist, "index.html"), "utf8");
  const scripts = initialScripts(html);
  if (scripts.length === 0) {
    console.error("dist/index.html loads no script. Run `npm run build` first.");
    process.exit(1);
  }
  let total = 0;
  for (const src of scripts) {
    const size = gzipSync(readFileSync(join(dist, src.replace(/^\//, ""))), { level: 9 }).length;
    total += size;
    console.log(`${(size / 1000).toFixed(1).padStart(8)} kB  ${src}`);
  }
  const verdict = total <= BUDGET_BYTES ? "within" : "over";
  console.log(
    `${(total / 1000).toFixed(1).padStart(8)} kB  initial JavaScript, gzip, ${verdict} the budget of ${BUDGET_BYTES / 1000} kB`,
  );
  if (total > BUDGET_BYTES) process.exit(1);
}

if (process.argv[1] === import.meta.filename) main();
