import { dirname } from "node:path";
import { mkdirSync } from "node:fs";

import { expect, test as setup } from "@playwright/test";

import { E2E_PASSWORD, E2E_STORAGE_STATE, E2E_USERNAME } from "./support/env";

/**
 * Authenticates once through the real login UI and stores the resulting cookie
 * session for the other projects. The state file is generated at test time and
 * is gitignored; no credentials or storage state are committed.
 */
setup("authenticate through the login UI", async ({ page }) => {
  await page.goto("/login");

  await page.getByLabel("Login Name").fill(E2E_USERNAME);
  await page.getByLabel("Password", { exact: true }).fill(E2E_PASSWORD);
  await page.getByRole("button", { name: "Sign In" }).click();

  await expect(page).toHaveURL(/\/search(?:[/?#]|$)/);

  mkdirSync(dirname(E2E_STORAGE_STATE), { recursive: true });
  await page.context().storageState({ path: E2E_STORAGE_STATE });
});
