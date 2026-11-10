import { expect, test } from "@playwright/test";

import { ADMIN_USER, GIT_FILES, GIT_SOURCE, MEMBER_USER, WORKSPACE_GROUP } from "./fixtures";
import { installAppTransport, recordExternalRequests } from "./transport";

test.describe("react group git sources", () => {
  test("browses, searches, configures and requests a Git snapshot", async ({ page }) => {
    const external = recordExternalRequests(page);
    const transport = await installAppTransport(page, {
      user: ADMIN_USER,
      workspace: {},
      git: {},
    });

    await page.goto("/groups/stock%2Falpha/git");
    await expect(page.getByTestId("group-shell").getByRole("link", { name: "Git" })).toBeVisible();
    await expect(page.getByTestId("git-sources-table")).toContainText("context69");
    await expect(page.getByTestId("git-connections-table")).toContainText("Research Forgejo");

    const row = page.getByTestId("git-source-row").filter({ hasText: "context69" });
    await expect(row).toContainText("Configured pin: 2222222222");
    await row.getByRole("button", { name: "Browse" }).click();
    await expect(page.getByTestId("git-source-detail")).toContainText("research/context69");
    await expect(page.getByTestId("git-source-detail")).toContainText("Observed target");
    await expect(page.getByTestId("git-source-detail")).toContainText("Configured pin");
    await expect(page.getByTestId("git-source-detail")).toContainText("222222222222");
    await expect(page.getByTestId("git-files-table")).toContainText("src/main.rs");

    await page.getByTestId("git-files-table").getByRole("button", { name: "src/main.rs" }).click();
    await expect(page.getByTestId("git-file-content")).toContainText("println");

    await page.getByTestId("git-code-search").getByLabel("Query").fill("research");
    await page.getByTestId("git-code-search").getByRole("button", { name: "Search" }).click();
    await expect(page.getByTestId("git-code-hits")).toContainText("src/main.rs");
    await page.getByTestId("git-code-hits").getByRole("button").first().click();
    await expect(page.getByTestId("git-file-content")).toContainText("println");
    await expect(page.getByTestId("git-source-detail")).toContainText("requested hybrid");

    await row.getByRole("button", { name: "Sync" }).click();
    const indexPath = "/git-repositories/88888888-8888-4888-8888-888888888888/index";
    const indexCalls = () => transport.calls.filter((call) => (
      call.method === "POST" && call.path.endsWith(indexPath)
    ));
    await expect.poll(() => indexCalls().length).toBe(1);
    expect(indexCalls()[0]?.body).toEqual({ force_rebuild: false });
    await expect(row).toContainText("Indexing");

    await row.getByRole("button", { name: "Rebuild" }).click();
    await expect.poll(() => indexCalls().length).toBe(2);
    expect(indexCalls()[1]?.body).toEqual({ force_rebuild: true });
    await expect(row).toContainText("Indexing");

    await expect(page.getByTestId("git-files-provenance")).toContainText("Generation 5", { timeout: 10_000 });
    await expect(page.getByTestId("git-content-provenance")).toContainText("Generation 5");
    await expect(page.getByTestId("git-search-provenance")).toContainText("Generation 5");
    await expect(page.getByTestId("git-file-content")).toContainText("generation five");
    await expect(page.getByTestId("git-code-hits")).toContainText("generation five");
    await expect(row).toContainText("Observed target: 3333333333");

    await row.getByRole("button", { name: "Edit" }).click();
    const editDialog = page.getByRole("dialog");
    await expect(editDialog.getByLabel("Refresh policy")).toHaveCount(0);
    await editDialog.getByLabel("Periodic interval (seconds)").fill("900");
    await editDialog.getByLabel("Target ref").fill("refs/heads/develop");
    await editDialog.getByRole("button", { name: "Save" }).click();
    await expect.poll(() => transport.calls.some((call) => call.method === "PATCH" && call.path.endsWith("/git-repositories/88888888-8888-4888-8888-888888888888"))).toBe(true);
    const patch = transport.calls.find((call) => call.method === "PATCH" && call.path.endsWith("/git-repositories/88888888-8888-4888-8888-888888888888"));
    expect(patch?.body).toMatchObject({
      interval_seconds: 900,
      retention_count: 2,
      target_ref: "refs/heads/develop",
      pinned_commit: { op: "keep" },
      index_profile: "hybrid",
    });

    await row.getByRole("button", { name: "Webhook" }).click();
    const webhookDialog = page.getByRole("dialog");
    await expect(webhookDialog).toContainText("research-context69-hook");
    await expect(webhookDialog).toContainText("/v1/webhooks/forgejo/research-context69-hook");
    await expect(webhookDialog).toContainText("X-Forgejo-Signature");
    await webhookDialog.getByRole("button", { name: "Close" }).first().click();

    await page.getByRole("button", { name: "Add repository" }).click();
    const createDialog = page.getByRole("dialog");
    await createDialog.getByLabel("Canonical repository URL").fill("https://github.com/cloudiful/context69");
    await createDialog.getByLabel("Default branch").fill("main");
    await createDialog.getByLabel("Target ref").fill("refs/heads/main");
    await createDialog.getByRole("button", { name: "Register repository" }).click();
    await expect.poll(() => transport.calls.some((call) => call.method === "POST" && call.path.endsWith("/git-repositories"))).toBe(true);
    const created = transport.calls.find((call) => call.method === "POST" && call.path.endsWith("/git-repositories"));
    expect(created?.body).toMatchObject({
      provider: "github",
      canonical_url: "https://github.com/cloudiful/context69",
      default_branch: "main",
      target_ref: "refs/heads/main",
    });

    await row.getByRole("button", { name: "Delete" }).click();
    const deleteDialog = page.getByRole("alertdialog");
    await expect(deleteDialog).toContainText("research/context69");
    await deleteDialog.getByRole("button", { name: "Delete" }).click();
    await expect.poll(() => transport.calls.some((call) => (
      call.method === "DELETE"
      && call.path.endsWith("/git-repositories/88888888-8888-4888-8888-888888888888")
    ))).toBe(true);
    await expect(page.getByTestId("git-sources-table")).toContainText("No Git repositories yet.");

    expect(transport.unmocked).toEqual([]);
    expect(external).toEqual([]);
  });

  test("keeps Git administration read-only for a viewer", async ({ page }) => {
    const transport = await installAppTransport(page, {
      user: MEMBER_USER,
      workspace: { group: { ...WORKSPACE_GROUP, current_role: "viewer" } },
      git: {},
    });

    await page.goto("/groups/stock%2Falpha/git");
    await expect(page.getByTestId("git-sources-table")).toContainText("context69");
    await expect(page.getByRole("button", { name: "Add repository" })).toHaveCount(0);
    await expect(page.getByRole("button", { name: "Add connection" })).toHaveCount(0);
    await expect(page.getByTestId("git-source-row").getByRole("button", { name: "Sync" })).toHaveCount(0);
    await expect(page.getByTestId("git-source-row").getByRole("button", { name: "Edit" })).toHaveCount(0);
    await expect(page.getByTestId("git-source-row").getByRole("button", { name: "Delete" })).toHaveCount(0);
    await expect(page.getByTestId("git-connections-table").getByRole("button", { name: "Disable" })).toHaveCount(0);

    await page.getByTestId("git-source-row").getByRole("button", { name: "Browse" }).click();
    await expect(page.getByTestId("git-source-detail")).toBeVisible();
    await expect(page.getByTestId("git-files-table")).toContainText("src/main.rs");
    expect(transport.calls.filter((call) => call.method === "PATCH" || call.method === "POST")).toEqual([]);
    expect(transport.unmocked).toEqual([]);
  });

  test("renders a bounded failure code for a failed source", async ({ page }) => {
    const transport = await installAppTransport(page, {
      user: ADMIN_USER,
      workspace: {},
      git: {
        sources: [{
          ...GIT_SOURCE,
          index_status: "failed",
          last_error_code: "provider_unavailable",
        }],
      },
    });

    await page.goto("/groups/stock%2Falpha/git");
    await expect(page.getByTestId("git-source-row")).toContainText("provider_unavailable");

    await page.getByTestId("git-source-row").getByRole("button", { name: "Browse" }).click();
    await expect(page.getByTestId("git-source-error-code")).toHaveText("provider_unavailable");
    expect(transport.unmocked).toEqual([]);
  });

  test("pages files forward and back with history cursors", async ({ page }) => {
    const filePage = (path: string) => ({ ...GIT_FILES[0], path });
    const transport = await installAppTransport(page, {
      user: ADMIN_USER,
      workspace: {},
      git: {
        filePages: [
          [filePage("src/main.rs")],
          [filePage("src/lib.rs")],
          [filePage("src/util.rs")],
        ],
      },
    });

    await page.goto("/groups/stock%2Falpha/git");
    await page.getByTestId("git-source-row").getByRole("button", { name: "Browse" }).click();
    const detail = page.getByTestId("git-source-detail");
    await expect(page.getByTestId("git-files-table")).toContainText("src/main.rs");

    await detail.getByRole("button", { name: "Next", exact: true }).click();
    await expect(page.getByTestId("git-files-table")).toContainText("src/lib.rs");
    await detail.getByRole("button", { name: "Next", exact: true }).click();
    await expect(page.getByTestId("git-files-table")).toContainText("src/util.rs");

    await detail.getByRole("button", { name: "Previous" }).click();
    await expect(page.getByTestId("git-files-table")).toContainText("src/lib.rs");
    await detail.getByRole("button", { name: "Previous" }).click();
    await expect(page.getByTestId("git-files-table")).toContainText("src/main.rs");
    expect(transport.unmocked).toEqual([]);
  });

  test("keeps polling after more than sixty source updates", async ({ page }) => {
    const transport = await installAppTransport(page, {
      user: ADMIN_USER,
      workspace: {},
      git: {},
    });

    await page.goto("/groups/stock%2Falpha/git");
    const row = page.getByTestId("git-source-row");
    const toggle = row.getByRole("button", { name: /^(Disable|Enable)$/ });
    await expect(toggle).toBeEnabled();

    const patchCalls = () => transport.calls.filter((call) => (
      call.method === "PATCH" && call.path.endsWith("/git-repositories/88888888-8888-4888-8888-888888888888")
    )).length;
    const listCalls = () => transport.calls.filter((call) => (
      call.method === "GET" && call.path.endsWith("/git-repositories")
    )).length;

    for (let index = 0; index < 62; index += 1) {
      const patchesBefore = patchCalls();
      const listsBefore = listCalls();
      await toggle.click();
      await expect.poll(patchCalls).toBe(patchesBefore + 1);
      await expect.poll(listCalls).toBeGreaterThan(listsBefore);
      await expect(toggle).toBeEnabled();
    }

    await row.getByRole("button", { name: "Sync" }).click();
    await expect.poll(() => transport.calls.some((call) => (
      call.method === "POST" && call.path.endsWith("/index")
    ))).toBe(true);
    await expect(row).toContainText("Observed target: 3333333333", { timeout: 10_000 });
    expect(transport.unmocked).toEqual([]);
  });
});
