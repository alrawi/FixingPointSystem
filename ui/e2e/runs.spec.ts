// SPDX-License-Identifier: GPL-3.0-only

import AxeBuilder from "@axe-core/playwright";
import { expect, test, type Page } from "@playwright/test";
import { MOCK_TOKEN } from "../mock/server";

// A real browser reads the live stream of the mock server, which ticks once per
// second, and the color contrast rule runs here, which jsdom cannot do.
async function expectAccessible(page: Page) {
  const results = await new AxeBuilder({ page }).analyze();
  expect(results.violations.map((violation) => `${violation.id}: ${violation.help}`)).toEqual([]);
}

test("starts a run, follows its live metrics, and stops it", async ({ page }) => {
  await page.goto("/");
  await page.getByLabel("API token").fill(MOCK_TOKEN);
  await page.getByRole("button", { name: "Sign in" }).click();
  await page.getByRole("navigation", { name: "Main" }).getByRole("link", { name: "Runs" }).click();
  await expect(page.getByRole("heading", { level: 1, name: "Runs" })).toBeVisible();
  await expectAccessible(page);

  await page.getByLabel("Configuration").selectOption("cfg_http_smoke");
  await page.getByRole("button", { name: "Start run" }).click();
  await expect(page.getByRole("heading", { level: 1, name: /^Run run_/ })).toBeVisible();

  // The run walks through its states and the stream starts to deliver.
  const state = page.getByRole("region", { name: "State" });
  await expect(state.locator(".state")).toHaveText("running", { timeout: 15_000 });
  await expect(page.locator(".feed-status")).toContainText("Live", { timeout: 15_000 });
  await expect(page.locator(".counter", { hasText: "Sessions attempted" })).not.toContainText("-");
  await expect
    .poll(
      async () =>
        page
          .locator('polyline[data-series="sessions.response_time.p50"]')
          .getAttribute("data-values"),
      { timeout: 15_000 },
    )
    .toMatch(/^\d+,\d+/);
  await expectAccessible(page);

  await page.getByRole("button", { name: "Stop run" }).click();
  await expect(state.locator(".state")).toHaveText("aborted", { timeout: 15_000 });
  await expect(state).toContainText("stopped by request");
  await expectAccessible(page);
});
