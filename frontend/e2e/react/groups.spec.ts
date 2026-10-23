import { expect, test } from "@playwright/test";

import {
  ADMIN_USER,
  MEMBER_USER,
  WORKSPACE_CHILD_GROUP,
  WORKSPACE_GROUP,
  WORKSPACE_METADATA,
  type LibraryTreeFixture,
} from "./fixtures";
import { installAppTransport, recordExternalRequests } from "./transport";

const EMPTY_WORKSPACE_TREE: LibraryTreeFixture = {
  root: {
    group_key: "alpha",
    group_path: "stock/alpha",
    visibility: "private",
    folder_id: null,
    parent_folder_id: null,
    name: "Alpha Research",
    path: "Alpha Research",
    processing_count: 0,
    children: [],
    files: [],
  },
};

test.describe("react group workspace", () => {
  for (const locale of ["en", "zh-CN"]) {
    test(`child and member read failures recover locally (${locale})`, async ({ page }) => {
      await page.setViewportSize({ width: locale === "en" ? 1280 : 600, height: 900 });
      await page.addInitScript((value) => localStorage.setItem("context69.locale", value), locale);
      const workspace = { childrenFailReads: 1, membersFailReads: 0, tree: EMPTY_WORKSPACE_TREE };
      const transport = await installAppTransport(page, { workspace });
      await page.goto("/groups/stock%2Falpha");
      const retry = locale === "en" ? "Retry" : "重试";
      const alert = page.getByRole("alert");
      await expect(alert).toContainText(locale === "en" ? "Failed to load child groups." : "加载子群组失败。");
      await expect(page.getByTestId("library-empty")).toHaveCount(0);
      await alert.getByRole("button", { name: retry }).click();
      await expect(page.getByTestId("library-group-row")).toContainText("Beta Desk");
      workspace.membersFailReads = 1;
      await page.goto("/groups/stock%2Falpha/members");
      await expect(alert).toContainText(locale === "en" ? "Failed to load members." : "加载成员失败。");
      await alert.getByRole("button", { name: retry }).click();
      await expect(page.getByTestId("members-row")).toHaveCount(2);
      await page.getByTestId("members-search").fill("absent");
      await expect(page.getByTestId("members-table")).toContainText(locale === "en" ? "No matching members." : "没有匹配的成员。");
      await page.getByTestId("members-search").fill("");
      await expect(page.getByTestId("members-row")).toHaveCount(2);
      expect(transport.unmocked).toEqual([]);
    });
  }

  test("browses groups, folders, file preview and scoped search", async ({ page }) => {
    const external = recordExternalRequests(page);
    const transport = await installAppTransport(page, {
      user: ADMIN_USER,
      workspace: {},
    });

    await page.goto("/groups");
    await expect(page.getByTestId("groups-row")).toContainText("Alpha Research");
    await page.getByRole("button", { name: "Open" }).click();
    await expect(page).toHaveURL(/\/groups\/stock%2Falpha$/);
    await expect(page.getByTestId("group-shell")).toBeVisible();
    await expect(page.getByTestId("library-group-row")).toContainText("Beta Desk");

    await page.getByRole("button", { name: /Reports.*folders/ }).click();
    await expect(page.getByTestId("library-entry-row").filter({ hasText: "quarterly.md" })).toBeVisible();

    await page.getByTestId("scoped-content-search-input").fill("policy");
    await page.getByTestId("scoped-content-search-input").press("Enter");
    await expect(page.getByTestId("scoped-search-results")).toContainText("Scoped Match");
    await page.getByRole("dialog").getByRole("button", { name: "Close" }).first().click();

    await page.getByRole("button", { name: /quarterly\.md/ }).first().click();
    const preview = page.getByRole("dialog");
    await expect(preview.getByText("Preview body from the browser suite.")).toBeVisible();
    await expect(page).toHaveURL(/file=file-ready/);

    expect(transport.unmocked).toEqual([]);
    expect(transport.calls.some((call) => call.path.endsWith("/library/tree"))).toBe(true);
    expect(transport.calls.some((call) => call.path.endsWith("/library/resources"))).toBe(true);
    expect(external).toEqual([]);
  });

  test("adds and removes a member with confirmed destructive action", async ({ page }) => {
    const transport = await installAppTransport(page, { user: ADMIN_USER, workspace: {} });

    await page.goto("/groups/stock%2Falpha/members");
    await expect(page.getByTestId("members-table")).toBeVisible();
    await page.getByTestId("members-add").click();
    await page.getByTestId("member-candidate").click();
    await page.getByRole("button", { name: "Save" }).click();

    await expect(page.getByTestId("members-row").filter({ hasText: "auditor" })).toBeVisible();
    const post = transport.calls.find((call) => call.method === "POST" && call.path.endsWith("/members"));
    expect(post?.body).toEqual({ login_name: "auditor", role: "viewer" });

    await page.getByTestId("members-row").filter({ hasText: "auditor" }).getByRole("button", { name: "Delete" }).click();
    await expect(page.getByRole("alertdialog")).toContainText("Remove member auditor?");
    await page.getByRole("alertdialog").getByRole("button", { name: "Delete" }).click();

    await expect(page.getByTestId("members-row").filter({ hasText: "auditor" })).toHaveCount(0);
    expect(transport.calls.some((call) => call.method === "DELETE" && call.path.endsWith("/members/auditor"))).toBe(true);
  });

  test("keeps member and settings administration hidden from a viewer", async ({ page }) => {
    await installAppTransport(page, {
      user: MEMBER_USER,
      workspace: { group: { ...WORKSPACE_GROUP, current_role: "viewer" } },
    });

    await page.goto("/groups/stock%2Falpha/members");
    await expect(page.getByTestId("members-table")).toBeVisible();
    await expect(page.getByTestId("members-add")).toHaveCount(0);
    await expect(page.getByRole("button", { name: "Edit" })).toHaveCount(0);

    await page.goto("/groups/stock%2Falpha/settings");
    await expect(page.getByTestId("group-settings")).toBeVisible();
    await expect(page.getByTestId("group-settings-name")).toHaveCount(0);
    await expect(page.getByRole("button", { name: "Move" })).toHaveCount(0);
    await expect(page.getByRole("button", { name: "Delete" })).toHaveCount(0);
  });

  test("saves group settings and creates a metadata index", async ({ page }) => {
    const transport = await installAppTransport(page, { user: ADMIN_USER, workspace: {} });

    await page.goto("/groups/stock%2Falpha/settings");
    await page.getByTestId("group-settings-name").fill("Alpha Renamed");
    await page.getByRole("button", { name: "Save" }).first().click();
    await expect.poll(() => transport.calls.some((call) => call.method === "PATCH" && call.path.endsWith("/stock%2Falpha"))).toBe(true);

    await page.getByTestId("metadata-source-key").fill("gov_documents");
    await page.getByTestId("metadata-source-key").press("Enter");
    await page.getByTestId("metadata-add").click();
    await page.getByTestId("metadata-path").fill("$.region");
    await page.getByRole("dialog").getByRole("button", { name: "Create" }).click();
    await expect.poll(() => transport.calls.some((call) => call.method === "POST" && call.path.endsWith("/metadata-indexes"))).toBe(true);
    await expect(page.getByTestId("metadata-row")).toContainText("$.region");
  });


  test("renders human group, visibility and role labels while sending enum codes", async ({ page }) => {
    await page.addInitScript(() => {
      if (!window.localStorage.getItem("context69.locale")) {
        window.localStorage.setItem("context69.locale", "en");
        window.localStorage.setItem("context69.theme", "light");
      }
    });
    const transport = await installAppTransport(page, { user: ADMIN_USER, workspace: {} });

    await page.goto("/groups");
    const listRow = page.getByTestId("groups-row");
    await expect(listRow).toContainText("Private");
    await expect(listRow).toContainText("Shared");
    await expect(listRow).toContainText("Owner");
    await expect(listRow).not.toContainText("private");
    await expect(listRow).not.toContainText("owner");

    await page.goto("/groups/stock%2Falpha/members");
    const members = page.getByTestId("members-table");
    await expect(members).toContainText("Owner");
    await expect(members).toContainText("Viewer");
    await expect(page.getByTestId("group-shell").locator("header")).toContainText("Owner");

    // The request still carries the generated enum code, not the label text.
    await page.getByTestId("members-add").click();
    await page.getByTestId("member-candidate").click();
    await page.getByRole("button", { name: "Save" }).click();
    await expect.poll(() => transport.calls.filter((call) => call.method === "POST" && call.path.endsWith("/members")).length).toBe(1);
    const post = transport.calls.find((call) => call.method === "POST" && call.path.endsWith("/members"));
    expect(post?.body).toEqual({ login_name: "auditor", role: "viewer" });

    await page.evaluate(() => {
      window.localStorage.setItem("context69.locale", "zh-CN");
      window.localStorage.setItem("context69.theme", "dark");
    });
    await page.reload();
    const chineseMembers = page.getByTestId("members-table");
    await expect(chineseMembers).toContainText("所有者");
    await expect(chineseMembers).toContainText("查看者");
    const chineseHeader = page.getByTestId("group-shell").locator("header");
    await expect(chineseHeader).toContainText("所有者");
    await expect(chineseHeader).toContainText("私有");
    await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");

    await page.goto("/groups");
    const chineseRow = page.getByTestId("groups-row");
    await expect(chineseRow).toContainText("私有");
    await expect(chineseRow).toContainText("共享");
    await expect(chineseRow).toContainText("所有者");

    // The library child-group row shares the same visibility label.
    await page.goto("/groups/stock%2Falpha");
    const libraryRow = page.getByTestId("library-group-row");
    await expect(libraryRow).toContainText("公开");
    await expect(libraryRow).not.toContainText("public");
    await page.evaluate(() => window.localStorage.setItem("context69.locale", "en"));
    await page.reload();
    await expect(page.getByTestId("library-group-row")).toContainText("Public");
    await expect(page.getByTestId("library-group-row")).not.toContainText("public");
    expect(transport.unmocked).toEqual([]);
  });

  test("keeps the create dialog described only by its fields", async ({ page }) => {
    const warnings: string[] = [];
    const pageErrors: string[] = [];
    page.on("console", (message) => {
      if (/Missing .*Description|aria-describedby/i.test(message.text())) {
        warnings.push(message.text());
      }
    });
    page.on("pageerror", (error) => pageErrors.push(error.message));
    await installAppTransport(page, { user: ADMIN_USER, workspace: {} });

    await page.goto("/groups");
    await page.getByTestId("groups-create").click();
    const dialog = page.getByRole("dialog");
    await expect(dialog.getByRole("heading", { name: "Create Group" })).toBeVisible();
    await expect(dialog).not.toContainText("Group name and visibility.");
    // The remaining labels still carry the field guidance.
    await expect(dialog.getByLabel("Group Key")).toBeVisible();
    await expect(dialog.getByLabel("Visibility")).toBeVisible();

    await page.keyboard.press("Escape");
    await expect(dialog).toHaveCount(0);
    await page.getByTestId("groups-create").click();
    await expect(page.getByRole("dialog")).toBeVisible();
    await page.keyboard.press("Escape");

    expect(warnings).toEqual([]);
    expect(pageErrors).toEqual([]);
  });

  test("navigates the group tabs and keeps the encoded nested path", async ({ page }) => {
    await installAppTransport(page, { user: ADMIN_USER, workspace: {} });

    await page.goto("/groups/stock%2Falpha");
    await page.getByTestId("group-shell").getByRole("link", { name: "Members" }).click();
    await expect(page).toHaveURL(/\/groups\/stock%2Falpha\/members$/);
    await expect(page.getByTestId("members-table")).toBeVisible();

    await page.getByTestId("group-shell").getByRole("link", { name: "Settings" }).click();
    await expect(page).toHaveURL(/\/groups\/stock%2Falpha\/settings$/);
    await expect(page.getByTestId("group-settings")).toBeVisible();

    await page.getByTestId("group-shell").getByRole("link", { name: "Overview" }).click();
    await expect(page).toHaveURL(/\/groups\/stock%2Falpha$/);
    await expect(page.getByTestId("library-table")).toBeVisible();
  });

  test("keeps a failed resource window recoverable", async ({ page }) => {
    const transport = await installAppTransport(page, {
      user: ADMIN_USER,
      workspace: { resourcesError: { status: 500, message: "resource window unavailable" } },
    });

    await page.goto("/groups/stock%2Falpha");

    const alert = page.getByRole("alert").filter({ hasText: "resource window unavailable" });
    await expect(alert).toBeVisible();
    await expect(alert.getByRole("button", { name: "Retry" })).toBeVisible();
    expect(transport.unmocked).toEqual([]);
  });

  test("uploads one pending file, reuses a prepared file and carries the release policy", async ({ page }) => {
    const transport = await installAppTransport(page, {
      user: ADMIN_USER,
      workspace: { reuseFiles: ["quarterly.md"] },
    });

    await page.goto("/groups/stock%2Falpha");
    await page.getByTestId("release-source-after-upload").click();
    await page.getByTestId("library-upload-input").setInputFiles([
      { name: "notes.txt", mimeType: "text/plain", buffer: Buffer.from("upload-body") },
      { name: "quarterly.md", mimeType: "text/markdown", buffer: Buffer.from("old-body") },
    ]);

    await expect.poll(() => transport.calls.filter((call) => call.path.endsWith("/files/prepare-upload")).length).toBe(2);
    const prepares = transport.calls.filter((call) => call.path.endsWith("/files/prepare-upload"));
    expect(prepares.map((call) => (call.body as { filename: string }).filename).sort())
      .toEqual(["notes.txt", "quarterly.md"]);
    for (const prepare of prepares) {
      expect((prepare.body as { sha256: string }).sha256).toMatch(/^[a-f0-9]{64}$/);
      expect((prepare.body as { options: { source_policy: string } }).options.source_policy)
        .toBe("release_after_processing");
    }

    await expect.poll(() => transport.calls.filter((call) => call.path.endsWith("/files/upload")).length).toBe(1);
    const upload = transport.calls.find((call) => call.path.endsWith("/files/upload"));
    expect(upload?.rawBody?.match(/name="sha256"/g) ?? []).toHaveLength(1);
    expect(upload?.rawBody?.match(/name="files"/g) ?? []).toHaveLength(1);
    expect(upload?.rawBody).toContain("notes.txt");
    expect(upload?.rawBody).not.toContain("quarterly.md");
    expect(upload?.rawBody).toContain("release_after_processing");

    await expect(page.getByTestId("library-entry-row").filter({ hasText: "quarterly.md" })).toBeVisible();
    expect(transport.unmocked).toEqual([]);
  });

  test("cancels a multi-selection delete without a request and deletes after confirmation", async ({ page }) => {
    const transport = await installAppTransport(page, { user: ADMIN_USER, workspace: {} });

    await page.goto("/groups/stock%2Falpha");
    await page.getByRole("button", { name: /Reports.*folders/ }).click();
    const rows = page.getByTestId("library-entry-row");
    await expect(rows).toHaveCount(2);
    for (const row of await rows.all()) {
      await row.getByRole("checkbox").click();
    }
    await expect(page.getByTestId("library-delete-selected")).toContainText("(2)");

    await page.getByTestId("library-delete-selected").click();
    await expect(page.getByRole("alertdialog")).toContainText("Delete 2 selected items?");
    await page.keyboard.press("Escape");
    await expect(page.getByRole("alertdialog")).toHaveCount(0);
    expect(transport.calls.filter((call) => call.method === "DELETE")).toHaveLength(0);

    await page.getByTestId("library-delete-selected").click();
    await page.getByRole("alertdialog").getByRole("button", { name: "Delete" }).click();
    await expect.poll(() => transport.calls.filter((call) => call.method === "DELETE").length).toBe(2);
    expect(transport.calls.filter((call) => call.method === "DELETE").map((call) => call.path).sort()).toEqual([
      "/v1/groups/by-path/stock%2Falpha/library/files/file-failed",
      "/v1/groups/by-path/stock%2Falpha/library/files/file-ready",
    ]);
  });

  test("starts a folder at page one and does not resume the root's later page", async ({ page }) => {
    const transport = await installAppTransport(page, {
      user: ADMIN_USER,
      workspace: { extraRootFolders: 51 },
    });
    const rootPageReads = (pageNumber: number) => transport.calls.filter((call) => (
      call.path.endsWith("/library/resources")
      && !call.query.includes("folder_id=")
      && !call.query.includes("recursive=true")
      && new URLSearchParams(call.query).get("page") === String(pageNumber)
    )).length;

    await page.goto("/groups/stock%2Falpha");
    await expect(page.getByTestId("library-table")).toBeVisible();
    await expect(page.getByText("Page 1 of 2")).toBeVisible();
    await page.getByRole("button", { name: "Next" }).click();
    await expect(page.getByText("Page 2 of 2")).toBeVisible();
    expect(rootPageReads(2)).toBeGreaterThan(0);

    await page.getByRole("button", { name: /Reports.*folders/ }).click();
    await expect(page.getByText("quarterly.md")).toBeVisible();

    const folderCall = transport.calls.find((call) => (
      call.path.endsWith("/library/resources") && call.query.includes("folder_id=folder-reports")
    ));
    expect(folderCall?.query).toContain("page=1");
    expect(folderCall?.query).not.toContain("page=2");

    // Returning to the root is a new effective scope: the page two window is
    // never resumed, and the visible window starts at one again.
    const pageTwoReads = rootPageReads(2);
    await page.getByTestId("library-go-up").click();
    await expect(page).toHaveURL(/\/groups\/stock%2Falpha$/);
    await expect(page.getByText("Page 1 of 2")).toBeVisible();
    expect(rootPageReads(2)).toBe(pageTwoReads);

    // A second visit repeats the boundary instead of restoring page two.
    await page.getByRole("button", { name: "Next" }).click();
    await expect(page.getByText("Page 2 of 2")).toBeVisible();
    await page.getByRole("button", { name: /Reports.*folders/ }).click();
    await expect(page.getByText("quarterly.md")).toBeVisible();
    const secondFolderCall = transport.calls.filter((call) => (
      call.path.endsWith("/library/resources") && call.query.includes("folder_id=folder-reports")
    )).at(-1);
    expect(secondFolderCall?.query).toContain("page=1");
    expect(secondFolderCall?.query).not.toContain("page=2");
    expect(transport.unmocked).toEqual([]);
  });

  test("reports the deletion outcome from the task, not from the accepted request", async ({ page }) => {
    const transport = await installAppTransport(page, {
      user: ADMIN_USER,
      workspace: { deletion: { states: ["running", "failed"] } },
    });

    await page.goto("/groups/stock%2Falpha");
    await page.getByRole("button", { name: /Reports.*folders/ }).click();
    const row = page.getByTestId("library-entry-row").filter({ hasText: "quarterly.md" });
    await row.getByRole("checkbox").click();
    await page.getByTestId("library-delete-selected").click();
    await page.getByRole("alertdialog").getByRole("button", { name: "Delete" }).click();

    await expect(page.getByText("The stored object could not be deleted.")).toBeVisible();
    await expect(page.getByText("1 items deleted.")).toHaveCount(0);
    expect(transport.calls.filter((call) => (
      call.method === "GET" && call.path === "/v1/tasks/task-library-delete"
    )).length).toBeGreaterThanOrEqual(2);
    expect(transport.unmocked).toEqual([]);
  });

  test("stops observing a started retry after leaving the library", async ({ page }) => {
    const transport = await installAppTransport(page, {
      user: ADMIN_USER,
      workspace: { runningTaskId: "task-library-retry" },
    });
    const taskReads = () => transport.calls.filter((call) => (
      call.method === "GET" && call.path === "/v1/tasks/task-library-retry"
    )).length;

    await page.goto("/groups/stock%2Falpha");
    await page.getByRole("button", { name: /Reports.*folders/ }).click();
    await page.getByText("broken.pdf").waitFor();
    await page.getByRole("row", { name: /broken\.pdf/ }).getByRole("button", { name: "Retry" }).click();

    await expect.poll(taskReads).toBe(1);
    expect(await taskReads()).toBeGreaterThanOrEqual(1);

    // Client-side navigation out of the workspace disposes the observation.
    await page.getByTestId("app-breadcrumbs").getByRole("link", { name: "Browser" }).click();
    await expect(page).toHaveURL(/\/groups$/);
    const readsAfterLeaving = taskReads();
    await page.waitForTimeout(3_500);
    expect(taskReads()).toBe(readsAfterLeaving);
    expect(transport.unmocked).toEqual([]);
  });

  test("never observes a retry admitted after leaving the library", async ({ page }) => {
    const transport = await installAppTransport(page, {
      user: ADMIN_USER,
      workspace: { runningTaskId: "task-library-retry", taskAdmissionDelayMs: 1_500 },
    });
    const taskReads = () => transport.calls.filter((call) => (
      call.method === "GET" && call.path === "/v1/tasks/task-library-retry"
    )).length;

    await page.goto("/groups/stock%2Falpha");
    await page.getByRole("button", { name: /Reports.*folders/ }).click();
    await page.getByText("broken.pdf").waitFor();
    await page.getByRole("row", { name: /broken\.pdf/ }).getByRole("button", { name: "Retry" }).click();

    // Leave while the submission is still pending; its answer arrives afterwards.
    await page.getByTestId("app-breadcrumbs").getByRole("link", { name: "Browser" }).click();
    await expect(page).toHaveURL(/\/groups$/);
    await page.waitForTimeout(3_000);

    expect(taskReads()).toBe(0);
    expect(transport.calls.some((call) => call.method === "POST" && call.path === "/v1/tasks")).toBe(true);
    expect(transport.unmocked).toEqual([]);
  });

  test("retries and deletes a metadata index", async ({ page }) => {
    const failedIndex = {
      ...WORKSPACE_METADATA,
      index_id: "77777777-7777-4777-8777-777777777777",
      path: "$.status",
      status: "failed" as const,
    };
    const transport = await installAppTransport(page, {
      user: ADMIN_USER,
      workspace: { metadata: [failedIndex, WORKSPACE_METADATA] },
    });

    await page.goto("/groups/stock%2Falpha/settings");
    await page.getByTestId("metadata-source-key").fill("gov_documents");
    await page.getByTestId("metadata-source-key").press("Enter");
    const failedRow = page.getByTestId("metadata-row").filter({ hasText: "$.status" });
    await expect(failedRow).toBeVisible();
    await failedRow.getByRole("button", { name: "Retry" }).click();
    await expect.poll(() => transport.calls.some((call) => call.method === "POST" && call.path.endsWith("/retry"))).toBe(true);

    const readyRow = page.getByTestId("metadata-row").filter({ hasText: "$.region" });
    await readyRow.getByRole("button", { name: "Delete" }).click();
    await expect.poll(() => transport.calls.some((call) => call.method === "DELETE" && call.path.includes("55555555"))).toBe(true);
    await expect(page.getByTestId("metadata-row").filter({ hasText: "$.region" })).toHaveCount(0);
  });

  test("saves a source folder configuration and syncs it", async ({ page }) => {
    const transport = await installAppTransport(page, { user: ADMIN_USER, workspace: {} });

    await page.goto("/groups/stock%2Falpha");
    await page.getByTestId("library-entry-row").filter({ hasText: "Sources" })
      .getByRole("button", { name: /folders/ }).click();
    const sourceFileRow = page.getByTestId("library-entry-row").filter({ hasText: "source.json" });
    await sourceFileRow.getByRole("button", { name: "Actions" }).click();
    await page.getByRole("menuitem", { name: "Edit source.json" }).click();

    const dialog = page.getByRole("dialog");
    await expect(dialog).toBeVisible();
    await dialog.getByLabel("Source Key").fill("gov_documents_v2");
    await dialog.getByRole("button", { name: "Save" }).click();

    await expect.poll(() => transport.calls.filter((call) => call.method === "PUT" && call.path.endsWith("/config")).length).toBe(1);
    const save = transport.calls.find((call) => call.method === "PUT" && call.path.endsWith("/config"));
    expect(save?.body).toMatchObject({
      source_key: "gov_documents_v2",
      connector_type: "postgres_sql",
      sync_strategy: "cursor",
    });
    await expect(dialog).toHaveCount(0);
    await page.getByTestId("library-go-up").click();

    await page.getByTestId("library-entry-row").filter({ hasText: "Sources" })
      .getByRole("button", { name: "Actions" }).click();
    await page.getByRole("menuitem", { name: "Sync" }).click();
    await expect.poll(() => transport.calls.filter((call) => call.method === "POST" && call.path.endsWith("/sync")).length).toBe(1);
    expect(transport.unmocked).toEqual([]);
  });

  test("renders the workspace in English/light/desktop and Chinese/dark/narrow layouts", async ({ page }) => {
    await page.setViewportSize({ width: 1280, height: 900 });
    await page.addInitScript(() => {
      if (!window.localStorage.getItem("context69.locale")) {
        window.localStorage.setItem("context69.locale", "en");
        window.localStorage.setItem("context69.theme", "light");
      }
    });
    await installAppTransport(page, { user: ADMIN_USER, workspace: {} });

    await page.goto("/groups/stock%2Falpha");
    await expect(page.getByRole("link", { name: "Overview" })).toBeVisible();
    await expect(page.getByRole("link", { name: "Members" })).toBeVisible();
    await expect(page.getByTestId("library-table")).toBeVisible();
    await expect(page.locator("html")).toHaveAttribute("data-theme", "light");

    await page.setViewportSize({ width: 390, height: 844 });
    await page.evaluate(() => {
      window.localStorage.setItem("context69.locale", "zh-CN");
      window.localStorage.setItem("context69.theme", "dark");
    });
    await page.reload();

    await expect(page.getByRole("link", { name: "概览" })).toBeVisible();
    await expect(page.getByRole("link", { name: "成员" })).toBeVisible();
    await expect(page.getByTestId("library-table")).toBeVisible();
    await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
  });

  test("keeps the child-group search usable through a result without a match", async ({ page }) => {
    const transport = await installAppTransport(page, {
      user: ADMIN_USER,
      workspace: { children: [WORKSPACE_CHILD_GROUP], tree: EMPTY_WORKSPACE_TREE },
    });

    await page.goto("/groups/stock%2Falpha");
    const search = page.getByTestId("child-group-search");
    await expect(search).toBeVisible();
    await expect(page.getByTestId("library-group-row")).toContainText("Beta Desk");

    await search.fill("no-such-child");
    await expect(search).toBeVisible();
    await expect(search).toHaveValue("no-such-child");
    await expect(search).toBeFocused();
    await expect(page.getByTestId("library-empty")).toContainText("No matching child groups.");

    await search.fill("");
    await expect(page.getByTestId("library-group-row")).toContainText("Beta Desk");
    expect(transport.unmocked).toEqual([]);
  });

  test("keeps the translation defaults unwritten until the group loads them", async ({ page }) => {
    const transport = await installAppTransport(page, {
      user: ADMIN_USER,
      workspace: { translationFailReads: 1 },
    });

    await page.goto("/groups/stock%2Falpha/settings");
    await expect(page.getByTestId("group-translation-retry")).toBeVisible();
    await expect(page.getByLabel("Target locales")).toBeDisabled();
    await expect(page.getByLabel("Default source locale")).toBeDisabled();
    await expect(page.getByTestId("group-translation-save")).toBeDisabled();
    expect(transport.calls.filter((call) => call.method === "PUT" && call.path.endsWith("/translation-settings"))).toHaveLength(0);

    await page.getByTestId("group-translation-retry").click();
    await expect(page.getByLabel("Target locales")).toBeEnabled();
    await expect(page.getByLabel("Target locales")).toHaveValue("zh-CN");
    await expect(page.getByLabel("Default source locale")).toHaveValue("en-US");

    await page.getByLabel("Default source locale").fill("en-GB");
    await page.getByTestId("group-translation-save").click();

    await expect.poll(() => transport.calls.filter((call) => call.method === "PUT" && call.path.endsWith("/translation-settings")).length).toBe(1);
    const save = transport.calls.find((call) => call.method === "PUT" && call.path.endsWith("/translation-settings"));
    expect(save?.body).toEqual({
      enabled: true,
      default_target_locales: ["zh-CN"],
      source_locale: "en-GB",
      glossary: [],
    });
    expect(transport.unmocked).toEqual([]);
  });

  test("keeps the translation defaults read-only for a viewer", async ({ page }) => {
    const transport = await installAppTransport(page, {
      user: MEMBER_USER,
      workspace: { group: { ...WORKSPACE_GROUP, current_role: "viewer" } },
    });

    await page.goto("/groups/stock%2Falpha/settings");
    await expect(page.getByTestId("group-translation-settings")).toBeVisible();
    await expect(page.getByLabel("Target locales")).toBeDisabled();
    await expect(page.getByTestId("group-translation-save")).toHaveCount(0);
    expect(transport.calls.filter((call) => call.method === "PUT")).toHaveLength(0);
  });

  test("opens the source configuration JSON in a real Monaco editor", async ({ page }) => {
    const errors: string[] = [];
    const workers: string[] = [];
    page.on("pageerror", (error) => errors.push(error.message));
    page.on("console", (message) => {
      if (message.type() === "error" && /Missing requestHandler|doValidation|findDocumentColors/.test(message.text())) {
        errors.push(message.text());
      }
    });
    page.on("worker", (worker) => workers.push(worker.url()));
    await installAppTransport(page, { user: ADMIN_USER, workspace: {} });

    await page.goto("/groups/stock%2Falpha");
    await page.getByTestId("library-create-menu").click();
    await page.getByRole("menuitem", { name: "New Source Folder" }).click();
    const dialog = page.getByRole("dialog");
    await expect(dialog).toBeVisible();
    await dialog.getByRole("tab", { name: "JSON" }).click();

    await expect(dialog.locator(".monaco-editor")).toBeVisible();
    await expect(dialog.locator(".monaco-editor .view-lines")).not.toHaveCount(0);
    await expect.poll(() => workers.length).toBeGreaterThan(0);
    await expect.poll(() => workers.some((url) => /json\.worker/.test(url))).toBe(true);

    await dialog.locator(".monaco-editor .view-lines").click();
    await page.keyboard.press("ControlOrMeta+A");
    await page.keyboard.type('{"source_key": }');
    await expect(dialog.locator(".monaco-editor .squiggly-error").first()).toBeVisible();
    expect(errors).toEqual([]);
  });
});
