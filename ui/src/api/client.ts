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
export type ModuleType = Schemas["ModuleType"];
/**
 * A module manifest as GET /modules/{name}/{version} returns it. The generator
 * copies the `$defs` of the schema into the type, which no manifest carries.
 */
export type ModuleManifest = Omit<Schemas["module-manifest.schema"], "$defs">;
/** A template function that a module declares in its manifest. */
export type ModuleFunction = Schemas["function"];
export type MetricSample = Schemas["MetricSample"];
export type MetricView = Schemas["MetricView"];
export type MetricSeriesSet = Schemas["MetricSeriesSet"];
export type Run = Schemas["Run"];
export type RunState = Schemas["RunState"];
export type RunSummary = Schemas["RunSummary"];
/** A test configuration as GET /configs/{id} returns it. */
export type TestConfig = Schemas["test-config.schema"];
export type ConfigRef = Schemas["ConfigRef"];
export type Finding = Schemas["Finding"];
export type ValidationReport = Schemas["ValidationReport"];
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

/** The largest page that GET /modules returns, from the PageSize parameter of the contract. */
export const MAX_PAGE_SIZE = 500;

/** Options of fetchModules. */
export interface FetchModulesOptions {
  /** Lists only modules of this type. */
  type?: ModuleType;
  /** Page size of each request, the contract maximum by default. */
  pageSize?: number;
  signal?: AbortSignal;
}

/** Fetches every installed module, following `next_page_token` until the last page. */
export async function fetchModules(
  client: ApiClient,
  options: FetchModulesOptions = {},
): Promise<ModuleSummary[]> {
  const modules: ModuleSummary[] = [];
  const seen = new Set<string>();
  let pageToken: string | undefined;
  for (;;) {
    const page = unwrap(
      await client.GET("/modules", {
        params: {
          query: {
            page_size: options.pageSize ?? MAX_PAGE_SIZE,
            ...(options.type === undefined ? {} : { type: options.type }),
            ...(pageToken === undefined ? {} : { page_token: pageToken }),
          },
        },
        ...(options.signal ? { signal: options.signal } : {}),
      }),
    );
    modules.push(...page.items);
    pageToken = page.next_page_token;
    // A token that repeats would loop forever, so it ends the listing.
    if (pageToken === undefined || pageToken === "" || seen.has(pageToken)) return modules;
    seen.add(pageToken);
  }
}

/** Fetches the manifest of one installed module. */
export async function fetchModule(
  client: ApiClient,
  name: string,
  version: string,
  signal?: AbortSignal,
): Promise<ModuleManifest> {
  return unwrap(
    await client.GET("/modules/{name}/{version}", {
      params: { path: { name, version } },
      ...(signal ? { signal } : {}),
    }),
  );
}

/** Fetches every stored configuration, newest first, following `next_page_token`. */
export async function fetchConfigs(client: ApiClient, signal?: AbortSignal): Promise<ConfigRef[]> {
  const configs: ConfigRef[] = [];
  const seen = new Set<string>();
  let pageToken: string | undefined;
  for (;;) {
    const page = unwrap(
      await client.GET("/configs", {
        params: {
          query: {
            page_size: MAX_PAGE_SIZE,
            ...(pageToken === undefined ? {} : { page_token: pageToken }),
          },
        },
        ...(signal ? { signal } : {}),
      }),
    );
    configs.push(...page.items);
    pageToken = page.next_page_token;
    if (pageToken === undefined || pageToken === "" || seen.has(pageToken)) return configs;
    seen.add(pageToken);
  }
}

/** Fetches one stored configuration. The controller answers JSON, so comments are gone. */
export async function fetchConfig(
  client: ApiClient,
  id: string,
  signal?: AbortSignal,
): Promise<TestConfig> {
  return unwrap(
    await client.GET("/configs/{id}", {
      params: { path: { id } },
      ...(signal ? { signal } : {}),
    }),
  );
}

/**
 * Stores the YAML text as a new configuration. The text goes to the controller
 * as it is, with the YAML content type, so the controller reads exactly what
 * the editor shows. A text that breaks the schema fails with 422 and an
 * `errors` list in the problem.
 */
export async function createConfig(client: ApiClient, yamlText: string): Promise<ConfigRef> {
  return unwrap(
    await client.POST("/configs", {
      body: yamlText as unknown as TestConfig,
      bodySerializer: (body) => body as unknown as string,
      headers: { "Content-Type": "application/yaml" },
    }),
  );
}

/** Asks the controller to validate a stored configuration. */
export async function validateConfig(client: ApiClient, id: string): Promise<ValidationReport> {
  return unwrap(await client.POST("/configs/{id}:validate", { params: { path: { id } } }));
}

/** Fetches the registered agents. */
export async function fetchAgents(client: ApiClient, signal?: AbortSignal): Promise<Agent[]> {
  return unwrap(await client.GET("/agents", signal ? { signal } : {}));
}

/** The states in which a run no longer changes. */
export const FINAL_RUN_STATES: readonly RunState[] = ["completed", "failed", "aborted"];

/** True when the run reached a state that it never leaves. */
export function isFinalRunState(state: RunState): boolean {
  return FINAL_RUN_STATES.includes(state);
}

/** Fetches every run, newest first, following `next_page_token`. */
export async function fetchRuns(
  client: ApiClient,
  options: { state?: RunState; signal?: AbortSignal } = {},
): Promise<Run[]> {
  const runs: Run[] = [];
  const seen = new Set<string>();
  let pageToken: string | undefined;
  for (;;) {
    const page = unwrap(
      await client.GET("/runs", {
        params: {
          query: {
            page_size: MAX_PAGE_SIZE,
            ...(options.state === undefined ? {} : { state: options.state }),
            ...(pageToken === undefined ? {} : { page_token: pageToken }),
          },
        },
        ...(options.signal ? { signal: options.signal } : {}),
      }),
    );
    runs.push(...page.items);
    pageToken = page.next_page_token;
    if (pageToken === undefined || pageToken === "" || seen.has(pageToken)) return runs;
    seen.add(pageToken);
  }
}

/** Starts a run of a stored configuration. Without agents the controller picks reserved ones. */
export async function startRun(
  client: ApiClient,
  request: { config_id: string; agents?: string[] },
): Promise<Run> {
  return unwrap(await client.POST("/runs", { body: request }));
}

/** Fetches the state and the summary counters of one run. */
export async function fetchRun(client: ApiClient, id: string, signal?: AbortSignal): Promise<Run> {
  return unwrap(
    await client.GET("/runs/{id}", {
      params: { path: { id } },
      ...(signal ? { signal } : {}),
    }),
  );
}

/** Asks the controller to stop a run. With `force` the run aborts at once and skips ramp down. */
export async function stopRun(client: ApiClient, id: string, force = false): Promise<Run> {
  return unwrap(
    await client.POST("/runs/{id}:stop", { params: { path: { id } }, body: { force } }),
  );
}

/** Fetches the samples of one metric view, optionally only those since a time. */
export async function fetchRunMetrics(
  client: ApiClient,
  id: string,
  options: { view: MetricView; from?: string; signal?: AbortSignal },
): Promise<MetricSample[]> {
  const set = unwrap(
    await client.GET("/runs/{id}/metrics", {
      params: {
        path: { id },
        query: {
          view: options.view,
          ...(options.from === undefined ? {} : { from: options.from }),
        },
      },
      ...(options.signal ? { signal: options.signal } : {}),
    }),
  );
  return set.samples;
}
