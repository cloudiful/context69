import { expect, test } from "@playwright/test";

/**
 * Read-only coverage of the admin-only users surface. The bootstrap admin must
 * reach the route and see the user table, without creating, resetting,
 * disabling, or enabling any account.
 */
test("renders the bootstrap admin in the read-only user table", async ({ page }) => {
  await page.goto("/settings/admin-users");
  await expect(page).toHaveURL(/\/settings\/admin-users$/);
  await expect(page.getByRole("heading", { name: "Users", exact: true })).toBeVisible();

  const table = page.getByRole("table");
  await expect(table).toBeVisible();

  const adminCell = page.getByRole("cell", { name: "admin", exact: true });
  const adminRow = page.getByRole("row").filter({ has: adminCell });
  await expect(adminRow).toHaveCount(1);
  await expect(adminRow).toContainText("Administrator");
  await expect(adminRow.getByText("Yes", { exact: true })).toBeVisible();
  await expect(adminRow.getByText("Active", { exact: true })).toBeVisible();
});
