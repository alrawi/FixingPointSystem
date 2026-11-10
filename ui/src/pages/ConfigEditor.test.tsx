// SPDX-License-Identifier: GPL-3.0-only
//
// Component tests of the configuration list and the editor against the mock
// server, which serves the contract examples and the configuration operations
// over real HTTP. Each test starts its own server, so stored copies never leak.

import { QueryClient } from "@tanstack/react-query";
import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import axe from "axe-core";
import { afterEach, describe, expect, it } from "vitest";
import { parse } from "yaml";
import {
  MOCK_TOKEN,
  startMockServer,
  type MockServerOptions,
  type RunningMockServer,
} from "../../mock/server";
import { loadMockData } from "../../mock/fixtures";
import { App } from "../App";
import { NEW_CONFIG_TEXT } from "../config/document";

const running: RunningMockServer[] = [];

afterEach(async () => {
  await Promise.all(running.splice(0).map((server) => server.close()));
});

async function renderAt(path: string, options: MockServerOptions = {}) {
  const mock = await startMockServer(0, "127.0.0.1", options);
  running.push(mock);
  window.sessionStorage.setItem("fps.token", MOCK_TOKEN);
  window.history.replaceState(null, "", path);
  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  render(<App baseUrl={mock.baseUrl} queryClient={queryClient} />);
  return { user: userEvent.setup(), mock };
}

async function expectNoAxeViolations() {
  // jsdom computes no layout, so the contrast rule cannot run here.
  const results = await axe.run(document.body, { rules: { "color-contrast": { enabled: false } } });
  expect(results.violations.map((violation) => `${violation.id}: ${violation.help}`)).toEqual([]);
}

/** The editor is lazy, so wait for its heading. */
async function openEditor(path: string, options: MockServerOptions = {}) {
  const session = await renderAt(path, options);
  await screen.findByRole("heading", { level: 2, name: "YAML" });
  return session;
}

function yamlBox(): HTMLTextAreaElement {
  return screen.getByRole("textbox", { name: "YAML" });
}

function section(name: string): HTMLElement {
  return screen.getByRole("region", { name });
}

/** The text of the elements that an input points to with aria-describedby. */
function describedText(element: HTMLElement): string {
  return (element.getAttribute("aria-describedby") ?? "")
    .split(" ")
    .map((id) => document.getElementById(id)?.textContent ?? "")
    .join(" ");
}

/** The status line of the checks section. */
function checkStatus(): string {
  return within(section("Checks")).getAllByRole("status")[0]?.textContent ?? "";
}

function setYaml(text: string) {
  fireEvent.change(yamlBox(), { target: { value: text } });
}

describe("configuration list", () => {
  it("lists the stored examples, newest first, and opens one", async () => {
    const { user } = await renderAt("/configs");
    const table = await screen.findByRole("table", { name: "Stored configurations" });
    const rows = within(table).getAllByRole("row");
    expect(rows).toHaveLength(7);
    expect(within(rows[1] as HTMLElement).getByRole("link").textContent).toBe("seedless_smoke");
    expect(screen.getByRole("link", { name: "New configuration" }).getAttribute("href")).toBe(
      "/configs/new",
    );
    await expectNoAxeViolations();

    await user.click(within(table).getByRole("link", { name: "enterprise_mix_10g" }));
    expect(
      await screen.findByRole("heading", { level: 1, name: "Configuration enterprise_mix_10g" }),
    ).toBeDefined();
    // The forms show the stored values.
    expect(within(section("Load")).getByLabelText("Objective value")).toHaveProperty(
      "value",
      "10000000000",
    );
    expect(within(section("Profile")).getAllByRole("group")).toHaveLength(3);
    const text = yamlBox().value;
    expect(parse(text)).toMatchObject({
      metadata: { name: "enterprise_mix_10g" },
      topology: "app_sim",
    });
    expect(checkStatus()).toContain("passes the schema check");
    await expectNoAxeViolations();
  });

  it("says so when no configuration is stored and when one is missing", async () => {
    await renderAt("/configs", { data: { ...loadMockData(), configs: [] } });
    expect(await screen.findByText(/No configuration is stored yet/)).toBeDefined();
    expect(screen.queryByRole("table")).toBeNull();
  });

  it("reports a configuration that does not exist", async () => {
    await renderAt("/configs/cfg_missing");
    expect(await screen.findByRole("alert")).toHaveProperty(
      "textContent",
      expect.stringContaining("No configuration has this id"),
    );
  });
});

