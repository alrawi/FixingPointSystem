// SPDX-License-Identifier: GPL-3.0-only
//
// Starts the mock server: `npm run mock`. FPS_MOCK_PORT sets the port, 8090 by
// default, which is where the Vite dev and preview servers send /v1.

import { MOCK_TOKEN, startMockServer } from "./server";

const port = Number(process.env["FPS_MOCK_PORT"] ?? "8090");
const running = await startMockServer(port);
console.log(`mock controller API at ${running.baseUrl}, token ${MOCK_TOKEN}`);
