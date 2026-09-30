import { expect, test, type Page } from "@playwright/test";

/**
 * Settings smoke coverage for the authenticated settings routes plus the
 * PostgreSQL-backed runtime chunking save path.
 *
 * The settings tests read and write the same runtime settings, so they run
 * serially to avoid concurrent settings races.
 */
test.describe.configure({ mode: "serial" });

const SETTINGS_SECTIONS = [
  { path: "/settings/appearance", region: "Appearance" },
  { path: "/settings/access-tokens", region: "Access Tokens" },
  { path: "/settings/search", region: "Search" },
  { path: "/settings/runtime", region: "Runtime" },
  { path: "/settings/docling", region: "Docling" },
  { path: "/settings/translation", region: "Translation and document enrichment" },
] as const;

type RuntimeSavePayload = {
  chunking?: { max_chars?: number };
};

function runtimeSettingsPut(page: Page) {
  return page.waitForResponse(
    (response) => response.request().method() === "PUT"
      && new URL(response.url()).pathname === "/v1/settings/runtime",
  );
}

async function restoreChunkingMaxChars(page: Page, original: number): Promise<void> {
  const maxChars = page.getByRole("spinbutton", { name: "Max Characters", exact: true });
  const saveButton = page.getByTestId("settings-save");

  const current = Number((await maxChars.inputValue()).replace(/,/g, ""));
  if (current !== original) {
    await maxChars.click();
    await maxChars.press("Control+a");
    await maxChars.pressSequentially(String(original));
    await page.keyboard.press("Tab");
    await expect(maxChars).toHaveAttribute("aria-valuenow", String(original));
  }

  if (await saveButton.isEnabled()) {
    const saved = runtimeSettingsPut(page);
    await saveButton.click();
    await saved;
  }

  await expect(saveButton).toBeDisabled();
}

test("navigates the authenticated settings sections", async ({ page }) => {
  await page.goto("/search");

  // Reach settings through the real sidebar link, proving authenticated
  // in-app navigation into the section routes.
  await page.getByRole("link", { name: "Settings", exact: true }).click();
  await expect(page).toHaveURL(/\/settings\/appearance$/);

  for (const section of SETTINGS_SECTIONS) {
    await page.goto(section.path);
    await expect(page).toHaveURL(new RegExp(`${section.path}$`));
    await expect(page.getByRole("region", { name: section.region, exact: true })).toBeVisible();
  }
});

test("saves runtime chunking settings and restores the original value", async ({ page }) => {
  await page.goto("/settings/runtime");
  await expect(page.getByRole("region", { name: "Runtime", exact: true })).toBeVisible();

  const saveButton = page.getByTestId("settings-save");
  const maxChars = page.getByRole("spinbutton", { name: "Max Characters", exact: true });
  const pendingBadge = page.getByText("Unsaved changes");
  const successBadge = page.getByTestId("settings-page-scroll").getByText("Settings saved");

  const original = Number((await maxChars.inputValue()).replace(/,/g, ""));
  const updated = original + 1;

  try {
    // Clean state: nothing pending and Save is inert.
    await expect(saveButton).toBeDisabled();
    await expect(pendingBadge).toHaveCount(0);

    // Dirty state: a real interaction with the chunking field enables Save.
    await maxChars.press("ArrowUp");
    await expect(maxChars).toHaveAttribute("aria-valuenow", String(updated));
    await expect(pendingBadge).toBeVisible();
    await expect(saveButton).toBeEnabled();

    // Save path: one PUT of the runtime settings that carries the new chunking value.
    const saved = runtimeSettingsPut(page);
    await saveButton.click();
    const response = await saved;
    expect(response.ok()).toBe(true);

    const payload = response.request().postDataJSON() as RuntimeSavePayload;
    expect(payload.chunking?.max_chars).toBe(updated);

    await expect(successBadge).toBeVisible();
    await expect(saveButton).toBeDisabled();
  } finally {
    await restoreChunkingMaxChars(page, original);
  }
});
