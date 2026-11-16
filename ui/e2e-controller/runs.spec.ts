// SPDX-License-Identifier: GPL-3.0-only

import { expect, test, type Page } from "@playwright/test";
import {
  createConfig,
  expectAccessible,
  openFromNavigation,
  signIn,
  smokeConfig,
  startRun,
  waitForAgentOnline,
  waitForFinalRun,
} from "./helpers";

// The run flow against the real controller and the fake agent `local`. The fake
// agent reports simulated counters while it runs, and it ends its plan on its
// own ten seconds after it started, as an engine does after its last phase.

test.beforeEach(async () => {
  // A run that an earlier test ended may still hold the agent for a moment.
  await waitForAgentOnline();
});

async function startFromForm(page: Page, configId: string) {
  await openFromNavigation(page, "Runs");
  await expect(page.getByRole("heading", { level: 1, name: "Runs" })).toBeVisible();
  await page.getByLabel("Configuration").selectOption(configId);
  await page.getByRole("checkbox", { name: /^local \(online\)/ }).check();
  await page.getByRole("button", { name: "Start run" }).click();
  await expect(page.getByRole("heading", { level: 1, name: /^Run \S+$/ })).toBeVisible();
  const heading = (await page.getByRole("heading", { level: 1 }).textContent()) ?? "";
  return heading.replace(/^Run /, "");
}

test("starts a run, follows its live metrics, and sees it complete", async ({ page }) => {
  const configId = await createConfig(smokeConfig("ui_e2e_live"));
  await signIn(page);
  const runId = await startFromForm(page, configId);
  expect(runId).not.toBe("");

  // The run walks through its states and the stream of the controller starts to deliver.
  const state = page.getByRole("region", { name: "State" });
  await expect(state.locator(".state")).toHaveText("running", { timeout: 30_000 });
  await expect(page.locator(".feed-status")).toContainText("Live", { timeout: 30_000 });
  await expect(page.locator(".counter", { hasText: "Sessions attempted" })).toContainText(/[1-9]/, {
    timeout: 30_000,
  });
  await expect(page.locator(".counter", { hasText: "Sessions succeeded" })).toContainText(/[1-9]/);
  await expectAccessible(page);

  // The fake agent ends its plan on its own, and the run completes.
  await expect(state.locator(".state")).toHaveText("completed", { timeout: 45_000 });
  await expect(page.locator(".feed-status")).toContainText("ended", { timeout: 15_000 });
  const transitions = page.getByRole("list", { name: "State transitions" });
  await expect(transitions).toContainText("running");
  await expect(transitions).toContainText("completed (final)");
  await expect(page.getByRole("button", { name: "Stop run" })).toHaveCount(0);
  await expectAccessible(page);

  // The finished run is in the list, and its report link opens the report.
  await page.getByRole("link", { name: "All runs" }).click();
  const row = page
    .getByRole("table", { name: "Runs" })
    .getByRole("row", { name: new RegExp(runId) });
  await expect(row).toContainText("completed");
  await row.getByRole("link", { name: `Report of ${runId}` }).click();
  await expect(
    page.getByRole("heading", { level: 1, name: `Report of run ${runId}` }),
  ).toBeVisible();
});

test("stops a run before the fake agent ends it", async ({ page }) => {
  const configId = await createConfig(smokeConfig("ui_e2e_stop"));
  await signIn(page);
  const runId = await startFromForm(page, configId);

  const state = page.getByRole("region", { name: "State" });
  await expect(state.locator(".state")).toHaveText("running", { timeout: 30_000 });
  await page.getByRole("button", { name: "Stop run" }).click();
  // A graceful stop ends the run as completed, and the controller still evaluates its criteria.
  await expect(state.locator(".state")).toHaveText("completed", { timeout: 30_000 });
  await expect(page.getByRole("button", { name: "Stop run" })).toHaveCount(0);
  await expectAccessible(page);
  expect((await waitForFinalRun(runId)).state).toBe("completed");
});

test("opens a run that started elsewhere in the middle of its life", async ({ page }) => {
  const configId = await createConfig(smokeConfig("ui_e2e_midrun"));
  const run = await startRun(configId);
  await signIn(page);
  await page.goto(`/runs/${run.id}`);
  await expect(page.getByRole("heading", { level: 1, name: `Run ${run.id}` })).toBeVisible();
  await expect(page.getByRole("region", { name: "State" }).locator(".state")).toHaveText(
    "completed",
    { timeout: 60_000 },
  );
  await expect(page.locator(".counter", { hasText: "Sessions attempted" })).toContainText(/[1-9]/);
});

test("reports a run that does not exist", async ({ page }) => {
  await signIn(page);
  await page.goto("/runs/run_does_not_exist");
  await expect(page.getByRole("alert")).toContainText("No run has this id.");
});

test("offers the agents of the controller and starts only with a configuration", async ({
  page,
}) => {
  await signIn(page);
  await openFromNavigation(page, "Runs");
  await expect(page.getByRole("button", { name: "Start run" })).toBeDisabled();
  await expect(page.getByRole("checkbox", { name: /^local \(online\)/ })).toBeEnabled();
  await expect(page.getByRole("checkbox", { name: /^spare \(online\)/ })).toBeEnabled();
  await expectAccessible(page);
});