describe("editor forms and YAML view", () => {
  it("starts a new configuration without problems", async () => {
    await openEditor("/configs/new");
    expect(yamlBox().value).toBe(NEW_CONFIG_TEXT);
    expect(checkStatus()).toContain("passes the schema check");
    expect(within(section("Checks")).queryByRole("list")).toBeNull();
    for (const name of ["General", "Network", "Profile", "Load", "Timeline", "Criteria"]) {
      expect(section(name)).toBeDefined();
    }
    await expectNoAxeViolations();
  });

  it("edits the YAML through a form and keeps the comment and the other lines", async () => {
    const { user } = await openEditor("/configs/new");
    const name = within(section("General")).getByLabelText("Name");
    await user.clear(name);
    await user.type(name, "edited_name");
    const lines = yamlBox().value.split("\n");
    const before = NEW_CONFIG_TEXT.split("\n");
    expect(lines).toHaveLength(before.length);
    expect(lines[0]).toBe(before[0]);
    expect(lines.filter((line, index) => line !== before[index])).toEqual(["  name: edited_name"]);
    expect((name as HTMLInputElement).value).toBe("edited_name");
  });

  it("updates the forms when the YAML changes", async () => {
    await openEditor("/configs/new");
    setYaml(
      NEW_CONFIG_TEXT.replace("timeline: { seed: 1 }", "timeline: { seed: 77, warmup_s: 5 }"),
    );
    const timeline = section("Timeline");
    expect(within(timeline).getByLabelText("Seed")).toHaveProperty("value", "77");
    expect(within(timeline).getByLabelText("Warmup in seconds")).toHaveProperty("value", "5");
  });

  it("removes an optional key when its field is emptied, and writes numbers as numbers", async () => {
    const { user } = await openEditor("/configs/new");
    const seed = within(section("Timeline")).getByLabelText("Seed");
    await user.clear(seed);
    expect(yamlBox().value).toContain("timeline: {}");
    await user.type(seed, "12");
    expect(yamlBox().value).toContain("timeline: { seed: 12 }");
  });

  it("shows inline errors from the schema and blocks saving", async () => {
    const { user } = await openEditor("/configs/new");
    const weight = within(section("Profile")).getByLabelText("Weight");
    await user.clear(weight);
    await user.type(weight, "abc");
    expect(weight.getAttribute("aria-invalid")).toBe("true");
    expect(describedText(weight)).toContain("must be number");
    // The same problem leads the list of checks, with its pointer.
    expect(within(section("Checks")).getByText(/\/profile\/mix\/0\/weight/)).toBeDefined();
    expect(screen.getByRole("button", { name: "Save as new" })).toHaveProperty("disabled", true);
    expect(screen.getByRole("button", { name: "Validate on controller" })).toHaveProperty(
      "disabled",
      true,
    );
    await expectNoAxeViolations();

    await user.clear(weight);
    await user.type(weight, "2.5");
    expect(weight.getAttribute("aria-invalid")).toBeNull();
    expect(screen.getByRole("button", { name: "Save as new" })).toHaveProperty("disabled", false);
  });

  it("reports a required field that the YAML lacks and a field that is not allowed", async () => {
    await openEditor("/configs/new");
    setYaml(NEW_CONFIG_TEXT.replace("  basis: sessions\n", "").replace("kind: Test", "kind: Nope"));
    const profile = section("Profile");
    expect(within(profile).getByLabelText("Mix basis").getAttribute("aria-invalid")).toBe("true");
    const checks = section("Checks");
    expect(within(checks).getByText(/\/profile\/basis: is required/)).toBeDefined();
    expect(within(checks).getByText(/\/kind: must be Test/)).toBeDefined();
    setYaml(NEW_CONFIG_TEXT.replace("kind: Test", "kind: Test\nsurprise: 1"));
    expect(within(section("Checks")).getByText(/\/surprise: is not a field/)).toBeDefined();
  });

  it("explains a syntax error, pauses the forms, and recovers", async () => {
    await openEditor("/configs/new");
    setYaml("a: [1, 2\n");
    expect(checkStatus()).toContain("syntax error");
    expect(within(section("Checks")).getByText(/line \d+, column \d+/)).toBeDefined();
    expect(within(section("General")).getByLabelText("Name")).toHaveProperty("disabled", true);
    expect(screen.getByRole("button", { name: "Save as new" })).toHaveProperty("disabled", true);
    expect(yamlBox().getAttribute("aria-invalid")).toBe("true");
    await expectNoAxeViolations();
    setYaml(NEW_CONFIG_TEXT);
    expect(within(section("General")).getByLabelText("Name")).toHaveProperty("disabled", false);
  });

  it("adds, moves, and removes list items", async () => {
    const { user } = await openEditor("/configs/new");
    const criteria = section("Criteria");
    await user.click(within(criteria).getByRole("button", { name: "Add criterion" }));
    expect(within(criteria).getAllByRole("group")).toHaveLength(2);
    expect(parse(yamlBox().value)).toMatchObject({
      criteria: [{ metric: "sessions.failed_ratio" }, { metric: "sessions.failed_ratio", op: "<" }],
    });
    // The new item takes the flow style of its sibling.
    expect(yamlBox().value).toMatch(
      /^ {2}- \{ metric: sessions.failed_ratio, op: "<", value: 0.01 \}$/m,
    );

    const metrics = within(criteria).getAllByLabelText("Metric");
    await user.type(metrics[1] as HTMLElement, "_x");
    await user.click(within(criteria).getByRole("button", { name: "Move up criterion 2" }));
    expect((within(criteria).getAllByLabelText("Metric")[0] as HTMLInputElement).value).toBe(
      "sessions.failed_ratio_x",
    );
    expect(within(criteria).getByRole("button", { name: "Move up criterion 1" })).toHaveProperty(
      "disabled",
      true,
    );

    await user.click(within(criteria).getByRole("button", { name: "Remove criterion 1" }));
    expect(within(criteria).getAllByRole("group")).toHaveLength(1);
    expect(parse(yamlBox().value)).toMatchObject({
      criteria: [{ metric: "sessions.failed_ratio" }],
    });
  });

  it("suggests the scenarios of the installed modules", async () => {
    await openEditor("/configs/new");
    await waitFor(() => {
      const input = within(section("Profile")).getByLabelText<HTMLInputElement>("Scenario");
      const options = Array.from(input.list?.options ?? []).map((option) => option.value);
      expect(options).toContain("http1/web_get");
      expect(options).toContain("dns/lookup");
    });
  });

  it("points to the YAML view for fields that the forms do not edit", async () => {
    await openEditor("/configs/cfg_dns_zone");
    expect(within(section("General")).getByText(/Module settings/).textContent).toContain(
      "zone_file",
    );
  });
});

