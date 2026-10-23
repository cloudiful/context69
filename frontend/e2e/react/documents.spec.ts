import { expect, test } from "@playwright/test";

import {
  ADMIN_USER,
  HOSTILE_DOCUMENT,
  LIBRARY_DOCUMENT,
  READER_DOCUMENT,
  STALE_DOCUMENT,
  searchHitFixture,
  searchPageFixture,
} from "./fixtures";
import { installAppTransport, recordExternalRequests } from "./transport";

test.describe("react document reader", () => {
  test("opens a search hit, highlights the terms and returns to the results", async ({ page }) => {
    const external = recordExternalRequests(page);
    const transport = await installAppTransport(page, {
      user: ADMIN_USER,
      search: {
        stream: "http-error",
        post: [
          searchPageFixture("cyber policy", [
            searchHitFixture({
              chunk_id: "c1",
              title: "Cyber Policy Update",
              document_id: 42,
              score: 0.9,
            }),
          ]),
          searchPageFixture("cyber policy", [
            searchHitFixture({
              chunk_id: "c1",
              title: "Cyber Policy Update",
              document_id: 42,
              score: 0.9,
            }),
          ]),
        ],
      },
      documents: {
        documents: {
          "42": {
            ...READER_DOCUMENT,
            chunks: [{
              chunk_id: "reader-chunk-1",
              chunk_index: 0,
              record_hash: "reader-hash",
              text: "Stored **cyber policy** body.",
            }],
          },
        },
      },
    });

    await page.goto("/search?q=cyber+policy");
    await page.getByTestId("search-result-open").first().click();

    await expect(page).toHaveURL(/\/documents\/42\?from=search$/);
    await expect(page.getByRole("heading", { name: "Cyber Policy Update", level: 1 })).toBeVisible();
    await expect(page.getByTestId("document-summary")).toContainText(
      "Stored summary about cyber policy and its scope.",
    );
    await expect(page.getByTestId("document-summary").locator("mark").first()).toBeVisible();
    await expect(page.getByTestId("document-block-content").locator("mark").first()).toBeVisible();
    await expect(page.getByTestId("document-provenance")).toContainText("reader-hash");
    await expect(page.getByTestId("document-provenance")).toContainText("stock/alpha");

    await page.getByTestId("document-back-to-search").click();

    await expect(page).toHaveURL(/\/search\?/);
    await expect(page.getByTestId("search-result-item")).toHaveCount(1);
    expect(transport.calls.some((call) => call.path === "/v1/documents/42")).toBe(true);
    expect(external).toEqual([]);
    expect(transport.unmocked).toEqual([]);
  });

  test("opens the library target with the encoded group path and return marker", async ({ page }) => {
    const transport = await installAppTransport(page, {
      user: ADMIN_USER,
      workspace: {},
      documents: { documents: { "43": LIBRARY_DOCUMENT } },
    });

    await page.goto("/documents/43?from=search");
    await expect(page.getByTestId("document-open-source")).toHaveText("Open In Library");
    await expect(page.getByTestId("app-breadcrumbs")).toContainText("Document Details");

    await page.getByTestId("document-open-source").click();

    await expect(page).toHaveURL(/\/groups\/stock%2Falpha\?file=file-ready&from=search$/);
    await expect(page.getByTestId("group-shell")).toBeVisible();
    await expect(page.getByRole("dialog")).toContainText("quarterly.md");
    await expect(page.getByTestId("app-breadcrumbs")).toContainText("stock/alpha");
    await expect(page.getByTestId("group-back-to-search")).toBeVisible();

    expect(transport.unmocked).toEqual([]);
  });

  test("pages stored blocks ten at a time", async ({ page }) => {
    await installAppTransport(page, {
      user: ADMIN_USER,
      documents: { documents: { "42": READER_DOCUMENT } },
    });

    await page.goto("/documents/42");

    await expect(page.getByTestId("document-block")).toHaveCount(10);
    await expect(page.getByTestId("document-page-status")).toHaveText("Page 1 of 2");
    await expect(page.getByText("Stored reader body 11 about cyber policy.")).toHaveCount(0);

    await page.getByRole("button", { name: "Next" }).click();

    await expect(page.getByTestId("document-block")).toHaveCount(2);
    await expect(page.getByTestId("document-page-status")).toHaveText("Page 2 of 2");
    await expect(page.getByText("Stored reader body 11 about cyber policy.")).toBeVisible();
  });

  test("rejects stale and empty blocks instead of showing them", async ({ page }) => {
    await installAppTransport(page, {
      user: ADMIN_USER,
      documents: { documents: { "44": STALE_DOCUMENT } },
    });

    await page.goto("/documents/44");

    await expect(page.getByText("current stored block")).toBeVisible();
    await expect(page.getByText("stale secret block")).toHaveCount(0);
    await expect(page.getByTestId("document-invalid-block")).toHaveCount(2);
    await expect(page.getByTestId("document-invalid-block").first()).toContainText(
      "changed while the document was loading",
    );
  });

  test("never executes or links hostile markdown", async ({ page }) => {
    const errors: string[] = [];
    page.on("pageerror", (error) => errors.push(error.message));
    await installAppTransport(page, {
      user: ADMIN_USER,
      documents: { documents: { "45": HOSTILE_DOCUMENT } },
    });

    await page.goto("/documents/45");
    await expect(page.getByTestId("document-block-content")).toBeVisible();

    expect(await page.evaluate(() => (window as unknown as { __p3cE2eXss?: number }).__p3cE2eXss))
      .toBeUndefined();
    const content = page.getByTestId("document-block-content");
    await expect(content.locator("script")).toHaveCount(0);
    await expect(content.locator("img")).toHaveCount(0);
    await expect(content.locator('a[href^="javascript:"]')).toHaveCount(0);
    await expect(content.locator('a[href^="data:"]')).toHaveCount(0);
    await expect(content.getByText("click me")).toBeVisible();
    const safe = content.getByRole("link", { name: "safe" });
    await expect(safe).toHaveAttribute("href", "https://example.test/ok");
    await expect(safe).toHaveAttribute("rel", "noreferrer noopener");
    expect(errors).toEqual([]);
  });

  test("shows not-found, forbidden and retryable failure states", async ({ page }) => {
    const transport = await installAppTransport(page, {
      user: ADMIN_USER,
      documents: {
        failures: {
          "404": { status: 404, message: "Document missing" },
          "403": { status: 403, message: "Denied" },
          "500": { status: 500, message: "Reader unavailable" },
        },
      },
    });

    await page.goto("/documents/404");
    await expect(page.getByTestId("document-error")).toContainText("Document not found");
    await expect(page.getByTestId("document-error").getByRole("button", { name: "Retry" })).toHaveCount(0);

    await page.goto("/documents/403");
    await expect(page.getByTestId("document-error")).toContainText("Document unavailable");

    await page.goto("/documents/500");
    const failure = page.getByTestId("document-error");
    await expect(failure).toContainText("Document load failed");
    await failure.getByRole("button", { name: "Retry" }).click();
    await expect.poll(() => transport.calls.filter((call) => call.path === "/v1/documents/500").length)
      .toBeGreaterThanOrEqual(2);
  });

  test("renders the reader in Chinese, dark mode and a narrow viewport", async ({ page }) => {
    await page.setViewportSize({ width: 390, height: 844 });
    await page.addInitScript(() => {
      window.localStorage.setItem("context69.locale", "zh-CN");
      window.localStorage.setItem("context69.theme", "dark");
      window.localStorage.setItem("context69.timeZone", "Asia/Tokyo");
    });
    await installAppTransport(page, {
      user: ADMIN_USER,
      documents: { documents: { "42": READER_DOCUMENT } },
    });

    await page.goto("/documents/42");

    await expect(page.getByRole("heading", { name: "Cyber Policy Update", level: 1 })).toBeVisible();
    await expect(page.getByTestId("document-summary")).toContainText("摘要");
    await expect(page.getByTestId("document-provenance")).toContainText("文档来源信息");
    await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
    await expect(page.getByTestId("document-block")).toHaveCount(10);
    await expect(page.getByTestId("document-page-status")).toContainText("第 1 / 2 页");
    await expect(page.getByTestId("document-visibility")).toContainText("私有");
    await expect(page.getByTestId("document-provenance")).toContainText("私有");
    await expect(page.getByTestId("document-visibility")).not.toContainText("private");
  });
});
