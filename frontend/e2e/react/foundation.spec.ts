import { expect, test } from "@playwright/test";

import { ADMIN_USER, MEMBER_USER, READER_DOCUMENT } from "./fixtures";
import { installAppTransport, installAuthTransport, recordExternalRequests } from "./transport";

test.describe("react foundation", () => {
  test("signs in, keeps the shell consistent and signs out", async ({ page }) => {
    const external = recordExternalRequests(page);
    await installAuthTransport(page, { guest: true, user: MEMBER_USER });

    await page.goto("/login");
    await expect(page.getByRole("heading", { name: "Sign In" })).toBeVisible();

    await page.getByLabel("Login Name").fill("planner");
    await page.getByLabel("Password").fill("secret");
    await page.getByRole("button", { name: "Sign In" }).click();

    await expect(page).toHaveURL(/\/search$/);
    await expect(page.getByRole("link", { name: "Search", exact: true })).toHaveAttribute("aria-current", "page");

    await page.getByRole("button", { name: "Planner" }).click();
    await page.getByRole("menuitem", { name: "Log Out" }).click();

    await expect(page).toHaveURL(/\/login$/);
    expect(external).toEqual([]);
  });

  test("reports invalid credentials in the sign-in form", async ({ page }) => {
    await installAuthTransport(page, { guest: true, loginRejects: true });

    await page.goto("/login");
    await page.getByLabel("Login Name").fill("admin");
    await page.getByLabel("Password").fill("wrong");
    await page.getByRole("button", { name: "Sign In" }).click();

    await expect(page.getByText("Invalid login name or password.")).toBeVisible();
    await expect(page).toHaveURL(/\/login$/);
  });

  test("redirects an expired session to login and back again", async ({ page }) => {
    await installAuthTransport(page, { guest: true, user: ADMIN_USER });

    await page.goto("/processing-queue");
    await expect(page).toHaveURL(/\/login\?redirect=%2Fprocessing-queue/);
    await expect(page.getByRole("heading", { name: "Sign In" })).toBeVisible();

    await page.getByLabel("Login Name").fill("admin");
    await page.getByLabel("Password").fill("secret");
    await page.getByRole("button", { name: "Sign In" }).click();

    await expect(page).toHaveURL(/\/processing-queue$/);
    await expect(page.getByRole("heading", { name: "Processing Queue" })).toBeVisible();
  });

  test("never serves one account's cached document to the next account", async ({ page }) => {
    const external = recordExternalRequests(page);
    await installAppTransport(page, {
      user: ADMIN_USER,
      accounts: [ADMIN_USER, MEMBER_USER],
      documents: {
        documents: { "42": READER_DOCUMENT },
        owners: { "42": ADMIN_USER.login_name },
      },
    });

    await page.goto("/documents/42");
    await expect(page.getByRole("heading", { name: READER_DOCUMENT.title })).toBeVisible();

    await page.getByRole("button", { name: "Administrator" }).click();
    await page.getByRole("menuitem", { name: "Log Out" }).click();
    await expect(page).toHaveURL(/\/login$/);

    await page.getByLabel("Login Name").fill(MEMBER_USER.login_name);
    await page.getByLabel("Password").fill("secret");
    await page.getByRole("button", { name: "Sign In" }).click();
    await expect(page).toHaveURL(/\/search$/);

    await page.goto("/documents/42");
    await expect(page.getByTestId("document-error")).toContainText("You do not have access to this document.");
    await expect(page.getByText(READER_DOCUMENT.title)).toHaveCount(0);
    expect(external).toEqual([]);
  });

  test("keeps the shell usable at a narrow viewport", async ({ page }) => {
    await page.setViewportSize({ width: 390, height: 844 });
    await installAuthTransport(page, { user: ADMIN_USER });

    await page.goto("/search");
    await expect(page.getByRole("heading", { name: "Search" })).toBeVisible();

    await page.getByRole("button", { name: "Show" }).click();

    const dialog = page.getByRole("dialog");
    await expect(dialog.getByRole("link", { name: "Settings" })).toBeVisible();
    await dialog.getByRole("link", { name: "Settings" }).click();

    await expect(page).toHaveURL(/\/settings\/appearance$/);
    await expect(page.getByRole("heading", { name: "Settings" })).toBeVisible();
  });

  test("renders stored locale, theme and time zone choices", async ({ page }) => {
    await installAuthTransport(page, { user: ADMIN_USER });

    await page.addInitScript(() => {
      window.localStorage.setItem("context69.locale", "zh-CN");
      window.localStorage.setItem("context69.theme", "light");
      window.localStorage.setItem("context69.timeZone", "Asia/Tokyo");
    });

    await page.goto("/search");

    await expect(page.getByRole("link", { name: "浏览器" })).toBeVisible();
    await expect(page.locator("html")).toHaveAttribute("lang", "zh-CN");
    await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
    await expect(page.locator("html")).not.toHaveClass(/dark/);
  });

  test("shows a clear not-found page for an unknown route", async ({ page }) => {
    await installAuthTransport(page, { user: ADMIN_USER });

    await page.goto("/missing/page");

    await expect(page.getByRole("heading", { name: "Page not found" })).toBeVisible();
    await page.getByRole("link", { name: "Return to Search" }).click();
    await expect(page).toHaveURL(/\/search$/);
  });
});
