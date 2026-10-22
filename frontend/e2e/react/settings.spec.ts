import { expect, test } from "@playwright/test";

import { ADMIN_USER, MEMBER_USER } from "./fixtures";
import { installAppTransport, recordExternalRequests } from "./transport";

/**
 * Section navigation keeps the shared drafts (the provider sits above the
 * outlet), but an unsaved draft still asks for confirmation first.
 */
async function openSettingsTab(page: import("@playwright/test").Page, name: string) {
  await page.getByRole("tab", { name, exact: true }).click();
  const leave = page.getByRole("alertdialog").getByRole("button", { name: "Leave" }).last();
  try {
    await leave.waitFor({ state: "visible", timeout: 2_000 });
    await leave.click();
  } catch {
    // No pending draft: the navigation already completed.
  }
}

test.describe("react settings", () => {
  test("saves appearance and keeps it after a reload", async ({ page }) => {
    const transport = await installAppTransport(page, { settings: true, user: ADMIN_USER });
    const external = recordExternalRequests(page);

    await page.goto("/settings/appearance");
    await expect(page.getByRole("heading", { name: "Appearance" })).toBeVisible();

    await page.getByTestId("settings-theme-select").click();
    await page.getByRole("option", { name: "Dark" }).click();
    await expect(page.getByText("Unsaved changes")).toBeVisible();
    await expect(page.locator("html")).not.toHaveAttribute("data-theme", "dark");

    await page.getByTestId("settings-save").click();
    await expect(page.getByText("Settings saved").first()).toBeVisible();
    await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");

    await page.reload();
    await expect(page.getByTestId("settings-theme-select")).toContainText("Dark");
    expect(external).toEqual([]);
    expect(transport.unmocked).toEqual([]);
  });

  test("discards an unsaved draft on reload", async ({ page }) => {
    await installAppTransport(page, { settings: true, user: ADMIN_USER });

    await page.goto("/settings/search");
    const candidates = page.getByLabel("Rerank Candidates");
    await expect(candidates).toHaveValue("40");
    await candidates.fill("55");
    await expect(page.getByText("Unsaved changes")).toBeVisible();

    await page.reload();
    await expect(page.getByLabel("Rerank Candidates")).toHaveValue("40");
    await expect(page.getByText("Unsaved changes")).toHaveCount(0);
  });

  test("warns instead of claiming a save when the browser denies storage", async ({ page }) => {
    await installAppTransport(page, { settings: true, user: ADMIN_USER });
    await page.addInitScript(() => {
      Storage.prototype.setItem = () => {
        throw new DOMException("Storage denied", "SecurityError");
      };
    });

    await page.goto("/settings/appearance");
    await page.getByTestId("settings-theme-select").click();
    await page.getByRole("option", { name: "Dark" }).click();
    await page.getByTestId("settings-save").click();

    await expect(page.getByText(/could not store the appearance/).first()).toBeVisible();
    await expect(page.getByTestId("settings-save")).toBeEnabled();
    await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
  });

  test("reports a failed connection probe", async ({ page }) => {
    await installAppTransport(page, { settings: true, user: ADMIN_USER, embeddingTestFails: true });

    await page.goto("/settings/runtime");
    await page.getByTestId("runtime-embedding-test").click();

    await expect(page.getByText("embedding refused").first()).toBeVisible();
  });

  test("saves the runtime fields and search page through one shared Save", async ({ page }) => {
    const transport = await installAppTransport(page, { settings: true, user: ADMIN_USER });

    await page.goto("/settings/runtime");
    await page.getByLabel("Embedding Model").fill("embed-2");

    await page.locator('a[href="/settings/search"]').first().click();
    const dialog = page.getByRole("alertdialog");
    await expect(dialog).toBeVisible();
    await dialog.getByRole("button", { name: "Leave" }).click();

    await page.getByLabel("Rerank Candidates").fill("55");
    await page.getByTestId("settings-save").click();
    await expect(page.getByText("Settings saved").first()).toBeVisible();

    const runtimePut = transport.calls.find((call) => call.method === "PUT" && call.path === "/v1/settings/runtime");
    const searchPut = transport.calls.find((call) => call.method === "PUT" && call.path === "/v1/settings/search");
    expect((runtimePut?.body as { embedding: { model: string } }).embedding.model).toBe("embed-2");
    expect((searchPut?.body as { candidate_limit: number }).candidate_limit).toBe(55);
    await expect(page.getByTestId("settings-save")).toBeDisabled();
  });

  test("reveals a created access token once and revokes it after confirmation", async ({ page }) => {
    await installAppTransport(page, { settings: true, user: ADMIN_USER });

    await page.goto("/settings/access-tokens");
    await page.getByTestId("personal-access-token-name").fill("deploy");
    await page.getByTestId("personal-access-token-create").click();

    await expect(page.getByTestId("personal-access-token-secret")).toHaveValue("c69_revealed_once");
    await page.getByTestId("settings-page-scroll").getByRole("button", { name: "Close" }).click();
    await expect(page.getByTestId("personal-access-token-secret")).toHaveCount(0);

    await page.getByRole("button", { name: "Revoke" }).first().click();
    await page.getByRole("alertdialog").getByRole("button", { name: "Revoke" }).click();
    await expect(page.getByText("Revoked").first()).toBeVisible();
  });

  test("localizes every access-token lifetime and keeps the numeric request", async ({ page }) => {
    const transport = await installAppTransport(page, { settings: true, user: ADMIN_USER });
    await page.addInitScript(() => {
      if (!window.localStorage.getItem("context69.locale")) {
        window.localStorage.setItem("context69.locale", "en");
      }
      window.localStorage.setItem("context69.theme", "light");
    });

    await page.goto("/settings/access-tokens");
    const trigger = page.getByTestId("personal-access-token-expiry");
    await expect(trigger).toContainText("30 days");
    await trigger.click();
    const options = page.getByRole("option");
    await expect(options.nth(0)).toHaveText("7 days");
    await expect(options.nth(1)).toHaveText("30 days");
    await expect(options.nth(2)).toHaveText("90 days");
    await expect(options.nth(3)).toHaveText("365 days");
    await page.getByRole("option", { name: "90 days" }).click();
    await expect(trigger).toContainText("90 days");

    await page.evaluate(() => window.localStorage.setItem("context69.locale", "zh-CN"));
    await page.reload();
    const chineseTrigger = page.getByTestId("personal-access-token-expiry");
    await expect(chineseTrigger).toContainText("30天");
    await chineseTrigger.click();
    const chineseOptions = page.getByRole("option");
    await expect(chineseOptions.nth(0)).toHaveText("7天");
    await expect(chineseOptions.nth(1)).toHaveText("30天");
    await expect(chineseOptions.nth(2)).toHaveText("90天");
    await expect(chineseOptions.nth(3)).toHaveText("365天");
    await expect(page.locator("body")).not.toContainText("expiry7Days");
    await page.getByRole("option", { name: "90天" }).click();
    await expect(chineseTrigger).toContainText("90天");

    await page.getByTestId("personal-access-token-name").fill("quarter");
    await page.getByTestId("personal-access-token-create").click();
    await expect(page.getByTestId("personal-access-token-secret")).toHaveValue("c69_revealed_once");

    const create = transport.calls.find((call) => (
      call.method === "POST" && call.path === "/v1/auth/personal-access-tokens"
    ));
    expect((create?.body as { expires_in_days: number }).expires_in_days).toBe(90);
  });

  test("hides admin-only sections from a non-admin and redirects their URL", async ({ page }) => {
    await installAppTransport(page, { settings: true, user: MEMBER_USER });

    await page.goto("/settings/appearance");
    await expect(page.getByRole("link", { name: "Appearance" })).toBeVisible();
    await expect(page.getByRole("link", { name: "Admin Users" })).toHaveCount(0);

    await page.goto("/settings/admin-users");
    await expect(page).toHaveURL(/\/settings\/appearance$/);
  });

  test("keeps the settings navigation usable in a narrow viewport", async ({ page }) => {
    await page.setViewportSize({ width: 390, height: 844 });
    await installAppTransport(page, { settings: true, user: ADMIN_USER });

    await page.goto("/settings/appearance");
    await page.getByRole("tab", { name: "Runtime" }).click();
    await expect(page).toHaveURL(/\/settings\/runtime$/);
    await expect(page.getByLabel("Qdrant URL")).toBeVisible();
  });

  test("shows the bootstrap admin in the read-only user table", async ({ page }) => {
    const transport = await installAppTransport(page, { settings: true, user: ADMIN_USER });

    await page.goto("/settings/admin-users");
    await expect(page).toHaveURL(/\/settings\/admin-users$/);
    await expect(page.getByRole("heading", { name: "Users", exact: true })).toBeVisible();

    const userCell = page.getByRole("cell", { name: "planner", exact: true });
    const userRow = page.getByRole("row").filter({ has: userCell });
    await expect(userRow).toHaveCount(1);
    await expect(userRow).toContainText("Planner");
    await expect(userRow.getByText("No", { exact: true })).toBeVisible();
    await expect(userRow.getByText("Active", { exact: true })).toBeVisible();

    // Read-only coverage: no create, reset, disable or enable request.
    expect(transport.calls.filter((call) => (
      call.method !== "GET" && call.path.startsWith("/v1/admin/users")
    ))).toEqual([]);
  });

  test("saves the chunking fields and restores the original mock value", async ({ page }) => {
    const transport = await installAppTransport(page, { settings: true, user: ADMIN_USER });

    await page.goto("/settings/runtime");
    const maxChars = page.getByLabel("Max Characters");
    await expect(maxChars).toHaveValue("1200");
    await maxChars.fill("1500");
    await page.getByTestId("settings-save").click();

    await expect.poll(() => transport.calls.filter((call) => (
      call.method === "PUT" && call.path === "/v1/settings/runtime"
    )).length).toBe(1);
    const firstPut = transport.calls.find((call) => call.method === "PUT" && call.path === "/v1/settings/runtime");
    expect((firstPut?.body as { chunking: { max_chars: number } }).chunking.max_chars).toBe(1500);
    await expect(page.getByText("Settings saved").first()).toBeVisible();

    await maxChars.fill("1200");
    await page.getByTestId("settings-save").click();
    await expect.poll(() => transport.calls.filter((call) => (
      call.method === "PUT" && call.path === "/v1/settings/runtime"
    )).length).toBe(2);
    const secondPut = transport.calls.filter((call) => (
      call.method === "PUT" && call.path === "/v1/settings/runtime"
    ))[1];
    expect((secondPut?.body as { chunking: { max_chars: number } }).chunking.max_chars).toBe(1200);
    await page.reload();
    await expect(page.getByLabel("Max Characters")).toHaveValue("1200");
  });

  test("keeps a rejected docling group dirty, adopts the fulfilled peer and commits no appearance", async ({ page }) => {
    await page.setViewportSize({ width: 600, height: 900 });
    const transport = await installAppTransport(page, {
      settings: true,
      user: ADMIN_USER,
      settingsFailures: { doclingPut: { status: 500, message: "docling write refused" } },
    });

    await page.goto("/settings/appearance");
    await page.getByTestId("settings-time-zone-select").click();
    await page.getByPlaceholder("Search time zones").fill("Shanghai");
    await page.getByTestId("settings-time-zone-option-Asia/Shanghai").click();
    await expect(page.getByText("Unsaved changes")).toBeVisible();

    await openSettingsTab(page, "Docling");
    await page.locator("#docling-base-url").fill("http://docling.fixture:5001");
    await openSettingsTab(page, "Search");
    await page.locator("#search-rerank-model").fill("cohere/rerank-fixture");

    await page.getByTestId("settings-save").click();

    // Every group is submitted once: runtime admission, then the docling peer
    // that rejects and the search peer that must still be adopted.
    await expect.poll(() => transport.calls.filter((call) => (
      call.method === "PUT" && call.path === "/v1/settings/docling"
    )).length).toBe(1);
    await expect.poll(() => transport.calls.filter((call) => (
      call.method === "PUT" && call.path === "/v1/settings/search"
    )).length).toBe(1);
    const searchPut = transport.calls.find((call) => call.method === "PUT" && call.path === "/v1/settings/search");
    expect(searchPut?.body).toMatchObject({ rerank_model: "cohere/rerank-fixture" });

    await expect(page.getByText("Settings saved")).toHaveCount(0);
    await expect(page.getByText("Unsaved changes")).toBeVisible();
    expect(await page.evaluate(() => window.localStorage.getItem("context69.timeZone"))).toBeNull();

    // The rejected group keeps its draft; the adopted peer is no longer resent.
    await openSettingsTab(page, "Docling");
    await expect(page.locator("#docling-base-url")).toHaveValue("http://docling.fixture:5001");
    await page.getByTestId("settings-save").click();
    await expect.poll(() => transport.calls.filter((call) => (
      call.method === "PUT" && call.path === "/v1/settings/docling"
    )).length).toBe(2);
    expect(transport.calls.filter((call) => (
      call.method === "PUT" && call.path === "/v1/settings/search"
    ))).toHaveLength(1);

    // Reloading discards the drafts: the appearance was never committed.
    await page.reload();
    await openSettingsTab(page, "Appearance");
    await expect(page.getByTestId("settings-time-zone-select")).toContainText(/Automatic \(UTC\)/);
  });

  test("saves a time zone and applies it to stored timestamps after reload", async ({ page }) => {
    await page.setViewportSize({ width: 600, height: 900 });
    await installAppTransport(page, { settings: true, user: ADMIN_USER });

    await page.goto("/settings/appearance");
    const picker = page.getByTestId("settings-time-zone-select");
    await expect(picker).toContainText(/Automatic \(UTC\)/);

    await picker.click();
    await page.getByPlaceholder("Search time zones").fill("Shanghai");
    await page.getByTestId("settings-time-zone-option-Asia/Shanghai").click();
    await expect(picker).toContainText("Asia/Shanghai");
    await expect.poll(() => page.evaluate(() => window.localStorage.getItem("context69.timeZone"))).toBeNull();

    await page.getByTestId("settings-save").click();
    await expect(page.getByText("Settings saved").first()).toBeVisible();
    await expect.poll(() => page.evaluate(() => window.localStorage.getItem("context69.timeZone"))).toBe("Asia/Shanghai");

    await page.getByRole("tab", { name: "Access Tokens" }).click();
    await expect(page.getByText("2026-10-01 16:00", { exact: true })).toHaveCount(1);

    await page.reload();
    await expect(page.getByText("2026-10-01 16:00", { exact: true })).toHaveCount(1);
    await page.goto("/settings/appearance");
    await expect(page.getByTestId("settings-time-zone-select")).toContainText("Asia/Shanghai");
  });

  test("keeps the appearance applied in memory when the localStorage getter is denied", async ({ page }) => {
    await page.setViewportSize({ width: 600, height: 900 });
    await installAppTransport(page, { settings: true, user: ADMIN_USER });
    await page.addInitScript(() => {
      Object.defineProperty(window, "localStorage", {
        configurable: true,
        get: () => {
          throw new DOMException("Storage denied", "SecurityError");
        },
      });
    });

    await page.goto("/settings/appearance");
    await page.getByTestId("settings-time-zone-select").click();
    await page.getByPlaceholder("Search time zones").fill("Shanghai");
    await page.getByTestId("settings-time-zone-option-Asia/Shanghai").click();
    await page.getByTestId("settings-save").click();

    await expect(page.getByText(/could not store the appearance/).first()).toBeVisible();
    await expect(page.getByText("Settings saved")).toHaveCount(0);
    await expect(page.getByTestId("settings-save")).toBeEnabled();

    // The applied zone still formats this page; leaving is allowed and keeps it.
    await page.getByRole("tab", { name: "Access Tokens" }).click();
    await page.getByRole("alertdialog").getByRole("button", { name: "Leave" }).click();
    await expect(page.getByText("2026-10-01 16:00", { exact: true })).toHaveCount(1);
  });

  test("stores the appearance when only localStorage reads are denied", async ({ page }) => {
    await page.setViewportSize({ width: 600, height: 900 });
    await installAppTransport(page, { settings: true, user: ADMIN_USER });
    await page.addInitScript(() => {
      Storage.prototype.getItem = () => {
        throw new DOMException("Storage denied", "SecurityError");
      };
    });

    await page.goto("/settings/appearance");
    await page.getByTestId("settings-time-zone-select").click();
    await page.getByPlaceholder("Search time zones").fill("Shanghai");
    await page.getByTestId("settings-time-zone-option-Asia/Shanghai").click();
    await page.getByTestId("settings-save").click();

    // Writes still work, so the save may claim success even though reads fail.
    await expect(page.getByText("Settings saved").first()).toBeVisible();
    await page.getByRole("tab", { name: "Access Tokens" }).click();
    await expect(page.getByText("2026-10-01 16:00", { exact: true })).toHaveCount(1);
  });
});
