import { expect, test, type Browser, type Page } from "@playwright/test";

import { TIME_ZONE_BASE_URL, TimeZoneMockTransport, type TimeZoneMockOptions } from "./fixtures";

type StorageDenialMode = "getter" | "getItem" | "setItem";

const DENIAL_MODES: StorageDenialMode[] = ["getter", "getItem", "setItem"];
const NARROW_VIEWPORT = { width: 600, height: 900 };

let transport: TimeZoneMockTransport;

function useTransport(options: TimeZoneMockOptions = {}): TimeZoneMockTransport {
  transport = new TimeZoneMockTransport(options);
  return transport;
}

test.beforeEach(async ({ context, page }) => {
  page.on("dialog", (dialog) => dialog.accept());
  // The settings section navigation is the compact tab bar, which only renders
  // at the narrow breakpoint; the desktop layout exposes just the breadcrumb.
  await page.setViewportSize(NARROW_VIEWPORT);
  const current = useTransport();
  await current.install(context, TIME_ZONE_BASE_URL);
});

test.afterEach(() => {
  expect(transport.unmatchedApiRequests, "every API endpoint must have an explicit fixture").toEqual([]);
  const unexpectedExternal = transport.blockedExternalRequests.filter(
    (entry) => !/^GET https:\/\/(api\.iconify\.design|api\.unisvg\.com|api\.simplesvg\.com)\/lucide\.json$/.test(entry),
  );
  expect(unexpectedExternal, "only icon assets may leave the preview origin").toEqual([]);
});

function timeoutZonePicker(page: Page) {
  return page.getByTestId("settings-time-zone-select");
}

async function storedTimeZone(page: Page) {
  return page.evaluate(() => window.localStorage.getItem("context69.timeZone"));
}

async function selectTimeZone(page: Page, query: string, option: string) {
  await timeoutZonePicker(page).click();
  await page.keyboard.type(query);
  await page.getByRole("option", { name: option, exact: true }).click();
  await expect(timeoutZonePicker(page)).toHaveText(option);
}

function openSettingsTab(page: Page, name: string) {
  return page.getByRole("tab", { name, exact: true }).click();
}

function savedBadge(page: Page) {
  return page.getByTestId("settings-page-scroll").getByText("Settings saved", { exact: true });
}

function pendingBadge(page: Page) {
  return page.getByTestId("settings-page-scroll").getByText("Unsaved changes", { exact: true });
}

function notices(page: Page) {
  return page.getByRole("region", { name: "Notifications (F8)" });
}

test("appearance edits need an explicit Save and a refresh discards them", async ({ page }) => {
  await page.goto("/settings/appearance");
  const picker = timeoutZonePicker(page);
  await expect(picker).toHaveText(/Automatic \(UTC\)/);

  await openSettingsTab(page, "Access Tokens");
  await expect(page.getByRole("region", { name: "Access Tokens", exact: true })).toBeVisible();
  await expect(page.getByText("2026-03-01 00:00", { exact: true })).toBeVisible();

  await openSettingsTab(page, "Appearance");
  await selectTimeZone(page, "Shanghai", "Asia/Shanghai");

  await expect(pendingBadge(page)).toBeVisible();
  await expect(page.getByTestId("settings-save")).toBeEnabled();
  await expect.poll(() => storedTimeZone(page)).toBeNull();

  // No live preview: the applied display stays on the last saved preference.
  await openSettingsTab(page, "Access Tokens");
  await expect(page.getByText("2026-03-01 00:00", { exact: true })).toBeVisible();
  await expect(page.getByText("2026-03-01 08:00", { exact: true })).toHaveCount(0);

  // A refresh discards the unsaved draft.
  await page.goto("/settings/appearance");
  await expect(picker).toHaveText(/Automatic \(UTC\)/);
  await expect(pendingBadge(page)).toHaveCount(0);
  await expect.poll(() => storedTimeZone(page)).toBeNull();

  // Save commits the submitted preference and survives a reload.
  await selectTimeZone(page, "Shanghai", "Asia/Shanghai");
  await page.getByTestId("settings-save").click();
  await expect(savedBadge(page)).toBeVisible();
  await expect(pendingBadge(page)).toHaveCount(0);
  await expect.poll(() => storedTimeZone(page)).toBe("Asia/Shanghai");

  await openSettingsTab(page, "Access Tokens");
  await expect(page.getByText("2026-03-01 08:00", { exact: true })).toBeVisible();

  await page.reload();
  await expect(page.getByText("2026-03-01 08:00", { exact: true })).toBeVisible();

  await page.goto("/settings/appearance");
  await expect(timeoutZonePicker(page)).toHaveText("Asia/Shanghai");
});

