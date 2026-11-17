// SPDX-License-Identifier: GPL-3.0-only

import { expect, test } from "@playwright/test";
import { AGENT_LOCAL, AGENT_SPARE } from "./constants";
import { expectAccessible, openFromNavigation, rest, signIn } from "./helpers";

// The agents view against the real controller and its two fake agents.

test.afterEach(async () => {
  // Leave the agent as the other specs expect it. A release of an agent that is not reserved succeeds.
  await rest("POST", `/v1/agents/${AGENT_SPARE}:release`);
});

test("lists the agents that registered over gRPC and their ports", async ({ page }) => {
  await signIn(page);
  await openFromNavigation(page, "Agents");
  await expect(page.getByRole("heading", { level: 1, name: "Agents" })).toBeVisible();
  const table = page.getByRole("table", { name: "Agents" });
  for (const id of [AGENT_LOCAL, AGENT_SPARE]) {
    const row = table.getByRole("row", { name: new RegExp(`^${id} `) });
    await expect(row).toContainText("online");
    await expect(row.getByRole("button", { name: `Reserve ${id}` })).toBeEnabled();
  }
  await expect(page.getByText("2 agents: 2 online")).toBeVisible();
  const ports = page.getByRole("table", { name: "Ports" });
  await expect(ports.getByRole("row", { name: new RegExp(`^${AGENT_LOCAL} lo `) })).toContainText(
    "loopback",
  );
  await expectAccessible(page);
});

test("reserves and releases an agent, and the controller keeps the reservation", async ({
  page,
}) => {
  await signIn(page);
  await openFromNavigation(page, "Agents");
  const row = page.getByRole("table", { name: "Agents" }).getByRole("row", {
    name: new RegExp(`^${AGENT_SPARE} `),
  });
  await row.getByRole("button", { name: `Reserve ${AGENT_SPARE}` }).click();
  await expect(page.getByRole("status").filter({ hasText: "Agent spare reserved." })).toContainText(
    "It is now reserved.",
  );
  await expect(row).toContainText("reserved");
  await expect(row).toContainText("operator");
  await expectAccessible(page);

  // The state lives in the controller, so a new page load shows it.
  await page.reload();
  const again = page.getByRole("table", { name: "Agents" }).getByRole("row", {
    name: new RegExp(`^${AGENT_SPARE} `),
  });
  await expect(again).toContainText("reserved");
  await again.getByRole("button", { name: `Release ${AGENT_SPARE}` }).click();
  await expect(page.getByRole("status").filter({ hasText: "Agent spare released." })).toContainText(
    "It is now online.",
  );
  await expect(again).toContainText("online");
});
