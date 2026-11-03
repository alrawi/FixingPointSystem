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

test("browses the module library with the keyboard", async ({ page }) => {
  await page.goto("/");
  await page.getByLabel("API token").fill(MOCK_TOKEN);
  await page.getByRole("button", { name: "Sign in" }).click();
  await expect(page.getByRole("heading", { level: 1, name: "Capabilities" })).toBeVisible();

  const nav = page.getByRole("navigation", { name: "Main" });
  await nav.getByRole("link", { name: "Modules" }).focus();
  await page.keyboard.press("Enter");
  await expect(page.getByRole("heading", { level: 1, name: "Modules" })).toBeFocused();
  const table = page.getByRole("table", { name: "Installed modules" });
  await expect(table.getByRole("link", { name: "http1 0.1.0" })).toBeVisible();
  await expectAccessible(page);

  await table.getByRole("link", { name: "http1 0.1.0" }).focus();
  await page.keyboard.press("Enter");
  await expect(page.getByRole("heading", { level: 1, name: "Module http1 0.1.0" })).toBeFocused();
  await expect(page.getByRole("table", { name: "Parameters" })).toBeVisible();
  await expect(page.getByRole("table", { name: "Functions" })).toBeVisible();
  await expectAccessible(page);
});