describe("saving and validating on the controller", () => {
  it("saves the text as a new configuration and lists it", async () => {
    const { user } = await openEditor("/configs/new");
    await user.click(screen.getByRole("button", { name: "Save as new" }));
    const notice = await screen.findByText(/Saved as a new configuration with the id cfg_mock0001/);
    expect(notice.textContent).toContain("Open the stored copy");
    await user.click(within(notice).getByRole("link", { name: "Open the stored copy" }));
    expect(
      await screen.findByRole("heading", { level: 1, name: "Configuration new_test" }),
    ).toBeDefined();
  });

  it("shows the findings of the controller next to their fields", async () => {
    const { user } = await openEditor("/configs/cfg_dns_zone");
    await user.click(screen.getByRole("button", { name: "Validate on controller" }));
    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toContain("found problems in the configuration cfg_dns_zone");
    const scenario = within(section("Profile")).getByLabelText("Scenario");
    expect(scenario.getAttribute("aria-invalid")).toBe("true");
    expect(describedText(scenario)).toContain("scenario dns/lookup_a is not provided");
    expect(screen.getByText(/Agents required 1, agents available 1/)).toBeDefined();
    await expectNoAxeViolations();

    // A change makes the findings stale, so they leave the form.
    await user.type(scenario, "x");
    expect(scenario.getAttribute("aria-describedby") ?? "").not.toContain("-err");
  });

  it("validates a stored configuration without storing another copy", async () => {
    const { user, mock } = await openEditor("/configs/cfg_http_smoke");
    await user.click(screen.getByRole("button", { name: "Validate on controller" }));
    await screen.findByText("The controller accepts the configuration cfg_http_smoke.");
    const list = await fetch(`${mock.baseUrl}/configs`, {
      headers: { Authorization: `Bearer ${MOCK_TOKEN}` },
    });
    expect(((await list.json()) as { items: unknown[] }).items).toHaveLength(6);
  });

  it("stores changed text before it validates, and validates the stored copy", async () => {
    const { user, mock } = await openEditor("/configs/cfg_http_smoke");
    const name = within(section("General")).getByLabelText("Name");
    await user.clear(name);
    await user.type(name, "http_smoke_v2");
    await user.click(screen.getByRole("button", { name: "Validate on controller" }));
    await screen.findByText(/The controller accepts the configuration cfg_mock0001/);
    const list = await fetch(`${mock.baseUrl}/configs`, {
      headers: { Authorization: `Bearer ${MOCK_TOKEN}` },
    });
    const body = (await list.json()) as { items: { name: string }[] };
    expect(body.items[0]?.name).toBe("http_smoke_v2");
  });

  it("shows an error of the controller and its findings when saving fails", async () => {
    const { user } = await openEditor("/configs/new", {
      faults: [{ method: "POST", path: "/configs", status: 503, detail: "The store is down." }],
    });
    await user.click(screen.getByRole("button", { name: "Save as new" }));
    expect((await screen.findByRole("alert")).textContent).toContain("The store is down.");
  });

  it("signs out when the controller rejects the token while saving", async () => {
    const { user } = await openEditor("/configs/new", {
      faults: [{ method: "POST", path: "/configs", status: 401 }],
    });
    await user.click(screen.getByRole("button", { name: "Save as new" }));
    expect(await screen.findByText(/no longer accepts the token/)).toBeDefined();
  });
});
