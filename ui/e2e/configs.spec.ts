// SPDX-License-Identifier: GPL-3.0-only

import AxeBuilder from "@axe-core/playwright";
import { expect, test, type Page } from "@playwright/test";
import { MOCK_TOKEN } from "../mock/server";

// Unlike the component tests, a real browser computes layout, so these checks
// include the color contrast rule.
async function expectAccessible(page: Page) {
  const results = await new AxeBuilder({ page }).analyze();
  expect(results.violations.map((violation) => `${violation.id}: ${violation.help}`)).toEqual([]);
}

async function signIn(page: Page) {
  await page.goto("/");
  await page.getByLabel("API token").fill(MOCK_TOKEN);
  await page.getByRole("button", { name: "Sign in" }).click();
  await expect(page.getByRole("heading", { level: 1, name: "Capabilities" })).toBeVisible();
}

test("edits a configuration through a form and the YAML view", async ({ page }) => {
  await signIn(page);
  await page
    .getByRole("navigation", { name: "Main" })
    .getByRole("link", { name: "Configurations" })
    .click();
  await expect(page.getByRole("table", { name: "Stored configurations" })).toBeVisible();
  await expectAccessible(page);

  await page.getByRole("link", { name: "New configuration" }).click();
  await expect(page.getByRole("heading", { level: 2, name: "YAML" })).toBeVisible();
  await expect(page.getByRole("status").first()).toContainText("passes the schema check");
  await expectAccessible(page);

  // A form edit shows in the YAML, and a YAML edit shows in the form.
  const yaml = page.getByRole("textbox", { name: "YAML" });
  const name = page.getByRole("region", { name: "General" }).getByLabel("Name");
  await name.fill("edited_in_form");
  await expect(yaml).toHaveValue(/ {2}name: edited_in_form\n/);
  const text = await yaml.inputValue();
  await yaml.fill(text.replace("timeline: { seed: 1 }", "timeline: { seed: 99 }"));
  await expect(page.getByRole("region", { name: "Timeline" }).getByLabel("Seed")).toHaveValue("99");

  // An error shows inline and in the list of checks, and it passes the contrast rule.
  const weight = page.getByRole("region", { name: "Profile" }).getByLabel("Weight");
  await weight.fill("many");
  await expect(weight).toHaveAttribute("aria-invalid", "true");
  await expect(page.getByRole("region", { name: "Checks" })).toContainText("/profile/mix/0/weight");
  await expect(page.getByRole("button", { name: "Save as new" })).toBeDisabled();
  await expectAccessible(page);

  await weight.fill("3");
  await page.getByRole("button", { name: "Save as new" }).click();
  await expect(page.getByText(/Saved as a new configuration with the id cfg_/)).toBeVisible();
});

test("shows the findings of the controller for a stored configuration", async ({ page }) => {
  await signIn(page);
  await page.goto("/configs/cfg_dns_zone");
  await page.getByRole("button", { name: "Validate on controller" }).click();
  await expect(page.getByRole("alert")).toContainText("found problems");
  await expect(
    page.getByRole("region", { name: "Profile" }).getByLabel("Scenario", { exact: true }),
  ).toHaveAttribute("aria-invalid", "true");
  await expectAccessible(page);
});
