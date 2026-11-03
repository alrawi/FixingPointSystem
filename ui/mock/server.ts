// SPDX-License-Identifier: GPL-3.0-only
//
// A mock of the controller REST API for UI tests and local development. It
// serves the read operations that the UI uses so far from the contract
// examples, checks the bearer token, and answers everything else with an RFC
// 9457 problem. It needs only node:http, so it adds no dependency.

import { createServer, type IncomingMessage, type Server, type ServerResponse } from "node:http";
import type { AddressInfo } from "node:net";
import type { Problem } from "../src/api/client";
import { loadMockData, type MockData } from "./fixtures";

/** The token that the mock server accepts unless the options name others. */
export const MOCK_TOKEN = "fps-mock-token";

export interface MockServerOptions {
  /** Tokens that the server accepts. */
  tokens?: string[];
  data?: MockData;
}

/** A running mock server. */
export interface RunningMockServer {
  server: Server;
  /** The API base URL, ending in /v1. */
  baseUrl: string;
  close: () => Promise<void>;
}

type Handler = (match: RegExpExecArray, url: URL) => { status: number; body: unknown };

function problem(status: number, title: string, detail: string): Problem {
  return { type: "about:blank", title, status, detail };
}

function routes(data: MockData): [string, RegExp, Handler][] {
  return [
    ["GET", /^\/v1\/capabilities$/, () => ({ status: 200, body: data.capabilities })],
    [
      "GET",
      /^\/v1\/modules$/,
      (_match, url) => {
        const type = url.searchParams.get("type");
        const items = data.modules
          .map((module) => module.summary)
          .filter((summary) => type === null || summary.type === type);
        return { status: 200, body: { items } };
      },
    ],
    [
      "GET",
      /^\/v1\/modules\/([^/]+)\/([^/]+)$/,
      (match) => {
        const found = data.modules.find(
          (module) => module.summary.name === match[1] && module.summary.version === match[2],
        );
        return found
          ? { status: 200, body: found.manifest }
          : { status: 404, body: problem(404, "Not Found", "No such module is installed.") };
      },
    ],
    ["GET", /^\/v1\/agents$/, () => ({ status: 200, body: data.agents })],
    ["GET", /^\/v1\/runs$/, () => ({ status: 200, body: { items: [] } })],
  ];
}

function send(res: ServerResponse, status: number, body: unknown): void {
  const isProblem = status >= 400;
  res.writeHead(status, {
    "Content-Type": isProblem ? "application/problem+json" : "application/json",
    "Cache-Control": "no-store",
  });
  res.end(JSON.stringify(body));
}

/** Creates the request handler of the mock server. */
export function createMockHandler(options: MockServerOptions = {}) {
  const tokens = new Set(options.tokens ?? [MOCK_TOKEN]);
  const table = routes(options.data ?? loadMockData());
  return (req: IncomingMessage, res: ServerResponse): void => {
    const url = new URL(req.url ?? "/", "http://mock.invalid");
    const auth = req.headers.authorization ?? "";
    const token = auth.startsWith("Bearer ") ? auth.slice("Bearer ".length) : "";
    if (!tokens.has(token)) {
      res.setHeader("WWW-Authenticate", 'Bearer realm="fps"');
      send(res, 401, problem(401, "Unauthorized", "The bearer token is missing or unknown."));
      return;
    }
    let pathMatched = false;
    for (const [method, pattern, handler] of table) {
      const match = pattern.exec(url.pathname);
      if (!match) continue;
      pathMatched = true;
      if (method !== req.method) continue;
      const { status, body } = handler(match, url);
      send(res, status, body);
      return;
    }
    if (pathMatched) {
      send(res, 405, problem(405, "Method Not Allowed", "The mock does not serve this method."));
    } else {
      send(res, 404, problem(404, "Not Found", `The mock does not serve ${url.pathname}.`));
    }
  };
}

/** Starts the mock server. Port 0 picks a free port. */
export async function startMockServer(
  port = 0,
  host = "127.0.0.1",
  options: MockServerOptions = {},
): Promise<RunningMockServer> {
  const server = createServer(createMockHandler(options));
  await new Promise<void>((resolve, reject) => {
    server.once("error", reject);
    server.listen(port, host, () => {
      resolve();
    });
  });
  const address = server.address() as AddressInfo;
  return {
    server,
    baseUrl: `http://${host}:${address.port}/v1`,
    close: () =>
      new Promise<void>((resolve, reject) => {
        server.close((error) => {
          if (error) reject(error);
          else resolve();
        });
      }),
  };
}
