// SPDX-License-Identifier: GPL-3.0-only
//
// A mock of the controller REST API for UI tests and local development. It
// serves the read operations that the UI uses so far from the contract
// examples, checks the bearer token, pages module lists, injects configured
// faults, and answers everything else with an RFC 9457 problem. It needs only
// node:http, so it adds no dependency.

import { createServer, type IncomingMessage, type Server, type ServerResponse } from "node:http";
import type { AddressInfo } from "node:net";
import type { Problem } from "../src/api/client";
import { loadMockData, type MockData } from "./fixtures";

/** The token that the mock server accepts unless the options name others. */
export const MOCK_TOKEN = "fps-mock-token";

/** An error answer that replaces the normal answer of matching requests, for error tests. */
export interface MockFault {
  /** The method to fail, every method when absent. */
  method?: string;
  /** The path below /v1, such as `/modules`, or a pattern for the path below /v1. */
  path: string | RegExp;
  status: number;
  /** The detail of the problem body. */
  detail?: string;
}

export interface MockServerOptions {
  /** Tokens that the server accepts. */
  tokens?: string[];
  data?: MockData;
  /** Errors to answer in place of the normal answers, checked after the token. */
  faults?: MockFault[];
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

/** The largest page_size that the contract allows. */
const MAX_PAGE_SIZE = 500;
/** The page_size that the contract names as the default. */
const DEFAULT_PAGE_SIZE = 50;

/**
 * Cuts one page out of a list as the PageSize and PageToken parameters
 * describe. The page token is the offset of the first item, which clients
 * treat as opaque.
 */
function paginate(items: unknown[], url: URL): { status: number; body: unknown } {
  const sizeText = url.searchParams.get("page_size");
  const tokenText = url.searchParams.get("page_token");
  const size = sizeText === null ? DEFAULT_PAGE_SIZE : Number(sizeText);
  if (!Number.isInteger(size) || size < 1 || size > MAX_PAGE_SIZE) {
    return {
      status: 400,
      body: problem(400, "Bad Request", `page_size must be an integer from 1 to ${MAX_PAGE_SIZE}.`),
    };
  }
  const offset = tokenText === null ? 0 : Number(tokenText);
  if (!/^\d+$/.test(tokenText ?? "0") || offset > items.length) {
    return { status: 400, body: problem(400, "Bad Request", "The page_token is not valid.") };
  }
  const end = offset + size;
  return {
    status: 200,
    body: {
      items: items.slice(offset, end),
      ...(end < items.length ? { next_page_token: String(end) } : {}),
    },
  };
}

function matchesFault(fault: MockFault, method: string, path: string): boolean {
  if (fault.method !== undefined && fault.method !== method) return false;
  return typeof fault.path === "string" ? fault.path === path : fault.path.test(path);
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
        return paginate(items, url);
      },
    ],
    [
      "GET",
      /^\/v1\/modules\/([^/]+)\/([^/]+)$/,
      (match) => {
        const name = decodeURIComponent(match[1] ?? "");
        const version = decodeURIComponent(match[2] ?? "");
        const found = data.modules.find(
          (module) => module.summary.name === name && module.summary.version === version,
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
  const faults = options.faults ?? [];
  return (req: IncomingMessage, res: ServerResponse): void => {
    const url = new URL(req.url ?? "/", "http://mock.invalid");
    const auth = req.headers.authorization ?? "";
    const token = auth.startsWith("Bearer ") ? auth.slice("Bearer ".length) : "";
    if (!tokens.has(token)) {
      res.setHeader("WWW-Authenticate", 'Bearer realm="fps"');
      send(res, 401, problem(401, "Unauthorized", "The bearer token is missing or unknown."));
      return;
    }
    const method = req.method ?? "GET";
    const below = url.pathname.replace(/^\/v1(?=\/)/, "");
    const fault = faults.find((candidate) => matchesFault(candidate, method, below));
    if (fault) {
      const detail = fault.detail ?? `The mock answers ${below} with HTTP ${fault.status}.`;
      send(res, fault.status, problem(fault.status, "Mock Fault", detail));
      return;
    }
    let pathMatched = false;
    for (const [routeMethod, pattern, handler] of table) {
      const match = pattern.exec(url.pathname);
      if (!match) continue;
      pathMatched = true;
      if (routeMethod !== method) continue;
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
