// SPDX-License-Identifier: GPL-3.0-only

import AxeBuilder from "@axe-core/playwright";
import { expect, test, type Page } from "@playwright/test";
import { MOCK_TOKEN } from "../mock/server";

async function expectAccessible(page: Page) {
  const results = await new AxeBuilder({ page }).analyze();
  expect(results.violations.map((violation) => `${violation.id}: ${violation.help}`)).toEqual([]);
}

test("signs in with a token and shows the capabilities of the mock server", async ({ page }) => {
  await page.goto("/");
  await expect(page.getByRole("heading", { level: 1, name: /Sign in/ })).toBeVisible();
  await expectAccessible(page);

  await page.getByLabel("API token").fill(MOCK_TOKEN);
  await page.getByRole("button", { name: "Sign in" }).click();

  await expect(page.getByRole("heading", { level: 1, name: "Capabilities" })).toBeVisible();
  const modules = page.getByRole("region", { name: "Modules" });
  await expect(modules.getByRole("rowheader", { name: "http1" })).toBeVisible();
  await expect(modules.getByRole("rowheader", { name: "dns" })).toBeVisible();
  await expect(
    page.getByRole("region", { name: "Agents" }).getByRole("rowheader", { name: "agent-local" }),
  ).toBeVisible();
  await expectAccessible(page);
});

test("the skip link moves focus to the main region", async ({ page }) => {
  await page.goto("/");
  await page.getByLabel("API token").fill(MOCK_TOKEN);
  await page.getByRole("button", { name: "Sign in" }).click();
  await expect(page.getByRole("heading", { level: 1, name: "Capabilities" })).toBeVisible();

  await page.keyboard.press("Tab");
  const skip = page.getByRole("link", { name: "Skip to content" });
  await expect(skip).toBeFocused();
  await page.keyboard.press("Enter");
  await expect(page.getByRole("main")).toBeFocused();
});
