// SPDX-License-Identifier: GPL-3.0-only
//
// The only way components reach the controller. Every call goes through the
// client that openapi-fetch builds from the generated contract types.

import createClient, { type Client } from "openapi-fetch";
import type { components, paths } from "./schema";

export type Schemas = components["schemas"];
export type Capabilities = Schemas["Capabilities"];
export type Agent = Schemas["Agent"];
export type ModuleSummary = Schemas["ModuleSummary"];
export type MetricSample = Schemas["MetricSample"];
export type Problem = Schemas["Problem"];

export type ApiClient = Client<paths>;

/** Base URL of the controller API. The controller serves the UI, so the default is relative. */
export const DEFAULT_API_BASE = "/v1";

export interface ApiClientOptions {
  /** Base URL that ends in the API version, for example `/v1` or `http://127.0.0.1:8090/v1`. */
  baseUrl?: string;
  /** Replacement for the global `fetch`, used by tests. */
  fetch?: (request: Request) => Promise<Response>;
}

/** Creates a client that sends the bearer token with every request. */
export function createApiClient(token: string, options: ApiClientOptions = {}): ApiClient {
  const client = createClient<paths>({
    baseUrl: options.baseUrl ?? DEFAULT_API_BASE,
    // Resolve the global at call time, so tests that replace fetch see the change.
    fetch: options.fetch ?? ((input: Request) => globalThis.fetch(input)),
  });
  client.use({
    onRequest({ request }) {
      request.headers.set("Authorization", `Bearer ${token}`);
      return request;
    },
  });
  return client;
}

/** An error response of the API, carrying the RFC 9457 problem details when the body had them. */
export class ApiError extends Error {
  readonly status: number;
  readonly problem: Problem | undefined;

  constructor(status: number, problem: Problem | undefined) {
    super(problem?.detail ?? problem?.title ?? `The controller answered with HTTP ${status}.`);
    this.name = "ApiError";
    this.status = status;
    this.problem = problem;
  }

  /** True when the token is missing, expired, or lacks the scope. */
  get isUnauthorized(): boolean {
    return this.status === 401 || this.status === 403;
  }
}

interface FetchResult<T> {
  data?: T;
  error?: unknown;
  response: Response;
}

/** Returns the data of a client call or throws an ApiError. */
export function unwrap<T>(result: FetchResult<T>): T {
  if (result.response.ok && result.data !== undefined) {
    return result.data;
  }
  const problem =
    typeof result.error === "object" && result.error !== null
      ? (result.error as Problem)
      : undefined;
  throw new ApiError(result.response.status, problem);
}

/** Fetches the capabilities of the controller. */
export async function fetchCapabilities(
  client: ApiClient,
  signal?: AbortSignal,
): Promise<Capabilities> {
  return unwrap(await client.GET("/capabilities", signal ? { signal } : {}));
}
