// SPDX-License-Identifier: GPL-3.0-only
//
// Component tests of the whole UI against the mock server, which serves the
// contract examples over real HTTP.

import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import axe from "axe-core";
import { afterAll, beforeAll, describe, expect, it } from "vitest";
import { MOCK_TOKEN, startMockServer, type RunningMockServer } from "../mock/server";
import { loadMockData } from "../mock/fixtures";
import { QueryClient } from "@tanstack/react-query";
import { ApiError } from "./api/client";
import { App, createQueryClient, isRetryable } from "./App";

let mock: RunningMockServer;

beforeAll(async () => {
  mock = await startMockServer();
});
afterAll(async () => {
  await mock.close();
});

function renderApp() {
  const queryClient = createQueryClient();
  return render(<App baseUrl={mock.baseUrl} queryClient={queryClient} />);
}

async function expectNoAxeViolations() {
  // jsdom computes no layout, so the contrast rule cannot run here; the
  // Playwright smoke test checks contrast in a real browser.
  const results = await axe.run(document.body, { rules: { "color-contrast": { enabled: false } } });
  expect(results.violations.map((violation) => `${violation.id}: ${violation.help}`)).toEqual([]);
}

async function signIn(token: string) {
  const user = userEvent.setup();
  await user.type(screen.getByLabelText(/API token/), token);
  await user.click(screen.getByRole("button", { name: "Sign in" }));
  return user;
}

describe("sign in", () => {
  it("shows an accessible sign in form first", async () => {
    renderApp();
    expect(screen.getByRole("heading", { level: 1, name: /Sign in/ })).toBeDefined();
    expect(screen.getByLabelText(/API token/).getAttribute("type")).toBe("password");
    await expectNoAxeViolations();
  });

  it("refuses a token that the controller rejects", async () => {
    renderApp();
    await signIn("wrong-token");
    expect((await screen.findByRole("alert")).textContent).toMatch(/rejected this token/);
    expect(window.sessionStorage.getItem("fps.token")).toBeNull();
  });

  it("keeps the token for the tab after a successful sign in", async () => {
    renderApp();
    await signIn(MOCK_TOKEN);
    await screen.findByRole("heading", { level: 1, name: "Capabilities" });
    expect(window.sessionStorage.getItem("fps.token")).toBe(MOCK_TOKEN);
  });
});

describe("capabilities", () => {
  it("renders the capabilities of the mock server", async () => {
    const expected = loadMockData().capabilities;
    renderApp();
    await signIn(MOCK_TOKEN);

    await screen.findByRole("heading", { level: 1, name: "Capabilities" });
    const fact = (term: string) => screen.getByText(term, { selector: "dt" }).nextElementSibling;
    expect(fact("Engine API")?.textContent).toBe(expected.engine_api);
    expect(fact("API version")?.textContent).toBe(expected.api_version);
    expect(fact("Backends")?.textContent).toBe(expected.backends.join(", "));

    const agents = within(screen.getByRole("region", { name: "Agents" }));
    for (const agent of expected.agents) {
      const row = agents.getByRole("row", { name: new RegExp(agent.id) });
      expect(within(row).getByText(agent.state)).toBeDefined();
    }

    const modules = within(screen.getByRole("region", { name: "Modules" }));
    expect(modules.getAllByRole("row")).toHaveLength(expected.modules.length + 1);
    for (const module of expected.modules) {
      const row = modules.getByRole("rowheader", { name: module.name }).closest("tr");
      expect(row).not.toBeNull();
      expect(within(row as HTMLElement).getByText(module.version)).toBeDefined();
    }
    await expectNoAxeViolations();
  });

  it("lays out a skip link, a banner, the main navigation, and the main region", async () => {
    renderApp();
    await signIn(MOCK_TOKEN);
    await screen.findByRole("heading", { level: 1, name: "Capabilities" });
    expect(screen.getByRole("link", { name: "Skip to content" }).getAttribute("href")).toBe(
      "#main",
    );
    expect(screen.getByRole("banner")).toBeDefined();
    const nav = screen.getByRole("navigation", { name: "Main" });
    expect(
      within(nav).getByRole("link", { name: "Capabilities" }).getAttribute("aria-current"),
    ).toBe("page");
    expect(screen.getByRole("main").id).toBe("main");
  });

  it("returns to sign in when the stored token has expired", async () => {
    window.sessionStorage.setItem("fps.token", "expired-token");
    renderApp();
    expect((await screen.findByRole("alert")).textContent).toMatch(/no longer accepts the token/);
    expect(screen.getByRole("heading", { level: 1, name: /Sign in/ })).toBeDefined();
    expect(window.sessionStorage.getItem("fps.token")).toBeNull();
  });

  it("signs out on request", async () => {
    renderApp();
    const user = await signIn(MOCK_TOKEN);
    await screen.findByRole("heading", { level: 1, name: "Capabilities" });
    await user.click(screen.getByRole("button", { name: "Sign out" }));
    expect(screen.getByRole("heading", { level: 1, name: /Sign in/ })).toBeDefined();
    expect(window.sessionStorage.getItem("fps.token")).toBeNull();
    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("reports an API error other than a rejected token and stays signed in", async () => {
    window.sessionStorage.setItem("fps.token", MOCK_TOKEN);
    const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    render(<App baseUrl={`${mock.baseUrl}/missing`} queryClient={queryClient} />);
    expect((await screen.findByRole("alert")).textContent).toMatch(
      /Capabilities could not be loaded\. The mock does not serve/,
    );
    expect(screen.getByRole("button", { name: "Sign out" })).toBeDefined();
  });

  it("shows a page for an unknown path", async () => {
    window.history.replaceState(null, "", "/no/such/view");
    window.sessionStorage.setItem("fps.token", MOCK_TOKEN);
    renderApp();
    expect(await screen.findByRole("heading", { level: 1, name: "Page not found" })).toBeDefined();
  });
});

describe("isRetryable", () => {
  it("retries only errors that may heal", () => {
    for (const status of [500, 502, 503, 408, 429]) {
      expect(isRetryable(new ApiError(status, undefined))).toBe(true);
    }
    for (const status of [400, 401, 403, 404, 409]) {
      expect(isRetryable(new ApiError(status, undefined))).toBe(false);
    }
    expect(isRetryable(new TypeError("Failed to fetch"))).toBe(true);
  });
});
