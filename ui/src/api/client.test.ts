// SPDX-License-Identifier: GPL-3.0-only

import { describe, expect, it, vi } from "vitest";
import { ApiError, createApiClient, fetchCapabilities } from "./client";

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
