// SPDX-License-Identifier: GPL-3.0-only

import { expect, test } from "@playwright/test";
import { expectAccessible, openFromNavigation, runToEnd, signIn, smokeConfig } from "./helpers";

// The report flow against the real controller. Two runs of the fake agent give
// a report that passes and a report that fails a criterion. The suite makes them
// once, through the REST API, so each test only reads.

let passing = "";
let failing = "";

test.beforeAll(async () => {
  test.setTimeout(240_000);
  const pass = await runToEnd(smokeConfig("ui_e2e_report_pass"));
  expect(pass.run.state).toBe("completed");
  passing = pass.run.id;
  // The fake agent attempts sessions, so this criterion cannot hold.
  const fail = await runToEnd(
    smokeConfig("ui_e2e_report_fail", '  - { metric: sessions.attempted, op: "<", value: 0 }'),
  );
  expect(fail.run.state).toBe("failed");
  failing = fail.run.id;
});

test("shows the report of a run that passed", async ({ page }) => {
  await signIn(page);
  await page.goto(`/runs/${passing}/report`);
  await expect(
    page.getByRole("heading", { level: 1, name: `Report of run ${passing}` }),
  ).toBeVisible();
  await expect(page.getByRole("region", { name: "Verdict" })).toContainText("Passed");

  const facts = page.getByRole("region", { name: "Run", exact: true });
  await expect(facts).toContainText("completed");

  const criteria = page.getByRole("table", { name: "Criteria" });
  const criterion = criteria.getByRole("row", { name: /sessions\.failed_ratio == 0/ });
  await expect(criterion).toContainText("Passed");

  // The fake agent reports client side counters for one scenario.
  const scenarios = page.getByRole("table", { name: "Scenarios" });
  await expect(scenarios.getByRole("row", { name: /http1\/web_get/ })).toContainText(/[1-9]/);
  await expect(
    page
      .getByRole("region", { name: "Totals" })
      .locator(".counter", { hasText: "Sessions attempted" }),
  ).toContainText(/[1-9]/);
  await expectAccessible(page);
});

test("shows the report of a run that failed a criterion", async ({ page }) => {
  await signIn(page);
  await page.goto(`/runs/${failing}/report`);
  await expect(page.getByRole("region", { name: "Verdict" })).toContainText("Failed");
  await expect(page.getByRole("region", { name: "Verdict" })).toContainText("criteria failed");
  const criterion = page
    .getByRole("table", { name: "Criteria" })
    .getByRole("row", { name: /sessions\.attempted < 0/ });
  await expect(criterion).toContainText("Failed");
  await expect(page.getByRole("region", { name: "Run", exact: true })).toContainText("failed");
  await expectAccessible(page);
});

test("opens a report from the list of runs and the live view", async ({ page }) => {
  await signIn(page);
  await openFromNavigation(page, "Runs");
  await page
    .getByRole("table", { name: "Runs" })
    .getByRole("link", { name: `Report of ${passing}` })
    .click();
  await expect(
    page.getByRole("heading", { level: 1, name: `Report of run ${passing}` }),
  ).toBeVisible();
  await page.getByRole("link", { name: "Live view" }).click();
  await expect(page.getByRole("heading", { level: 1, name: `Run ${passing}` })).toBeVisible();
  await page.getByRole("link", { name: "Report", exact: true }).click();
  await expect(
    page.getByRole("heading", { level: 1, name: `Report of run ${passing}` }),
  ).toBeVisible();
});

test("compares the passing run with the failing run", async ({ page }) => {
  await signIn(page);
  await page.goto(`/runs/${passing}/report`);
  await page.getByRole("link", { name: "Compare with another run" }).click();
  await expect(page.getByRole("heading", { level: 1, name: "Compare runs" })).toBeVisible();
  await expect(page.getByLabel("Run A")).toHaveValue(passing);

  await page.getByLabel("Run B").selectOption(failing);
  const facts = page.getByRole("table", { name: "Runs compared" });
  const verdict = facts.getByRole("row", { name: /^Verdict/ });
  await expect(verdict).toContainText("Passed");
  await expect(verdict).toContainText("Failed");
  await expect(facts.getByRole("row", { name: /^State/ })).toContainText("completed");
  await expect(facts.getByRole("row", { name: /^State/ })).toContainText("failed");
  await expect(page.getByRole("table", { name: "Totals compared" })).toBeVisible();
  // Each run has a criterion that the other lacks.
  const compared = page.getByRole("table", { name: "Criteria compared" });
  await expect(compared).toContainText("sessions.failed_ratio == 0");
  await expect(compared).toContainText("sessions.attempted < 0");
  await expect(page).toHaveURL(new RegExp(`a=${passing}&b=${failing}|b=${failing}&a=${passing}`));
  await expectAccessible(page);
});
