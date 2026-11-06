// SPDX-License-Identifier: GPL-3.0-only
//
// Component tests of the module list and detail views against the mock server,
// which serves the contract examples over real HTTP. Each test starts its own
// server, so the registry can be empty, hold the two example modules, or fail.

import { QueryClient } from "@tanstack/react-query";
import { render, screen, within } from "@testing-library/react";
import userEvent, { type UserEvent } from "@testing-library/user-event";
import axe from "axe-core";
import { afterEach, describe, expect, it } from "vitest";
import { loadMockData, withoutModules } from "../../mock/fixtures";
import {
  MOCK_TOKEN,
  startMockServer,
  type MockServerOptions,
  type RunningMockServer,
} from "../../mock/server";
import { App } from "../App";

const running: RunningMockServer[] = [];

afterEach(async () => {
  await Promise.all(running.splice(0).map((server) => server.close()));
});

/** Starts a mock server, signs in through the stored token, and renders the UI at a path. */
async function renderAt(path: string, options: MockServerOptions = {}) {
  const mock = await startMockServer(0, "127.0.0.1", options);
  running.push(mock);
  window.sessionStorage.setItem("fps.token", MOCK_TOKEN);
  window.history.replaceState(null, "", path);
  // No retries, so error states appear at once.
  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  render(<App baseUrl={mock.baseUrl} queryClient={queryClient} />);
  return userEvent.setup();
}

async function expectNoAxeViolations() {
  // jsdom computes no layout, so the contrast rule cannot run here; the
  // Playwright test checks contrast in a real browser.
  const results = await axe.run(document.body, { rules: { "color-contrast": { enabled: false } } });
  expect(results.violations.map((violation) => `${violation.id}: ${violation.help}`)).toEqual([]);
}

/** Presses Tab until the element has focus, and fails after a bound. */
async function tabTo(user: UserEvent, element: HTMLElement) {
  for (let presses = 0; presses < 30 && document.activeElement !== element; presses += 1) {
    await user.tab();
  }
  expect(document.activeElement).toBe(element);
}

function fact(section: HTMLElement, term: string): string | null | undefined {
  return within(section).getByText(term, { selector: "dt" }).nextElementSibling?.textContent;
}

describe("module list", () => {
  it("reports an empty registry", async () => {
    await renderAt("/modules", { data: withoutModules(loadMockData()) });
    expect(await screen.findByText(/No module is installed/)).toBeDefined();
    expect(screen.queryByRole("table")).toBeNull();
    expect(screen.getByRole("heading", { level: 1, name: "Modules" })).toBeDefined();
    await expectNoAxeViolations();
  });

  it("lists the two example modules with their manifests' summary fields", async () => {
    const expected = loadMockData().modules.map((module) => module.summary);
    expect(expected).toHaveLength(2);
    await renderAt("/modules");

    const table = await screen.findByRole("table", { name: "Installed modules" });
    expect(screen.getByText("2 modules are installed.")).toBeDefined();
    expect(within(table).getAllByRole("row")).toHaveLength(expected.length + 1);
    for (const module of expected) {
      const link = within(table).getByRole("link", { name: `${module.name} ${module.version}` });
      expect(link.getAttribute("href")).toBe(`/modules/${module.name}/${module.version}`);
      const row = link.closest("tr") as HTMLElement;
      expect(within(row).getByText(module.type)).toBeDefined();
      expect(within(row).getByText(module.license)).toBeDefined();
      expect(within(row).getByText((module.scenarios ?? []).join(", "))).toBeDefined();
      expect(within(row).getByText(module.summary ?? "")).toBeDefined();
    }
    const nav = screen.getByRole("navigation", { name: "Main" });
    expect(within(nav).getByRole("link", { name: "Modules" }).getAttribute("aria-current")).toBe(
      "page",
    );
    await expectNoAxeViolations();
  });

  it("reports an API error and stays signed in", async () => {
    await renderAt("/modules", {
      faults: [{ path: "/modules", status: 503, detail: "The registry is offline." }],
    });
    expect((await screen.findByRole("alert")).textContent).toBe(
      "Modules could not be loaded. The registry is offline.",
    );
    expect(screen.queryByRole("table")).toBeNull();
    expect(screen.getByRole("button", { name: "Sign out" })).toBeDefined();
    await expectNoAxeViolations();
  });

  it("signs out when the controller rejects the token", async () => {
    await renderAt("/modules", { tokens: ["another-token"] });
    expect((await screen.findByRole("alert")).textContent).toMatch(/no longer accepts the token/);
    expect(screen.getByRole("heading", { level: 1, name: /Sign in/ })).toBeDefined();
  });
});

