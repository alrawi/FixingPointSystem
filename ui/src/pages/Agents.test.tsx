// SPDX-License-Identifier: GPL-3.0-only
//
// Component tests of the agents view against the mock server: the list with capabilities,
// reservations, and ports, the reserve and release actions, and the conflicts that the API
// reports. The expected values are written out here, not read from the mock, so a change of
// the mock or of the view shows as a failure.

import { QueryClient } from "@tanstack/react-query";
import { cleanup, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import axe from "axe-core";
import { afterEach, describe, expect, it } from "vitest";
import { loadMockData, type MockData } from "../../mock/fixtures";
import {
  MOCK_TOKEN,
  startMockServer,
  type MockServerOptions,
  type RunningMockServer,
} from "../../mock/server";
import type { Agent } from "../api/client";
import { App } from "../App";

const running: RunningMockServer[] = [];

afterEach(async () => {
  cleanup();
  await Promise.all(
    running.splice(0).map((mock) => {
      mock.server.closeAllConnections();
      return mock.close();
    }),
  );
});

/** Four agents, one in each state, with the capabilities that the view shows. */
const AGENTS: Agent[] = [
  {
    id: "agent-local",
    address: "127.0.0.1:50051",
    state: "online",
    version: "0.4.0",
    cores: 8,
    backend: "af_xdp",
    ports: [
      { name: "client0", driver: "ixgbe", speed_mbps: 10000 },
      { name: "server0", driver: "ixgbe", speed_mbps: 10000 },
    ],
    labels: { site: "lab", rack: "r2" },
  },
  {
    id: "agent-held",
    address: "10.0.0.7:50051",
    state: "reserved",
    version: "0.4.0",
    cores: 4,
    backend: "kernel",
    ports: [{ name: "eth1", driver: "virtio", speed_mbps: 1000 }],
  },
  {
    id: "agent-busy",
    address: "10.0.0.8:50051",
    state: "busy",
    cores: 16,
    backend: "dpdk",
    ports: [{ name: "p0", driver: "mlx5", speed_mbps: 100000 }],
  },
  { id: "agent-down", state: "offline" },
];

function dataWith(agents: Agent[]): MockData {
  const base = loadMockData();
  return { ...base, agents, capabilities: { ...base.capabilities, agents } };
}

/** The pollMs value is large, so the list changes only through the actions and their refreshes. */
const SETTINGS = { flushMs: 5, pollMs: 60_000 };

async function renderAgents(agents: Agent[] = AGENTS, options: MockServerOptions = {}) {
  const mock = await startMockServer(0, "127.0.0.1", { data: dataWith(agents), ...options });
  running.push(mock);
  window.sessionStorage.setItem("fps.token", MOCK_TOKEN);
  window.history.replaceState(null, "", "/agents");
  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  render(<App baseUrl={mock.baseUrl} queryClient={queryClient} runSettings={SETTINGS} />);
  await screen.findByRole("heading", { level: 1, name: "Agents" });
  return { user: userEvent.setup(), mock };
}

/** Calls an agent operation straight on the API, as another user would. */
async function direct(mock: RunningMockServer, id: string, action: "reserve" | "release") {
  const response = await fetch(`${mock.baseUrl}/agents/${id}:${action}`, {
    method: "POST",
    headers: { Authorization: `Bearer ${MOCK_TOKEN}` },
  });
  return response.status;
}

async function expectNoAxeViolations() {
  const results = await axe.run(document.body, { rules: { "color-contrast": { enabled: false } } });
  expect(results.violations.map((violation) => `${violation.id}: ${violation.help}`)).toEqual([]);
}

/** The cell texts of the first table row with this row header. */
function rowTexts(table: HTMLElement, name: string): string[] {
  const row = within(table).getAllByRole("rowheader", { name })[0]?.closest("tr");
  if (row === null || row === undefined) throw new Error(`no row ${name}`);
  return within(row)
    .getAllByRole("cell")
    .map((cell) => cell.textContent);
}

describe("agents view", () => {
  it("lists the agents with capabilities, reservations, and ports", async () => {
    await renderAgents();
    const agents = await screen.findByRole("table", { name: "Agents" });

    expect(screen.getByText("4 agents: 1 online, 1 reserved, 1 busy, 1 offline")).toBeDefined();
    expect(rowTexts(agents, "agent-local")).toEqual([
      "online",
      "af_xdp",
      "8",
      "0.4.0",
      "127.0.0.1:50051",
      "rack=r2, site=lab",
      "none",
      "Reserve agent-local",
    ]);
    expect(rowTexts(agents, "agent-held")).toEqual([
      "reserved",
      "kernel",
      "4",
      "0.4.0",
      "10.0.0.7:50051",
      "",
      "not reported",
      "Release agent-held",
    ]);
    expect(rowTexts(agents, "agent-busy")).toEqual([
      "busy",
      "dpdk",
      "16",
      "not reported",
      "10.0.0.8:50051",
      "",
      "none",
      "In use by a run",
    ]);
    expect(rowTexts(agents, "agent-down")).toEqual([
      "offline",
      "not reported",
      "not reported",
      "not reported",
      "offline",
      "",
      "none",
      "Not connected",
    ]);

    const ports = screen.getByRole("table", { name: "Ports" });
    expect(within(ports).getAllByRole("row")).toHaveLength(5);
    expect(rowTexts(ports, "agent-local")).toEqual(["client0", "ixgbe", "10 Gbit/s"]);
    expect(within(ports).getAllByRole("rowheader", { name: "agent-local" })).toHaveLength(2);
    expect(within(ports).getByRole("cell", { name: "server0" })).toBeDefined();
    expect(within(ports).getByRole("cell", { name: "1 Gbit/s" })).toBeDefined();
    expect(within(ports).getByRole("cell", { name: "100 Gbit/s" })).toBeDefined();
    await expectNoAxeViolations();
  });

  it("opens from the main navigation", async () => {
    window.history.replaceState(null, "", "/");
    const mock = await startMockServer(0, "127.0.0.1", { data: dataWith(AGENTS) });
    running.push(mock);
    window.sessionStorage.setItem("fps.token", MOCK_TOKEN);
    render(
      <App
        baseUrl={mock.baseUrl}
        queryClient={new QueryClient({ defaultOptions: { queries: { retry: false } } })}
        runSettings={SETTINGS}
      />,
    );
    const user = userEvent.setup();
    await screen.findByRole("heading", { level: 1, name: "Capabilities" });
    const nav = screen.getByRole("navigation", { name: "Main" });
    await user.click(within(nav).getByRole("link", { name: "Agents" }));
    await screen.findByRole("table", { name: "Agents" });
    expect(within(nav).getByRole("link", { name: "Agents" }).getAttribute("aria-current")).toBe(
      "page",
    );
  });

  it("says so when no agent is registered and no agent reports a port", async () => {
    await renderAgents([]);
    expect(await screen.findByText("No agent is registered.")).toBeDefined();
    expect(screen.queryByRole("table", { name: "Agents" })).toBeNull();
    await expectNoAxeViolations();
  });

  it("says so when the agents report no port", async () => {
    await renderAgents([{ id: "agent-bare", address: "10.0.0.9:50051", state: "online" }]);
    expect(await screen.findByText("No agent reports a port.")).toBeDefined();
  });

  it("reports a failed load in an alert", async () => {
    window.sessionStorage.setItem("fps.token", MOCK_TOKEN);
    window.history.replaceState(null, "", "/agents");
    const mock = await startMockServer(0, "127.0.0.1", {
      faults: [{ method: "GET", path: "/agents", status: 500, detail: "agent registry is down" }],
    });
    running.push(mock);
    render(
      <App
        baseUrl={mock.baseUrl}
        queryClient={new QueryClient({ defaultOptions: { queries: { retry: false } } })}
        runSettings={SETTINGS}
      />,
    );
    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toBe("Agents could not be loaded. agent registry is down");
  });
});

describe("reserve and release", () => {
  it("reserves an online agent and releases it again", async () => {
    const { user } = await renderAgents();
    await user.click(await screen.findByRole("button", { name: "Reserve agent-local" }));

    expect(
      await screen.findByText("Agent agent-local reserved. It is now reserved."),
    ).toBeDefined();
    const agents = screen.getByRole("table", { name: "Agents" });
    expect(rowTexts(agents, "agent-local")[0]).toBe("reserved");
    expect(rowTexts(agents, "agent-local")[7]).toBe("Release agent-local");
    expect(screen.getByText("4 agents: 2 reserved, 1 busy, 1 offline")).toBeDefined();

    await user.click(screen.getByRole("button", { name: "Release agent-local" }));
    expect(await screen.findByText("Agent agent-local released. It is now online.")).toBeDefined();
    expect(rowTexts(agents, "agent-local")[0]).toBe("online");
    expect(screen.getByRole("button", { name: "Reserve agent-local" })).toBeDefined();
    expect(screen.queryByRole("alert")).toBeNull();
    await expectNoAxeViolations();
  });

  it("releases an agent that another user reserved", async () => {
    const { user } = await renderAgents();
    await user.click(await screen.findByRole("button", { name: "Release agent-held" }));
    expect(await screen.findByText("Agent agent-held released. It is now online.")).toBeDefined();
    expect(screen.getByRole("button", { name: "Reserve agent-held" })).toBeDefined();
  });

  it("reserves an agent again when another caller reserved it meanwhile", async () => {
    const { user, mock } = await renderAgents();
    await screen.findByRole("button", { name: "Reserve agent-local" });
    // The controller reserves for the operator, so a second reservation is not an error.
    expect(await direct(mock, "agent-local", "reserve")).toBe(200);

    await user.click(screen.getByRole("button", { name: "Reserve agent-local" }));

    expect(
      await screen.findByText("Agent agent-local reserved. It is now reserved."),
    ).toBeDefined();
    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("reports a reservation conflict when the agent went offline after the list loaded", async () => {
    const { user, mock } = await renderAgents();
    await screen.findByRole("button", { name: "Reserve agent-local" });
    const agent = mock.agents.find((candidate) => candidate.id === "agent-local");
    if (agent === undefined) throw new Error("the mock lost agent-local");
    agent.state = "offline";

    await user.click(screen.getByRole("button", { name: "Reserve agent-local" }));

    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toBe(
      "Agent agent-local was not reserved because of a conflict. reserve agent agent-local: agent offline",
    );
    // The refresh after the conflict replaces the stale button with the state of the controller.
    await waitFor(() => {
      expect(screen.queryByRole("button", { name: "Reserve agent-local" })).toBeNull();
    });
    const agents = screen.getByRole("table", { name: "Agents" });
    expect(rowTexts(agents, "agent-local")[0]).toBe("offline");
    expect(rowTexts(agents, "agent-local")[7]).toBe("Not connected");
    await expectNoAxeViolations();
  });

  it("reports a reservation conflict when the agent started a run after the list loaded", async () => {
    const { user, mock } = await renderAgents();
    await screen.findByRole("button", { name: "Reserve agent-local" });
    const agent = mock.agents.find((candidate) => candidate.id === "agent-local");
    if (agent === undefined) throw new Error("the mock lost agent-local");
    agent.state = "busy";

    await user.click(screen.getByRole("button", { name: "Reserve agent-local" }));

    expect((await screen.findByRole("alert")).textContent).toBe(
      "Agent agent-local was not reserved because of a conflict. reserve agent agent-local: plan loaded: agent busy",
    );
    await waitFor(() => {
      expect(rowTexts(screen.getByRole("table", { name: "Agents" }), "agent-local")[7]).toBe(
        "In use by a run",
      );
    });
  });

  it("shows the detail of any problem, such as a conflict that names the holder", async () => {
    const detail = "Agent agent-local is reserved by token ci-bot until 2026-11-10T16:00:00Z.";
    const { user } = await renderAgents(AGENTS, {
      faults: [{ method: "POST", path: "/agents/agent-local:reserve", status: 409, detail }],
    });
    await user.click(await screen.findByRole("button", { name: "Reserve agent-local" }));
    expect((await screen.findByRole("alert")).textContent).toBe(
      `Agent agent-local was not reserved because of a conflict. ${detail}`,
    );
  });

  it("shows a problem that is not a conflict without the word conflict", async () => {
    const { user } = await renderAgents(AGENTS, {
      faults: [
        {
          method: "POST",
          path: "/agents/agent-local:reserve",
          status: 500,
          detail: "The controller could not reach its store.",
        },
      ],
    });
    await user.click(await screen.findByRole("button", { name: "Reserve agent-local" }));
    expect((await screen.findByRole("alert")).textContent).toBe(
      "Agent agent-local was not reserved. The controller could not reach its store.",
    );
  });

  it("clears the conflict message when the next action succeeds", async () => {
    const { user, mock } = await renderAgents();
    await screen.findByRole("button", { name: "Reserve agent-local" });
    const agent = mock.agents.find((candidate) => candidate.id === "agent-local");
    if (agent === undefined) throw new Error("the mock lost agent-local");
    agent.state = "offline";
    await user.click(screen.getByRole("button", { name: "Reserve agent-local" }));
    await screen.findByRole("alert");

    await user.click(screen.getByRole("button", { name: "Release agent-held" }));
    expect(await screen.findByText("Agent agent-held released. It is now online.")).toBeDefined();
    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("drops an agent that the controller no longer knows after a 404", async () => {
    const { user, mock } = await renderAgents();
    await screen.findByRole("button", { name: "Release agent-held" });
    mock.agents.splice(
      mock.agents.findIndex((candidate) => candidate.id === "agent-held"),
      1,
    );

    await user.click(screen.getByRole("button", { name: "Release agent-held" }));

    expect((await screen.findByRole("alert")).textContent).toBe(
      "Agent agent-held was not released. release agent agent-held: unknown agent",
    );
    await waitFor(() => {
      expect(screen.queryByRole("rowheader", { name: "agent-held" })).toBeNull();
    });
    expect(screen.getByText("3 agents: 1 online, 1 busy, 1 offline")).toBeDefined();
  });

  it("offers no action for busy and offline agents", async () => {
    await renderAgents();
    const agents = await screen.findByRole("table", { name: "Agents" });
    const buttons = within(agents)
      .getAllByRole("button")
      .map((button) => button.textContent);
    expect(buttons).toEqual(["Reserve agent-local", "Release agent-held"]);
  });

  it("signs out when the controller rejects the token during an action", async () => {
    const { user } = await renderAgents(AGENTS, {
      faults: [{ method: "POST", path: "/agents/agent-local:reserve", status: 401 }],
    });
    await user.click(await screen.findByRole("button", { name: "Reserve agent-local" }));
    expect(await screen.findByRole("heading", { level: 1, name: /Sign in/ })).toBeDefined();
    expect(window.sessionStorage.getItem("fps.token")).toBeNull();
  });
});

/** One agent for each kind of holder, with literal run identifiers. */
const HOLDERS: Agent[] = [
  { id: "agent-free", address: "10.0.1.1:50051", state: "online" },
  { id: "agent-op", address: "10.0.1.2:50051", state: "reserved", reserved_by: "operator" },
  { id: "agent-run", address: "10.0.1.3:50051", state: "reserved", reserved_by: "run_mock0007" },
  { id: "agent-run-busy", address: "10.0.1.4:50051", state: "busy", reserved_by: "run_mock0008" },
  { id: "agent-down", state: "offline", reserved_by: "operator" },
];

describe("reservation holders", () => {
  it("shows the operator, a run with a link to its page, and no holder", async () => {
    await renderAgents(HOLDERS);
    const agents = await screen.findByRole("table", { name: "Agents" });

    expect(rowTexts(agents, "agent-free").slice(6)).toEqual(["none", "Reserve agent-free"]);
    expect(rowTexts(agents, "agent-op").slice(6)).toEqual(["operator", "Release agent-op"]);
    expect(rowTexts(agents, "agent-run").slice(6)).toEqual(["Run run_mock0007", "Held by a run"]);
    // A reservation shows while a busy or offline state takes precedence over `reserved`.
    expect(rowTexts(agents, "agent-run-busy").slice(6)).toEqual([
      "Run run_mock0008",
      "In use by a run",
    ]);
    expect(rowTexts(agents, "agent-down").slice(6)).toEqual(["operator", "Not connected"]);

    const link = within(agents).getByRole("link", { name: "Run run_mock0007" });
    expect(link.getAttribute("href")).toBe("/runs/run_mock0007");
    expect(
      within(agents)
        .getAllByRole("link")
        .map((candidate) => candidate.textContent),
    ).toEqual(["Run run_mock0007", "Run run_mock0008"]);
    // The view offers no release for an agent that a run holds.
    expect(
      within(agents)
        .getAllByRole("button")
        .map((button) => button.textContent),
    ).toEqual(["Reserve agent-free", "Release agent-op"]);
    await expectNoAxeViolations();
  });

  it("shows the operator after a reservation and no holder after the release", async () => {
    const { user } = await renderAgents(HOLDERS);
    const agents = await screen.findByRole("table", { name: "Agents" });
    await user.click(screen.getByRole("button", { name: "Reserve agent-free" }));
    expect(await screen.findByText("Agent agent-free reserved. It is now reserved.")).toBeDefined();
    expect(rowTexts(agents, "agent-free").slice(6)).toEqual(["operator", "Release agent-free"]);

    await user.click(screen.getByRole("button", { name: "Release agent-free" }));
    expect(await screen.findByText("Agent agent-free released. It is now online.")).toBeDefined();
    expect(rowTexts(agents, "agent-free").slice(6)).toEqual(["none", "Reserve agent-free"]);
  });

  it("explains a conflict with the run that took the agent and then shows that run", async () => {
    const { user, mock } = await renderAgents(HOLDERS);
    await screen.findByRole("button", { name: "Reserve agent-free" });
    const agent = mock.agents.find((candidate) => candidate.id === "agent-free");
    if (agent === undefined) throw new Error("the mock lost agent-free");
    agent.state = "reserved";
    agent.reserved_by = "run_mock0009";

    await user.click(screen.getByRole("button", { name: "Reserve agent-free" }));

    expect((await screen.findByRole("alert")).textContent).toBe(
      "Agent agent-free was not reserved because of a conflict. " +
        "reserve agent agent-free for operator: held by run_mock0009: agent reserved by another holder",
    );
    const agents = screen.getByRole("table", { name: "Agents" });
    await waitFor(() => {
      expect(rowTexts(agents, "agent-free").slice(6)).toEqual([
        "Run run_mock0009",
        "Held by a run",
      ]);
    });
    expect(
      within(agents).getByRole("link", { name: "Run run_mock0009" }).getAttribute("href"),
    ).toBe("/runs/run_mock0009");
    await expectNoAxeViolations();
  });

  it("explains an offline conflict when the agent went offline after the list loaded", async () => {
    const { user, mock } = await renderAgents(HOLDERS);
    await screen.findByRole("button", { name: "Reserve agent-free" });
    const agent = mock.agents.find((candidate) => candidate.id === "agent-free");
    if (agent === undefined) throw new Error("the mock lost agent-free");
    agent.state = "offline";

    await user.click(screen.getByRole("button", { name: "Reserve agent-free" }));

    expect((await screen.findByRole("alert")).textContent).toBe(
      "Agent agent-free was not reserved because of a conflict. reserve agent agent-free: agent offline",
    );
    await expectNoAxeViolations();
  });
});
