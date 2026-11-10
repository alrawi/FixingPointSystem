// SPDX-License-Identifier: GPL-3.0-only
//
// Records the metrics stream of a mock run as a fixture: `npm run record:stream`.
// The script starts the mock server, starts a run of the http_smoke example,
// reads GET /runs/{id}/metrics:stream until the end event, and writes the raw
// response body. The component tests replay that file. The recording repeats
// until every tick appears exactly once, because a timer that runs late could
// otherwise repeat a tick.

import { mkdirSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { MOCK_TOKEN, startMockServer } from "./server";

const OUT = join(import.meta.dirname, "..", "src", "runs", "fixtures", "run-stream.sse");
const TICKS = 10;

async function record(): Promise<string> {
  const mock = await startMockServer(0, "127.0.0.1", {
    runs: { stepMs: 100, runTicks: TICKS },
  });
  try {
    const headers = { Authorization: `Bearer ${MOCK_TOKEN}`, "Content-Type": "application/json" };
    const started = await fetch(`${mock.baseUrl}/runs`, {
      method: "POST",
      headers,
      body: JSON.stringify({ config_id: "cfg_http_smoke" }),
    });
    const run = (await started.json()) as { id: string };
    const response = await fetch(`${mock.baseUrl}/runs/${run.id}/metrics:stream?view=application`, {
      headers: { Authorization: `Bearer ${MOCK_TOKEN}`, Accept: "text/event-stream" },
    });
    return await response.text();
  } finally {
    await mock.close();
  }
}

/** True when the body holds the ticks 1 to TICKS once each and ends with the end event. */
function isClean(body: string): boolean {
  const stamps = [...body.matchAll(/"ts":"([^"]+)"/g)].map((match) => match[1]);
  const expected = Array.from({ length: TICKS }, (_, index) =>
    new Date(Date.UTC(2026, 10, 1, 10, 0, index + 1)).toISOString(),
  );
  return (
    JSON.stringify(stamps) === JSON.stringify(expected) &&
    body.endsWith('event: end\ndata: {"state":"completed"}\n\n')
  );
}

for (let attempt = 1; attempt <= 10; attempt += 1) {
  const body = await record();
  if (isClean(body)) {
    mkdirSync(dirname(OUT), { recursive: true });
    writeFileSync(OUT, body);
    console.log(`wrote ${OUT} (${String(body.length)} bytes)`);
    process.exit(0);
  }
}
console.error("no clean recording after 10 attempts");
process.exit(1);