describe("module navigation", () => {
  it("reaches the modules and a module detail view with the keyboard", async () => {
    const user = await renderAt("/");
    await screen.findByRole("heading", { level: 1, name: "Capabilities" });

    const nav = screen.getByRole("navigation", { name: "Main" });
    await tabTo(user, within(nav).getByRole("link", { name: "Modules" }));
    await user.keyboard("{Enter}");
    const listHeading = await screen.findByRole("heading", { level: 1, name: "Modules" });
    expect(document.activeElement).toBe(listHeading);

    const link = await screen.findByRole("link", { name: "dns 0.1.0" });
    await tabTo(user, link);
    await user.keyboard("{Enter}");
    const heading = await screen.findByRole("heading", { level: 1, name: "Module dns 0.1.0" });
    expect(document.activeElement).toBe(heading);
    await screen.findByRole("region", { name: "Manifest" });

    await tabTo(user, screen.getByRole("link", { name: "All modules" }));
    await user.keyboard("{Enter}");
    const back = await screen.findByRole("heading", { level: 1, name: "Modules" });
    expect(document.activeElement).toBe(back);
    await screen.findByRole("table", { name: "Installed modules" });
  });
});

describe("module detail", () => {
  it("shows the manifest, license, scenarios, parameters, and functions", async () => {
    const http1 = loadMockData().modules.find((module) => module.summary.name === "http1");
    expect(http1).toBeDefined();
    await renderAt("/modules/http1/0.1.0");

    const manifest = await screen.findByRole("region", { name: "Manifest" });
    expect(fact(manifest, "Name")).toBe("http1");
    expect(fact(manifest, "Version")).toBe("0.1.0");
    expect(fact(manifest, "Type")).toBe("app");
    expect(fact(manifest, "Engine API")).toBe(">=0.2, <0.3");
    expect(fact(manifest, "Manifest API")).toBe("fixingpoint.dev/v1alpha1");
    expect(fact(manifest, "Provides")).toBe("http over tcp port 80, http over tcp port 8080");
    expect(fact(manifest, "Requires")).toBe("none");
    expect(fact(manifest, "Assets")).toBe("html_body from content.html");
    expect(fact(manifest, "Extension")).toBe("none");
    expect(fact(manifest, "Summary")).toBe(http1?.manifest["summary"]);

    const license = screen.getByRole("region", { name: "License and provenance" });
    expect(fact(license, "License")).toBe("GPL-3.0-only");
    expect(fact(license, "Source")).toBe("specification");

    const scenarios = screen.getByRole("region", { name: "Scenarios" });
    expect(
      within(scenarios)
        .getAllByRole("listitem")
        .map((item) => item.textContent),
    ).toEqual(http1?.manifest["scenarios"]);

    const parameters = screen.getByRole("table", { name: "Parameters" });
    expect(within(parameters).getAllByRole("row")).toHaveLength(6);
    const port = within(parameters).getByRole("rowheader", { name: "server_port" }).closest("tr");
    expect(
      within(port as HTMLElement)
        .getAllByRole("cell")
        .map((cell) => cell.textContent),
    ).toEqual(["integer", "8080", "no", "minimum 1, maximum 65535", ""]);
    expect(screen.getByText("The module accepts only the parameters below.")).toBeDefined();

    const functions = screen.getByRole("table", { name: "Functions" });
    const request = within(functions).getByRole("rowheader", { name: "request" }).closest("tr");
    expect(
      within(request as HTMLElement).getByRole("cell", { name: /^http1\.request/ }).textContent,
    ).toBe(
      'http1.request(method: text, path: text, host: text, accept: text = "*/*", headers: text = "")',
    );
    expect(within(functions).getAllByRole("row")).toHaveLength(3);
    await expectNoAxeViolations();
  });

  it("reports a module that is not installed", async () => {
    await renderAt("/modules/nope/9.9.9");
    expect((await screen.findByRole("alert")).textContent).toBe(
      "No module named nope is installed at version 9.9.9.",
    );
    expect(screen.getByRole("link", { name: "All modules" })).toBeDefined();
    await expectNoAxeViolations();
  });

  it("reports an API error of the manifest", async () => {
    await renderAt("/modules/dns/0.1.0", {
      faults: [{ method: "GET", path: /^\/modules\/dns\//, status: 500, detail: "Disk failed." }],
    });
    expect((await screen.findByRole("alert")).textContent).toBe(
      "The manifest could not be loaded. Disk failed.",
    );
    expect(screen.queryByRole("region", { name: "Manifest" })).toBeNull();
  });
});
