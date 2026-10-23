import { expect, test } from "@playwright/test";

import {
  ADMIN_USER,
  PREVIEW_DOCUMENT,
  searchHitFixture,
  searchPageFixture,
} from "./fixtures";
import { installAppTransport, recordExternalRequests } from "./transport";

const LOCAL_PAGE = searchPageFixture("policy", [
  searchHitFixture({ chunk_id: "c1", title: "Policy Local", score: 0.4 }),
  searchHitFixture({ chunk_id: "c2", title: "Policy Winner", score: 0.9 }),
]);

const RERANKED_PAGE = searchPageFixture("policy", [
  searchHitFixture({ chunk_id: "c2", title: "Policy Winner", score: 0.9 }),
  searchHitFixture({ chunk_id: "c1", title: "Policy Local", score: 0.4 }),
]);

test.describe("react search", () => {
  for (const locale of ["en", "zh-CN"]) {
    test(`persistent failure retries committed filters (${locale})`, async ({ page }) => {
      await page.setViewportSize({ width: locale === "en" ? 1280 : 600, height: 900 });
      await page.addInitScript((value) => localStorage.setItem("context69.locale", value), locale);
      const transport = await installAppTransport(page, { search: { stream: "http-error", postError: { status: 503, message: "search unavailable" }, postErrorReads: 1, post: LOCAL_PAGE } });
      await page.goto("/search?q=policy&limit=16");
      const alert = page.getByRole("alert");
      await expect(alert).toContainText("search unavailable");
      await page.getByTestId("search-query").fill("unsent");
      const route = page.url();
      await alert.getByRole("button", { name: locale === "en" ? "Retry" : "重试" }).click();
      await expect(page.getByTestId("search-result-item")).toHaveCount(2);
      await expect(alert).toHaveCount(0);
      expect(page.url()).toBe(route);
      await expect(page.getByTestId("search-query")).toHaveValue("unsent");
      expect(transport.calls.filter((call) => call.path === "/v1/search").map((call) => call.body)).toEqual([expect.objectContaining({ query: "policy", limit: 16 }), expect.objectContaining({ query: "policy", limit: 16 })]);
      expect(transport.unmocked).toEqual([]);
    });
  }

  test("streams a local page, reorders on reranked and ends with done", async ({ page }) => {
    const external = recordExternalRequests(page);
    const transport = await installAppTransport(page, {
      user: ADMIN_USER,
      search: { stream: "success", local: LOCAL_PAGE, reranked: RERANKED_PAGE },
    });

    await page.goto("/search?q=policy&limit=16");

    await expect(page.getByTestId("search-result-item")).toHaveCount(2);
    await expect(page.getByTestId("search-result-select").first()).toContainText("Policy Winner");
    await expect(page.getByTestId("search-refining")).toHaveCount(0);
    await expect(page.getByRole("heading", { name: "Search Results" })).toBeVisible();

    const stream = transport.calls.find((call) => call.path === "/v1/search/stream");
    expect(stream?.method).toBe("GET");
    expect(stream?.query).toContain("query=policy");
    expect(stream?.query).toContain("limit=16");
    expect(transport.calls.some((call) => call.method === "POST" && call.path === "/v1/search")).toBe(false);
    expect(external).toEqual([]);
    expect(transport.unmocked).toEqual([]);
  });

  test("falls back to POST with a subtle notice when the stream fails", async ({ page }) => {
    const transport = await installAppTransport(page, {
      user: ADMIN_USER,
      search: {
        stream: "http-error",
        post: searchPageFixture("fallback", [
          searchHitFixture({ chunk_id: "c-fb", title: "Fallback Result", score: 0.8 }),
        ]),
      },
    });

    await page.goto("/search?q=fallback");

    await expect(page.getByTestId("search-result-item")).toHaveCount(1);
    await expect(page.getByTestId("search-result-select")).toContainText("Fallback Result");
    await expect(page.getByTestId("search-stream-fallback-notice")).toBeVisible();

    const post = transport.calls.find((call) => call.method === "POST" && call.path === "/v1/search");
    expect(post?.body).toMatchObject({ query: "fallback", page: 1, limit: 8 });
  });

  test("recovers from a malformed frame through POST", async ({ page }) => {
    const transport = await installAppTransport(page, {
      user: ADMIN_USER,
      search: {
        stream: "malformed",
        post: searchPageFixture("policy", [
          searchHitFixture({ chunk_id: "safe", title: "Safe Recovery", score: 0.7 }),
        ]),
      },
    });

    await page.goto("/search?q=policy");

    await expect(page.getByTestId("search-result-select")).toContainText("Safe Recovery");
    await expect(page.getByTestId("search-stream-fallback-notice")).toBeVisible();
    expect(transport.calls.filter((call) => call.method === "POST" && call.path === "/v1/search")).toHaveLength(1);
  });

  test("pages cursors through POST and keeps the URL cursor", async ({ page }) => {
    const first = searchPageFixture("beta", [
      searchHitFixture({ chunk_id: "c1", title: "Beta One", score: 0.9 }),
    ], { total: 2, total_pages: 2, has_more: true, next_cursor: "c1:next8" });
    const second = searchPageFixture("beta", [
      searchHitFixture({ chunk_id: "c2", title: "Beta Two", score: 0.8 }),
    ], { page: 2, total: 2, total_pages: 2, has_more: false, prev_cursor: "c1:prev8" });

    const transport = await installAppTransport(page, {
      user: ADMIN_USER,
      search: { stream: "success", local: first, post: second },
    });

    await page.goto("/search?q=beta");
    await expect(page.getByTestId("search-result-select")).toContainText("Beta One");

    await page.getByTestId("search-next").click();

    await expect(page.getByTestId("search-result-select")).toContainText("Beta Two");
    await expect.poll(() => new URL(page.url()).searchParams.get("cursor")).toBe("c1:next8");
    expect(new URL(page.url()).searchParams.get("page")).toBeNull();

    const post = transport.calls.find((call) => call.method === "POST" && call.path === "/v1/search");
    expect(post?.body).toMatchObject({ query: "beta", cursor: "c1:next8", page: 1 });
  });

  test("keeps date ordering forward-only", async ({ page }) => {
    await installAppTransport(page, {
      user: ADMIN_USER,
      search: {
        local: searchPageFixture("date", [
          searchHitFixture({ chunk_id: "d1", title: "Date Result", score: 0.5 }),
        ], { page: 2, total: 9, total_pages: 2, prev_cursor: "c1:prev8" }),
      },
    });

    await page.goto("/search?q=date&sort=date");

    await expect(page.getByTestId("search-result-select")).toContainText("Date Result");
    await expect(page.getByTestId("search-prev")).toBeDisabled();
    await expect(page.getByTestId("search-next")).toBeDisabled();
    await expect(page.getByTestId("search-sort-mode")).toBeVisible();
  });

  test("keeps filters in the stream request and replays history cursor-free", async ({ page }) => {
    const transport = await installAppTransport(page, {
      user: ADMIN_USER,
      search: {
        local: searchPageFixture("policy", [
          searchHitFixture({ chunk_id: "c1", title: "Policy Result", score: 0.9 }),
        ]),
      },
    });

    await page.goto("/search");
    await page.getByTestId("search-query").fill("policy");
    await page.getByTestId("search-toggle-advanced").click();
    await page.getByTestId("search-source").click();
    await page.getByRole("option", { name: "Government Documents (gov_documents)" }).click();
    await page.getByTestId("search-limit").fill("16");
    await page.getByTestId("search-submit").click();

    await expect(page.getByTestId("search-result-select")).toContainText("Policy Result");
    const firstStream = transport.calls.filter((call) => call.path === "/v1/search/stream").at(-1);
    expect(firstStream?.query).toContain("query=policy");
    expect(firstStream?.query).toContain("limit=16");
    expect(firstStream?.query).toContain("source_key=gov_documents");

    await page.getByRole("button", { name: "Recent searches" }).click();
    await expect(page.getByTestId("search-history-entry")).toHaveCount(1);

    await page.getByTestId("search-query").fill("budget");
    await page.getByTestId("search-submit").click();
    await expect(page.getByRole("button", { name: "Recent searches" })).toBeVisible();
    await page.getByRole("button", { name: "Recent searches" }).click();
    await page.getByTestId("search-history-entry").filter({ hasText: "policy" }).click();

    await expect.poll(() => {
      const streams = transport.calls.filter((call) => call.path === "/v1/search/stream");
      return streams.at(-1)?.query ?? "";
    }).toContain("query=policy");
    await expect.poll(() => {
      const streams = transport.calls.filter((call) => call.path === "/v1/search/stream");
      return streams.at(-1)?.query ?? "";
    }).toContain("source_key=gov_documents");

    const stored = await page.evaluate(() => JSON.parse(
      window.localStorage.getItem("context69.search-history") ?? "[]",
    ) as Array<Record<string, unknown>>);
    const policyEntry = stored.find((entry) => entry.query === "policy");
    expect(policyEntry).toBeDefined();
    expect(policyEntry).not.toHaveProperty("cursor");
    expect(policyEntry).not.toHaveProperty("page");
  });

  test("previews the stored document and opens the hit with return context", async ({ page }) => {
    const transport = await installAppTransport(page, {
      user: ADMIN_USER,
      search: {
        stream: "success",
        local: searchPageFixture("policy", [
          searchHitFixture({ chunk_id: "c1", title: "Policy Brief", score: 0.8 }),
        ]),
        document: PREVIEW_DOCUMENT,
      },
    });

    await page.goto("/search?q=policy");
    await page.getByTestId("search-result-preview").click();

    const dialog = page.getByRole("dialog");
    await expect(dialog).toBeVisible();
    await expect(dialog.getByTestId("search-selection-preview")).toContainText(
      "Stored document body about policy.",
    );
    expect(transport.calls.some((call) => call.path === "/v1/documents/1")).toBe(true);

    await dialog.getByTestId("search-preview-open").click();
    await expect(page).toHaveURL(/\/documents\/1\?from=search$/);

    const session = await page.evaluate(() => window.sessionStorage.getItem("context69.search-session") ?? "");
    expect(session).toContain("policy");
    expect(session).not.toContain("chunk_id");
  });

  test("keeps search usable with in-memory history when storage is denied", async ({ page }) => {
    await page.addInitScript(() => {
      const getItem = Storage.prototype.getItem;
      const setItem = Storage.prototype.setItem;
      const deny = (key: string) => String(key).startsWith("context69.search");
      Storage.prototype.getItem = function (key: string) {
        if (deny(key)) {
          throw new DOMException("Storage denied", "SecurityError");
        }
        return getItem.call(this, key);
      };
      Storage.prototype.setItem = function (key: string, value: string) {
        if (deny(key)) {
          throw new DOMException("Storage denied", "SecurityError");
        }
        return setItem.call(this, key, value);
      };
    });

    await installAppTransport(page, {
      user: ADMIN_USER,
      search: {
        local: searchPageFixture("policy", [
          searchHitFixture({ chunk_id: "c1", title: "Offline Policy", score: 0.9 }),
        ]),
      },
    });

    await page.goto("/search");
    await page.getByTestId("search-query").fill("policy");
    await page.getByTestId("search-submit").click();

    await expect(page.getByTestId("search-result-select")).toContainText("Offline Policy");
    await page.getByRole("button", { name: "Recent searches" }).click();
    await expect(page.getByTestId("search-history-entry")).toContainText("policy");
  });

  test("surfaces a POST failure after the stream fails", async ({ page }) => {
    await installAppTransport(page, {
      user: ADMIN_USER,
      search: {
        stream: "http-error",
        postError: { status: 500, message: "search rejected" },
      },
    });

    await page.goto("/search?q=policy");

    await expect(page.getByRole("alert")).toContainText("search rejected");
    await expect(page.getByRole("alert").getByRole("button", { name: "Retry" })).toBeEnabled();
  });

  test("renders consistently in Chinese, dark mode and a narrow viewport", async ({ page }) => {
    await page.setViewportSize({ width: 390, height: 844 });
    await page.addInitScript(() => {
      window.localStorage.setItem("context69.locale", "zh-CN");
      window.localStorage.setItem("context69.theme", "dark");
      window.localStorage.setItem("context69.timeZone", "Asia/Tokyo");
    });
    await installAppTransport(page, {
      user: ADMIN_USER,
      search: {
        local: searchPageFixture("policy", [
          searchHitFixture({
            chunk_id: "c1",
            title: "Policy Brief",
            score: 0.8123,
            group_path: "stock/sgs-disclosures",
          }),
        ]),
      },
    });

    await page.goto("/search?q=policy");

    await expect(page.getByRole("heading", { name: "搜索", exact: true })).toBeVisible();
    await expect(page.getByTestId("search-result-select")).toContainText("Policy Brief");
    await expect(page.getByTestId("search-result-score")).toHaveText("0.812");
    await expect(page.getByText("stock/sgs-disclosures")).toBeVisible();
    await expect(page.locator("mark").first()).toBeVisible();
    await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
    await expect(page.getByTestId("search-result-item")).toHaveCount(1);
  });
});
