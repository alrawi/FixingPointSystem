// SPDX-License-Identifier: GPL-3.0-only

import { describe, expect, it, vi } from "vitest";
import {
  ApiError,
  createApiClient,
  fetchCapabilities,
  fetchModule,
  fetchModules,
  MAX_PAGE_SIZE,
} from "./client";

function json(status: number, body: unknown, type = "application/json"): Response {
  return new Response(JSON.stringify(body), { status, headers: { "Content-Type": type } });
}

describe("createApiClient", () => {
  it("sends the bearer token to the base URL", async () => {
    const fetchMock = vi.fn((request: Request) => {
      expect(request.url).toBe("http://controller.test/v1/capabilities");
      expect(request.headers.get("Authorization")).toBe("Bearer abc");
      return Promise.resolve(
        json(200, { engine_api: "0.1.0", backends: [], agents: [], modules: [] }),
      );
    });
    const client = createApiClient("abc", {
      baseUrl: "http://controller.test/v1",
      fetch: fetchMock,
    });
    const capabilities = await fetchCapabilities(client);
    expect(capabilities.engine_api).toBe("0.1.0");
    expect(fetchMock).toHaveBeenCalledOnce();
  });

  it("turns a problem response into an ApiError", async () => {
    const client = createApiClient("abc", {
      baseUrl: "http://controller.test/v1",
      fetch: () =>
        Promise.resolve(
          json(
            401,
            { title: "Unauthorized", status: 401, detail: "Token expired." },
            "application/problem+json",
          ),
        ),
    });
    const error = await fetchCapabilities(client).catch((caught: unknown) => caught);
    expect(error).toBeInstanceOf(ApiError);
    expect((error as ApiError).isUnauthorized).toBe(true);
    expect((error as ApiError).message).toBe("Token expired.");
  });

  it("names the status when the error body is not a problem", async () => {
    const client = createApiClient("abc", {
      baseUrl: "http://controller.test/v1",
      fetch: () => Promise.resolve(new Response("", { status: 502 })),
    });
    await expect(fetchCapabilities(client)).rejects.toThrow("HTTP 502");
  });
});

describe("fetchModules", () => {
  const modules = ["a", "b", "c"].map((name) => ({
    name,
    version: "1.0.0",
    type: "app",
    license: "MIT",
  }));

  /** Serves the modules in pages, with the offset as the page token. */
  function pagedClient(requests: URL[], token: (end: number) => string = String) {
    return createApiClient("abc", {
      baseUrl: "http://controller.test/v1",
      fetch: (request) => {
        const url = new URL(request.url);
        requests.push(url);
        const size = Number(url.searchParams.get("page_size"));
        const offset = Number(url.searchParams.get("page_token") ?? "0");
        const end = offset + size;
        return Promise.resolve(
          json(200, {
            items: modules.slice(offset, end),
            ...(end < modules.length ? { next_page_token: token(end) } : {}),
          }),
        );
      },
    });
  }

  it("asks for the largest page by default", async () => {
    const requests: URL[] = [];
    expect(await fetchModules(pagedClient(requests))).toEqual(modules);
    expect(requests).toHaveLength(1);
    expect(requests[0]?.searchParams.get("page_size")).toBe(String(MAX_PAGE_SIZE));
  });

  it("follows next_page_token until the last page", async () => {
    const requests: URL[] = [];
    const listed = await fetchModules(pagedClient(requests), { pageSize: 2, type: "app" });
    expect(listed.map((module) => module.name)).toEqual(["a", "b", "c"]);
    expect(requests.map((url) => url.searchParams.get("page_token"))).toEqual([null, "2"]);
    expect(requests.every((url) => url.searchParams.get("type") === "app")).toBe(true);
  });

  it("stops when a page token repeats", async () => {
    const requests: URL[] = [];
    const client = pagedClient(requests, () => "1");
    const listed = await fetchModules(client, { pageSize: 1 });
    expect(requests).toHaveLength(2);
    expect(listed.map((module) => module.name)).toEqual(["a", "b"]);
  });

  it("encodes the name and version of fetchModule into the path", async () => {
    const client = createApiClient("abc", {
      baseUrl: "http://controller.test/v1",
      fetch: (request) => {
        expect(request.url).toBe("http://controller.test/v1/modules/dns/1.0.0%2Bbuild");
        return Promise.resolve(json(404, { title: "Not Found", status: 404 }));
      },
    });
    const error = await fetchModule(client, "dns", "1.0.0+build").catch(
      (caught: unknown) => caught,
    );
    expect((error as ApiError).status).toBe(404);
  });
});