test("a rejected docling save keeps its dirty peers and commits no appearance", async ({ page, browser }) => {
  await page.close();
  const context = await browser.newContext({
    viewport: NARROW_VIEWPORT,
    timezoneId: "UTC",
    serviceWorkers: "block",
  });
  const failing = useTransport({ doclingPutStatus: 500 });
  await failing.install(context, TIME_ZONE_BASE_URL);
  const nextPage = await context.newPage();
  const dialogs: string[] = [];
  nextPage.on("dialog", async (dialog) => {
    dialogs.push(dialog.message());
    await dialog.accept();
  });

  try {
    await nextPage.goto("/settings/appearance");
    await selectTimeZone(nextPage, "Shanghai", "Asia/Shanghai");
    await openSettingsTab(nextPage, "Docling");
    await nextPage.locator("#docling-base-url").fill("http://docling.fixture:5001");
    await openSettingsTab(nextPage, "Search");
    await nextPage.locator("#search-rerank-model").fill("cohere/rerank-fixture");
    await openSettingsTab(nextPage, "Runtime");
    await nextPage.locator("#runtime-embedding-model").fill("text-embedding-3-small");

    await nextPage.getByTestId("settings-save").click();
    await expect.poll(() => failing.putRequests.map((entry) => entry.path)).toEqual([
      "/v1/settings/runtime",
      "/v1/settings/docling",
      "/v1/settings/search",
    ]);
    const searchPut = failing.putRequests.find((entry) => entry.path === "/v1/settings/search");
    expect(searchPut?.body).toMatchObject({ rerank_model: "cohere/rerank-fixture" });

    await expect(savedBadge(nextPage)).toHaveCount(0);
    await expect(pendingBadge(nextPage)).toBeVisible();
    await expect.poll(() => nextPage.evaluate(() => window.localStorage.getItem("context69.timeZone"))).toBeNull();
    expect(failing.putRequests[0].body).toMatchObject({
      embedding: { model: "text-embedding-3-small" },
    });

    // The rejected docling group keeps its draft; the appearance commit never ran.
    await openSettingsTab(nextPage, "Docling");
    await expect(nextPage.locator("#docling-base-url")).toHaveValue("http://docling.fixture:5001");
    expect(dialogs.length).toBeGreaterThan(0);

    // Only the still-dirty docling group is resent; the adopted search peer is not.
    await nextPage.getByTestId("settings-save").click();
    await expect.poll(() => failing.putRequests.filter((entry) => entry.path === "/v1/settings/docling").length).toBe(2);
    expect(failing.putRequests.filter((entry) => entry.path === "/v1/settings/search")).toHaveLength(1);
    expect(failing.putRequests.filter((entry) => entry.path === "/v1/settings/runtime")).toHaveLength(1);

    await nextPage.goto("/settings/appearance");
    await expect(timeoutZonePicker(nextPage)).toHaveText(/Automatic \(UTC\)/);
  } finally {
    await context.close();
  }
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

for (const mode of DENIAL_MODES) {
  test(`the app starts, saves a zone, and keeps it in memory when localStorage ${mode} is denied`, async ({ browser }) => {
    const context = await browser.newContext({
      viewport: NARROW_VIEWPORT,
      timezoneId: "UTC",
      serviceWorkers: "block",
    });
    const localTransport = useTransport();
    await localTransport.install(context, TIME_ZONE_BASE_URL);
    await context.addInitScript(deniedStorageScript, mode);
    const page = await context.newPage();
    page.on("dialog", (dialog) => dialog.accept());
    const pageErrors: string[] = [];
    page.on("pageerror", (error) => pageErrors.push(error.message));

    try {
      await page.goto("/settings/appearance");
      const picker = timeoutZonePicker(page);
      await expect(picker).toBeVisible();
      await expect(picker).toHaveText(/Automatic \(UTC\)/);
      await expect(page.getByRole("region", { name: "Appearance", exact: true })).toBeVisible();
      const denialActive = await page.evaluate(() => {
        try {
          window.localStorage.setItem("context69.probe", "1");
          return window.localStorage.getItem("context69.probe") !== "1";
        } catch {
          return true;
        }
      });
      expect(denialActive, "the injected storage denial must be active").toBe(true);

      await selectTimeZone(page, "Shanghai", "Asia/Shanghai");
      await page.getByTestId("settings-save").click();

      await openSettingsTab(page, "Access Tokens");
      await expect(page.getByText("2026-03-01 08:00", { exact: true })).toBeVisible();

      if (mode === "getItem") {
        // Reads are denied but writes still work, so the save really stored the
        // preference and may claim success.
        await expect(savedBadge(page)).toBeVisible();
      } else {
        await expect(notices(page).getByText(/could not store the appearance completely/)).toBeVisible();
        await expect(savedBadge(page)).toHaveCount(0);
        await expect(pendingBadge(page)).toBeVisible();

        // Returning the draft to the saved baseline must not clear the pending
        // state: the applied Shanghai zone is still unstored and Save stays
        // actionable.
        await openSettingsTab(page, "Appearance");
        await selectTimeZone(page, "UTC", "UTC");
        await expect(pendingBadge(page)).toBeVisible();
        await expect(page.getByTestId("settings-save")).toBeEnabled();

        // The explicit retry applies the current draft in memory and still
        // warns while writes stay denied.
        await page.getByTestId("settings-save").click();
        await openSettingsTab(page, "Access Tokens");
        await expect(page.getByText("2026-03-01 00:00", { exact: true })).toBeVisible();
        await expect(savedBadge(page)).toHaveCount(0);
        await expect(pendingBadge(page)).toBeVisible();
      }

      expect(pageErrors, "storage denial must not raise an uncaught error").toEqual([]);
    } finally {
      await context.close();
    }
  });
}
