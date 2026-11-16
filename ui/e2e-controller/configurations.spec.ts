// SPDX-License-Identifier: GPL-3.0-only

import { expect, test } from "@playwright/test";
import { createConfig, expectAccessible, openFromNavigation, signIn, smokeConfig } from "./helpers";

// The configuration flow against the real controller: the controller stores,
// lists, and validates what the editor sends, and the editor shows its answers.

test("creates, validates, saves, and lists a configuration", async ({ page }) => {
  await signIn(page);
  await openFromNavigation(page, "Configurations");
  await expect(page.getByRole("heading", { level: 1, name: "Configurations" })).toBeVisible();
  await page.getByRole("link", { name: "New configuration" }).click();
  await expect(page.getByRole("status").first()).toContainText("passes the schema check");

  const name = page.getByRole("region", { name: "General" }).getByLabel("Name");
  await name.fill("ui_e2e_created");
  await expect(page.getByRole("textbox", { name: "YAML" })).toHaveValue(
    / {2}name: ui_e2e_created\n/,
  );

  // The controller validates only stored configurations, so the editor stores the text first.
  await page.getByRole("button", { name: "Validate on controller" }).click();
  await expect(page.getByText(/The controller accepts the configuration \S+\./)).toBeVisible();
  await expectAccessible(page);

  await page.getByRole("button", { name: "Save as new" }).click();
  const saved = page.getByText(/Saved as a new configuration with the id \S+\./);
  await expect(saved).toBeVisible();
  const id = /with the id (\S+)\./.exec((await saved.textContent()) ?? "")?.[1];
  expect(id).toBeTruthy();

  // The list reads the controller, so the stored configuration is in it with the id that the notice named.
  await page.getByRole("link", { name: "Back to the configurations" }).click();
  const table = page.getByRole("table", { name: "Stored configurations" });
  const row = table.getByRole("row", { name: new RegExp(`ui_e2e_created.*${id ?? ""}`) });
  await expect(row).toBeVisible();
  await expectAccessible(page);

  // The name opens the editor on the stored copy, which the controller returned as JSON.
  await row.getByRole("link", { name: "ui_e2e_created" }).click();
  await expect(
    page.getByRole("heading", { level: 1, name: "Configuration ui_e2e_created" }),
  ).toBeVisible();
  await expect(page.getByRole("textbox", { name: "YAML" })).toHaveValue(
    / {2}name: ui_e2e_created\n/,
  );
  await expect(page.getByRole("region", { name: "Timeline" }).getByLabel("Seed")).toHaveValue("1");
});

test("shows the findings of the controller next to the field", async ({ page }) => {
  // The schema accepts any scenario name. Only the controller knows the registry.
  const yaml = smokeConfig("ui_e2e_unknown_scenario").replace(
    "http1/web_get",
    "http1/no_such_scenario",
  );
  const id = await createConfig(yaml);
  await signIn(page);
  await page.goto(`/configs/${id}`);
  await page.getByRole("button", { name: "Validate on controller" }).click();
  await expect(page.getByRole("alert")).toContainText("found problems");
  await expect(
    page.getByRole("region", { name: "Profile" }).getByLabel("Scenario", { exact: true }),
  ).toHaveAttribute("aria-invalid", "true");
  await expectAccessible(page);
});

test("a configuration that the controller stored keeps its values in the form", async ({
  page,
}) => {
  const id = await createConfig(smokeConfig("ui_e2e_roundtrip"));
  await signIn(page);
  await page.goto(`/configs/${id}`);
  await expect(page.getByRole("region", { name: "General" }).getByLabel("Name")).toHaveValue(
    "ui_e2e_roundtrip",
  );
  await expect(page.getByRole("region", { name: "Timeline" }).getByLabel("Seed")).toHaveValue("7");
  await expect(page.getByRole("status").first()).toContainText("passes the schema check");
});
