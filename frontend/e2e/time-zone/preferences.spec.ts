import { expect, test, type Browser, type Page } from "@playwright/test";

import { TIME_ZONE_BASE_URL, TimeZoneMockTransport } from "./fixtures";

type StorageDenialMode = "getter" | "getItem" | "setItem";

const DENIAL_MODES: StorageDenialMode[] = ["getter", "getItem", "setItem"];

let transport: TimeZoneMockTransport;

test.beforeEach(async ({ context }) => {
  transport = new TimeZoneMockTransport();
  await transport.install(context, TIME_ZONE_BASE_URL);
});

test.afterEach(() => {
  expect(transport.unmatchedApiRequests, "every API endpoint must have an explicit fixture").toEqual([]);
  const unexpectedExternal = transport.blockedExternalRequests.filter(
    (entry) => !/^GET https:\/\/(api\.iconify\.design|api\.unisvg\.com|api\.simplesvg\.com)\/lucide\.json$/.test(entry),
  );
  expect(unexpectedExternal, "only icon assets may leave the preview origin").toEqual([]);
});

function deniedStorageScript(mode: StorageDenialMode) {
  const denied = () => {
    throw new DOMException("Storage denied", "SecurityError");
  };

  if (mode === "getter") {
    Object.defineProperty(window, "localStorage", {
      configurable: true,
      get: denied,
    });
    return;
  }

  if (mode === "getItem") {
    Storage.prototype.getItem = denied;
    return;
  }

  Storage.prototype.setItem = denied;
}

async function openWithDeniedStorage(browser: Browser, mode: StorageDenialMode): Promise<{
  page: Page;
  pageErrors: string[];
  close: () => Promise<void>;
}> {
  const context = await browser.newContext({
    viewport: { width: 600, height: 900 },
    timezoneId: "UTC",
    serviceWorkers: "block",
  });
  const page = await context.newPage();
  const pageErrors: string[] = [];
  page.on("pageerror", (error) => pageErrors.push(error.message));
  const deniedTransport = new TimeZoneMockTransport();
  await deniedTransport.install(context, TIME_ZONE_BASE_URL);
  await context.addInitScript(deniedStorageScript, mode);
  transport = deniedTransport;

  return { page, pageErrors, close: () => context.close() };
}

test("manual time zone selection corrects displayed instants and survives reload", async ({ page }) => {
  await page.goto("/settings/appearance");

  const picker = page.getByTestId("settings-time-zone-select");
  await expect(picker).toBeVisible();
  await expect(picker).toHaveText(/Automatic \(UTC\)/);

  await picker.click();
  await page.keyboard.type("Shanghai");
  const shanghai = page.getByRole("option", { name: "Asia/Shanghai", exact: true });
  await expect(shanghai).toBeVisible();
  await shanghai.click();
  await expect(picker).toHaveText("Asia/Shanghai");

  await page.goto("/settings/access-tokens");
  await expect(page.getByRole("region", { name: "Access Tokens", exact: true })).toBeVisible();
  await expect(page.getByText("2026-03-01 08:00", { exact: true })).toBeVisible();

  await page.reload();
  await expect(page.getByText("2026-03-01 08:00", { exact: true })).toBeVisible();

  await page.goto("/settings/appearance");
  await expect(picker).toHaveText("Asia/Shanghai");

  await picker.click();
  await page.keyboard.type("UTC");
  await page.getByRole("option", { name: "UTC", exact: true }).click();
  await expect(picker).toHaveText("UTC");
  await expect(picker).not.toContainText("Automatic");

  await page.goto("/settings/access-tokens");
  await expect(page.getByText("2026-03-01 00:00", { exact: true })).toBeVisible();
  await expect(page.getByText("2026-03-01 08:00", { exact: true })).toHaveCount(0);
});

for (const mode of DENIAL_MODES) {
  test(`the app starts and applies a zone in memory when localStorage ${mode} is denied`, async ({ browser }) => {
    const app = await openWithDeniedStorage(browser, mode);

    try {
      await app.page.goto("/settings/appearance");
      const picker = app.page.getByTestId("settings-time-zone-select");
      await expect(picker).toBeVisible();
      await expect(picker).toHaveText(/Automatic \(UTC\)/);
      await expect(app.page.getByRole("region", { name: "Appearance", exact: true })).toBeVisible();
      const denialActive = await app.page.evaluate(() => {
        try {
          if (typeof window.localStorage !== "object" && typeof window.localStorage !== "function") {
            return true;
          }
          window.localStorage.setItem("context69.probe", "1");
          return window.localStorage.getItem("context69.probe") !== "1";
        } catch {
          return true;
        }
      });
      expect(denialActive, "the injected storage denial must be active").toBe(true);

      await picker.click();
      await app.page.keyboard.type("Shanghai");
      await app.page.getByRole("option", { name: "Asia/Shanghai", exact: true }).click();
      await expect(picker).toHaveText("Asia/Shanghai");

      await app.page.getByRole("tab", { name: "Access Tokens", exact: true }).click();
      await expect(app.page.getByRole("region", { name: "Access Tokens", exact: true })).toBeVisible();
      await expect(app.page.getByText("2026-03-01 08:00", { exact: true })).toBeVisible();

      expect(app.pageErrors, "storage denial must not raise an uncaught error").toEqual([]);

      await app.page.reload();
      await expect(app.page.getByRole("region", { name: "Access Tokens", exact: true })).toBeVisible();
      await app.page.goto("/settings/appearance");
      await expect(app.page.getByTestId("settings-time-zone-select")).toBeVisible();
      expect(app.pageErrors).toEqual([]);
    } finally {
      await app.close();
    }
  });
}
