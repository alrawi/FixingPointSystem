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
import { App, createQueryClient } from "./App";

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

async function expectNoAxeViolations(container: HTMLElement) {
  // jsdom computes no layout, so the contrast rule cannot run here; the
  // Playwright smoke test checks contrast in a real browser.
  const results = await axe.run(container, { rules: { "color-contrast": { enabled: false } } });
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
    const { container } = renderApp();
    expect(screen.getByRole("heading", { level: 1, name: /Sign in/ })).toBeDefined();
    expect(screen.getByLabelText(/API token/).getAttribute("type")).toBe("password");
    await expectNoAxeViolations(container);
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
    const { container } = renderApp();
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
    await expectNoAxeViolations(container);
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

  it("shows a page for an unknown path", async () => {
    window.history.replaceState(null, "", "/no/such/view");
    window.sessionStorage.setItem("fps.token", MOCK_TOKEN);
    renderApp();
    expect(await screen.findByRole("heading", { level: 1, name: "Page not found" })).toBeDefined();
  });
});
