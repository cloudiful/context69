import { webcrypto } from "node:crypto";
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { toast } from "sonner";
import { createMemoryRouter, RouterProvider } from "react-router";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

import { AppProviders } from "@/app/providers";
import { appRoutes } from "@/app/router";
import { AppToaster } from "@/app/toaster";
import { clearSession } from "@/lib/auth-store";
import { setLocale } from "@/lib/preferences-store";
import { installDomPolyfills } from "@/features/search/test-support";
import type { LibraryResourceItem, SearchHit, TaskResponse } from "@/services/api";

import {
  authenticate,
  fileDetail,
  installGroupBackend,
  pagination,
  TEST_USER,
} from "../groups/test-support";

function libraryHit(overrides: Partial<SearchHit>): SearchHit {
  return {
    chunk_id: "chunk-1",
    chunk_index: 0,
    chunk_text: "matching body",
    document_id: 1,
    external_id: "ext-1",
    group_key: "alpha",
    group_path: "stock/alpha",
    source_key: "gov_documents",
    source_uri: "",
    visibility: "private",
    match_reason: "keyword",
    published_at: null,
    title: "Quarterly",
    score: 1,
    is_library_file: true,
    library_file_id: "file-ready",
    library_path: "Alpha/Reports",
    ...overrides,
  };
}

const RESOURCES_PATH = "/v1/groups/by-path/{group_path}/library/resources";
const TASKS_PATH = "/v1/tasks/{task_id}";

function taskResponse(overrides: Partial<TaskResponse> = {}): TaskResponse {
  return {
    task_id: "task-delete",
    kind: "file_batch",
    origin: "manual",
    status: "succeeded",
    group_path: "stock/alpha",
    source_key: null,
    stage: "done",
    waiting_reason: null,
    dependency_key: null,
    failure_stage: null,
    error_summary: null,
    eta_seconds: null,
    created_at: "2026-01-01T00:00:00Z",
    updated_at: "2026-01-01T00:00:05Z",
    progress: { total: 1, queued: 0, running: 0, waiting: 0, succeeded: 1, failed: 0, cancelled: 0 },
    ...overrides,
  };
}

function folderResourceItem(index: number): LibraryResourceItem {
  const id = index === 0 ? "folder-reports" : `folder-${index}`;
  return {
    kind: "folder",
    id,
    group_key: "alpha",
    group_path: "stock/alpha",
    visibility: "private",
    parent_folder_id: null,
    name: index === 0 ? "Reports" : `Archive ${index}`,
    updated_at: "2026-01-02T00:00:00Z",
    created_at: "2026-01-01T00:00:00Z",
    child_folder_count: 0,
    file_count: 0,
    is_source_folder: false,
    is_source_records_folder: false,
    processing_count: 0,
  };
}

function fileResourceItem(index: number): LibraryResourceItem {
  return {
    kind: "file",
    id: `file-${index}`,
    group_key: "alpha",
    group_path: "stock/alpha",
    visibility: "private",
    parent_folder_id: "folder-reports",
    name: `report-${index}.md`,
    media_type: "text/markdown",
    size_bytes: 120,
    ingest_status: "succeeded",
    error_message: null,
    updated_at: "2026-01-02T00:00:00Z",
    created_at: "2026-01-01T00:00:00Z",
    child_folder_count: 0,
    file_count: 0,
    is_source_folder: false,
    is_source_records_folder: false,
    processing_count: 0,
  };
}

function accepted<T>(data: T) {
  return { data, response: { ok: true, status: 200 } };
}

type Backend = ReturnType<typeof installGroupBackend>;

function record(backend: Backend, method: string, path: string, init?: {
  params?: { path?: Record<string, string>; query?: Record<string, unknown> };
  signal?: AbortSignal;
}) {
  backend.calls.push({ method, path, params: init?.params, body: undefined, signal: init?.signal });
}

function installPagedBackend(): Backend {
  const backend = installGroupBackend();
  const originalGet = backend.spies.GET.getMockImplementation() as
    (path: string, init?: never) => Promise<unknown>;
  const rootItems = Array.from({ length: 60 }, (_, index) => folderResourceItem(index));
  const reportFiles = Array.from({ length: 60 }, (_, index) => (
    index === 50
      ? { ...fileResourceItem(index), id: "file-ready", name: "quarterly.md" }
      : index === 51
        ? { ...fileResourceItem(index), id: "file-failed", name: "broken.pdf", ingest_status: "failed" as const }
        : fileResourceItem(index)
  ));

  backend.spies.GET.mockImplementation(((path: string, init?: never) => {
    const request = init as {
      params?: { path?: Record<string, string>; query?: Record<string, unknown> };
      signal?: AbortSignal;
    } | undefined;
    if (path !== RESOURCES_PATH) {
      return originalGet(path, init);
    }
    record(backend, "GET", path, request);
    const query = request?.params?.query ?? {};
    const folderId = query.folder_id;
    const page = Number(query.page ?? 1);
    const pageSize = Number(query.page_size ?? 50);
    const items = folderId === "folder-reports" ? reportFiles : folderId ? [] : rootItems;
    const start = (page - 1) * pageSize;
    return Promise.resolve(accepted({
      items: items.slice(start, start + pageSize),
      pagination: pagination(page, items.length, pageSize),
    }));
  }) as never);

  return backend;
}

function pageList(backend: Backend, filter: (call: Backend["calls"][number]) => boolean): number[] {
  return backend.callsFor("GET", RESOURCES_PATH)
    // The recursive failed-count probe describes the window, not its page.
    .filter((call) => call.params?.query?.recursive !== true)
    .filter(filter)
    .map((call) => Number(call.params?.query?.page ?? 1));
}

function rootPages(backend: Backend): number[] {
  return pageList(backend, (call) => !call.params?.query?.folder_id);
}

function folderPages(backend: Backend, folderId = "folder-reports"): number[] {
  return pageList(backend, (call) => call.params?.query?.folder_id === folderId);
}

/** Resource reads recorded after `mark`, split by the window they belong to. */
function readsSince(backend: Backend, mark: number) {
  const reads = backend.callsFor("GET", RESOURCES_PATH)
    .slice(mark)
    .filter((call) => call.params?.query?.recursive !== true);
  return {
    root: reads
      .filter((call) => !call.params?.query?.folder_id)
      .map((call) => Number(call.params?.query?.page ?? 1)),
    folder: reads
      .filter((call) => call.params?.query?.folder_id === "folder-reports")
      .map((call) => Number(call.params?.query?.page ?? 1)),
  };
}

async function renderOverviewView(options: { toasts?: boolean } = {}) {
  const router = createMemoryRouter(appRoutes, { initialEntries: ["/groups/stock%2Falpha"] });
  const view = render(
    <AppProviders>
      <RouterProvider router={router} />
      {options.toasts ? <AppToaster /> : null}
    </AppProviders>,
  );
  await screen.findByTestId("group-shell");
  await screen.findByTestId("library-table");
  return { router, unmount: view.unmount };
}

async function renderOverview(options: { toasts?: boolean } = {}) {
  const { router } = await renderOverviewView(options);
  return router;
}

async function openFolder(
  user: ReturnType<typeof userEvent.setup>,
  expectedFileName: string,
) {
  const table = screen.getByTestId("library-table");
  const folderRow = (await screen.findAllByTestId("library-entry-row"))
    .find((row) => row.textContent?.includes("Reports")) as HTMLElement;
  const folderButton = within(folderRow)
    .getAllByRole("button")
    .find((button) => button.textContent?.includes("folders"));
  await user.click(folderButton as HTMLElement);
  await waitFor(() => {
    expect(screen.getByText(expectedFileName)).toBeInTheDocument();
  });
  return table;
}

async function openReports(user: ReturnType<typeof userEvent.setup>) {
  return openFolder(user, "quarterly.md");
}

describe("group library", () => {
  beforeAll(() => installDomPolyfills());

  beforeEach(async () => {
    clearSession(null);
    setLocale("en");
    vi.stubGlobal("fetch", vi.fn(async () => new Response(
      JSON.stringify({ user: TEST_USER }),
      { status: 200, headers: { "Content-Type": "application/json" } },
    )));
    await authenticate();
  });

  afterEach(() => {
    toast.dismiss();
    vi.useRealTimers();
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
  });

  it("shows child groups and folders, and opens a folder through the URL-scoped window", async () => {
    const backend = installGroupBackend({
      children: [{
        group_id: 2,
        group_key: "beta",
        group_path: "beta",
        name: "Beta",
        visibility: "public",
        kind: "shared",
        current_role: "viewer",
        created_at: "2026-01-01T00:00:00Z",
        updated_at: "2026-01-01T00:00:00Z",
      }],
    });
    const user = userEvent.setup();
    const router = await renderOverview();

    const groupRow = await screen.findByTestId("library-group-row");
    expect(groupRow).toHaveTextContent("Beta");
    expect(groupRow).toHaveTextContent("Public");
    expect(groupRow).not.toHaveTextContent("public");
    await openReports(user);

    expect(router.state.location.search).toContain("folder=folder-reports");
    await waitFor(() => {
      const calls = backend.callsFor("GET", "/v1/groups/by-path/{group_path}/library/resources");
      expect(calls.at(-1)?.params?.query).toMatchObject({ folder_id: "folder-reports", page: 1 });
    });
    expect(await screen.findByText("quarterly.md")).toBeInTheDocument();
    expect(screen.getByText("broken.pdf")).toBeInTheDocument();
    const treeCall = backend.callsFor("GET", "/v1/groups/by-path/{group_path}/library/tree").at(-1);
    const resourceCall = backend.callsFor("GET", "/v1/groups/by-path/{group_path}/library/resources").at(-1);
    expect(treeCall?.signal).toBeInstanceOf(AbortSignal);
    expect(resourceCall?.signal).toBeInstanceOf(AbortSignal);
  });

  it("localizes the group visibility badge in both locales and keeps unknown values visible", async () => {
    installGroupBackend({
      children: [{
        group_id: 2,
        group_key: "beta",
        group_path: "beta",
        name: "Beta",
        visibility: "public",
        kind: "shared",
        current_role: "viewer",
        created_at: "2026-01-01T00:00:00Z",
        updated_at: "2026-01-01T00:00:00Z",
      }],
    });
    await renderOverview();

    const english = await screen.findByTestId("library-group-row");
    expect(english).toHaveTextContent("Public");
    expect(english).not.toHaveTextContent(/\bpublic\b/);

    setLocale("zh-CN");
    await waitFor(() => {
      const chinese = screen.getByTestId("library-group-row");
      expect(chinese).toHaveTextContent("公开");
      expect(chinese).not.toHaveTextContent("public");
    });
    setLocale("en");
  });

  it("opens the selected file preview with the file query and detail request", async () => {
    const backend = installGroupBackend();
    const user = userEvent.setup();
    const router = await renderOverview();
    await openReports(user);

    await user.click(screen.getByText("quarterly.md"));

    await waitFor(() => {
      expect(router.state.location.search).toContain("file=file-ready");
    });
    const dialog = await screen.findByRole("dialog");
    expect(await within(dialog).findByText("Preview body text")).toBeInTheDocument();
    expect(backend.callsFor("GET", "/v1/groups/by-path/{group_path}/library/files/{file_id}").length)
      .toBeGreaterThanOrEqual(1);
  });

  it("cancels a multi-selection delete without issuing a request", async () => {
    const backend = installGroupBackend();
    const user = userEvent.setup();
    await renderOverview();
    await openReports(user);

    const row = (await screen.findByText("broken.pdf")).closest("tr") as HTMLElement;
    await user.click(within(row).getByRole("checkbox"));
    await user.click(screen.getByTestId("library-delete-selected"));

    const dialog = await screen.findByRole("alertdialog");
    expect(dialog).toHaveTextContent("Delete 1 selected items?");
    await user.click(within(dialog).getByRole("button", { name: "Cancel" }));

    expect(backend.callsFor("DELETE", "/v1/groups/by-path/{group_path}/library/files/{file_id}")).toHaveLength(0);
  });

  it("deletes the selected files after confirmation", async () => {
    const backend = installGroupBackend();
    const originalDelete = backend.spies.DELETE.getMockImplementation() as
      (path: string, init?: unknown) => unknown;
    backend.spies.DELETE.mockImplementation(((path: string, init?: never) => {
      if (path !== "/v1/groups/by-path/{group_path}/library/files/{file_id}") {
        return originalDelete(path, init);
      }
      record(backend, "DELETE", path, init as never);
      return Promise.resolve(accepted({ task_id: "task-delete" }));
    }) as never);
    const user = userEvent.setup();
    await renderOverview();
    await openReports(user);

    const row = (await screen.findByText("broken.pdf")).closest("tr") as HTMLElement;
    await user.click(within(row).getByRole("checkbox"));
    await user.click(screen.getByTestId("library-delete-selected"));
    await user.click(within(await screen.findByRole("alertdialog")).getByRole("button", { name: "Delete" }));

    await waitFor(() => {
      const calls = backend.callsFor("DELETE", "/v1/groups/by-path/{group_path}/library/files/{file_id}");
      expect(calls).toHaveLength(1);
      expect(calls[0].params?.path).toEqual({ group_path: "stock/alpha", file_id: "file-failed" });
    });
  });

  it("commits the folder scope so returning to the previous folder starts at page one", async () => {
    const backend = installPagedBackend();
    const user = userEvent.setup();
    const router = await renderOverview();

    fireEvent.click(screen.getByRole("button", { name: "Next" }));
    await waitFor(() => expect(screen.getByText("Page 2 of 2")).toBeInTheDocument());
    expect(rootPages(backend)).toEqual([1, 2]);

    await act(async () => {
      await router.navigate("/groups/stock%2Falpha?folder=folder-reports");
    });
    await waitFor(() => expect(screen.getByText("Page 1 of 2")).toBeInTheDocument());
    expect(folderPages(backend)).toEqual([1]);

    const mark = backend.callsFor("GET", RESOURCES_PATH).length;
    await user.click(screen.getByTestId("library-go-up"));
    await waitFor(() => expect(screen.getByText("Page 1 of 2")).toBeInTheDocument());
    // The root is entered as a new effective scope, so it never resumes the
    // page two window it was left on, even though that page is cached.
    expect(readsSince(backend, mark).root).not.toContain(2);
    expect(router.state.location.search).toBe("");
  });

  it("starts at page one for breadcrumb, direct URL and back/forward revisits", async () => {
    const backend = installPagedBackend();
    const user = userEvent.setup();
    const router = await renderOverview();

    fireEvent.click(screen.getByRole("button", { name: "Next" }));
    await waitFor(() => expect(screen.getByText("Page 2 of 2")).toBeInTheDocument());

    let mark = backend.callsFor("GET", RESOURCES_PATH).length;
    await act(async () => {
      await router.navigate("/groups/stock%2Falpha?folder=folder-reports");
    });
    await waitFor(() => expect(screen.getByText("Page 1 of 2")).toBeInTheDocument());
    expect(readsSince(backend, mark).folder).toEqual([1]);

    // Back to the root: the cached page two window is not resumed.
    mark = backend.callsFor("GET", RESOURCES_PATH).length;
    await act(async () => {
      await router.navigate(-1);
    });
    await waitFor(() => expect(screen.getByText("Page 1 of 2")).toBeInTheDocument());
    expect(readsSince(backend, mark).root).not.toContain(2);

    // Forward again to the child: still page one.
    mark = backend.callsFor("GET", RESOURCES_PATH).length;
    await act(async () => {
      await router.navigate(1);
    });
    await waitFor(() => expect(screen.getByText("Page 1 of 2")).toBeInTheDocument());
    expect(readsSince(backend, mark).folder).not.toContain(2);

    fireEvent.click(screen.getByRole("button", { name: "Next" }));
    await waitFor(() => expect(screen.getByText("Page 2 of 2")).toBeInTheDocument());

    mark = backend.callsFor("GET", RESOURCES_PATH).length;
    const navigation = screen.getByRole("navigation", { name: "Navigation" });
    await user.click(within(navigation).getAllByRole("button")[0]);
    await waitFor(() => expect(screen.getByText("Page 1 of 2")).toBeInTheDocument());
    expect(readsSince(backend, mark).root).not.toContain(2);
  });

  it("keeps the page for a same-folder file selection and starts a new group at page one", async () => {
    const backend = installPagedBackend();
    const user = userEvent.setup();
    const router = await renderOverview();

    await openFolder(user, "report-0.md");
    await waitFor(() => expect(screen.getByText("Page 1 of 2")).toBeInTheDocument());
    fireEvent.click(screen.getByRole("button", { name: "Next" }));
    await waitFor(() => expect(screen.getByText("Page 2 of 2")).toBeInTheDocument());
    const folderReadsBeforeSelection = folderPages(backend).length;

    await user.click(screen.getByText("quarterly.md"));
    await waitFor(() => expect(router.state.location.search).toContain("file=file-ready"));
    expect(folderPages(backend)).toHaveLength(folderReadsBeforeSelection);
    expect(folderPages(backend).at(-1)).toBe(2);
    expect(screen.getByText("Page 2 of 2")).toBeInTheDocument();

    await act(async () => {
      await router.navigate("/groups/stock%2Fbeta");
    });
    await waitFor(() => {
      const betaPages = backend.callsFor("GET", RESOURCES_PATH)
        .filter((call) => call.params?.path?.group_path === "stock/beta")
        .map((call) => Number(call.params?.query?.page ?? 1));
      // A remount may read the first page more than once, but the new group
      // never inherits the page the previous group's window was left on.
      expect(betaPages.length).toBeGreaterThan(0);
      expect(new Set(betaPages)).toEqual(new Set([1]));
    });
  });

  it("refreshes only after the accepted deletion task settles", async () => {
    const backend = installGroupBackend();
    const originalGet = backend.spies.GET.getMockImplementation() as
      (path: string, init?: never) => Promise<unknown>;
    const originalDelete = backend.spies.DELETE.getMockImplementation() as
      (path: string, init?: unknown) => unknown;
    backend.spies.DELETE.mockImplementation(((path: string, init?: never) => {
      if (path !== "/v1/groups/by-path/{group_path}/library/files/{file_id}") {
        return originalDelete(path, init);
      }
      record(backend, "DELETE", path, init as never);
      return Promise.resolve(accepted({ task_id: "task-delete" }));
    }) as never);

    let releaseTask: ((value: unknown) => void) | null = null;
    backend.spies.GET.mockImplementation(((path: string, init?: never) => {
      if (path !== TASKS_PATH) {
        return originalGet(path, init);
      }
      record(backend, "GET", path, init as never);
      return new Promise((resolve) => {
        releaseTask = resolve;
      });
    }) as never);

    const user = userEvent.setup();
    await renderOverview({ toasts: true });
    await openReports(user);
    const row = (await screen.findByText("broken.pdf")).closest("tr") as HTMLElement;
    await user.click(within(row).getByRole("checkbox"));
    await user.click(screen.getByTestId("library-delete-selected"));
    await user.click(within(await screen.findByRole("alertdialog")).getByRole("button", { name: "Delete" }));

    await waitFor(() => expect(backend.callsFor("GET", TASKS_PATH)).toHaveLength(1));
    const resourcesBefore = backend.callsFor("GET", RESOURCES_PATH).length;
    expect(backend.callsFor("GET", RESOURCES_PATH)).toHaveLength(resourcesBefore);
    expect(screen.queryByText("Failed to delete file")).not.toBeInTheDocument();

    await act(async () => {
      releaseTask?.(accepted(taskResponse({ status: "succeeded" })));
    });

    await waitFor(() => expect(
      backend.callsFor("GET", RESOURCES_PATH).length,
    ).toBeGreaterThan(resourcesBefore));
    expect(screen.queryByText("Failed to delete file")).not.toBeInTheDocument();
  });

  it("reports an accepted deletion that failed after refreshing the window", async () => {
    const backend = installGroupBackend();
    const originalGet = backend.spies.GET.getMockImplementation() as
      (path: string, init?: never) => Promise<unknown>;
    const originalDelete = backend.spies.DELETE.getMockImplementation() as
      (path: string, init?: unknown) => unknown;
    backend.spies.DELETE.mockImplementation(((path: string, init?: never) => {
      if (path !== "/v1/groups/by-path/{group_path}/library/files/{file_id}") {
        return originalDelete(path, init);
      }
      record(backend, "DELETE", path, init as never);
      return Promise.resolve(accepted({ task_id: "task-delete" }));
    }) as never);
    backend.spies.GET.mockImplementation(((path: string, init?: never) => {
      if (path !== TASKS_PATH) {
        return originalGet(path, init);
      }
      record(backend, "GET", path, init as never);
      return Promise.resolve(accepted(taskResponse({ status: "failed", error_summary: "storage unavailable" })));
    }) as never);

    const user = userEvent.setup();
    await renderOverview({ toasts: true });
    await openReports(user);
    const resourcesBefore = backend.callsFor("GET", RESOURCES_PATH).length;
    const row = (await screen.findByText("broken.pdf")).closest("tr") as HTMLElement;
    await user.click(within(row).getByRole("checkbox"));
    await user.click(screen.getByTestId("library-delete-selected"));
    await user.click(within(await screen.findByRole("alertdialog")).getByRole("button", { name: "Delete" }));

    expect(await screen.findByText("storage unavailable")).toBeInTheDocument();
    expect(backend.callsFor("GET", RESOURCES_PATH).length).toBeGreaterThan(resourcesBefore);
  });

  it("settles the accepted part of a multi-delete when another dispatch fails", async () => {
    const backend = installGroupBackend();
    const originalDelete = backend.spies.DELETE.getMockImplementation() as
      (path: string, init?: unknown) => unknown;
    backend.spies.DELETE.mockImplementation(((path: string, init?: never) => {
      if (path !== "/v1/groups/by-path/{group_path}/library/files/{file_id}") {
        return originalDelete(path, init);
      }
      record(backend, "DELETE", path, init as never);
      const fileId = (init as { params?: { path?: { file_id?: string } } } | undefined)
        ?.params?.path?.file_id;
      return fileId === "file-ready"
        ? Promise.reject(new Error("network down"))
        : Promise.resolve(accepted({ task_id: "task-delete" }));
    }) as never);

    const user = userEvent.setup();
    await renderOverview({ toasts: true });
    await openReports(user);
    for (const name of ["quarterly.md", "broken.pdf"]) {
      const row = (await screen.findByText(name)).closest("tr") as HTMLElement;
      await user.click(within(row).getByRole("checkbox"));
    }
    await user.click(screen.getByTestId("library-delete-selected"));
    await user.click(within(await screen.findByRole("alertdialog")).getByRole("button", { name: "Delete" }));

    expect(await screen.findByText("1 items deleted.")).toBeInTheDocument();
    expect(await screen.findByText("Failed to delete file")).toBeInTheDocument();
    expect(backend.callsFor("DELETE", "/v1/groups/by-path/{group_path}/library/files/{file_id}")).toHaveLength(2);
    expect(backend.callsFor("GET", TASKS_PATH)).toHaveLength(1);
  });

  describe("task observation lifecycle", () => {
    const TASK_READ_PATH = "/v1/tasks/{task_id}";
    const SUBMIT_TASK_PATH = "/v1/tasks";

    function deferred<T>() {
      let resolve!: (value: T) => void;
      let reject!: (reason?: unknown) => void;
      const promise = new Promise<T>((res, rej) => {
        resolve = res;
        reject = rej;
      });
      return { promise, resolve, reject };
    }

    function taskReads(backend: Backend) {
      return backend.callsFor("GET", TASK_READ_PATH);
    }

    /** Serves every task read from `task` and records its abort signal. */
    function installTaskRead(
      backend: Backend,
      task: (path: string) => Promise<unknown>,
      taskId = "task-observation",
    ) {
      const originalGet = backend.spies.GET.getMockImplementation() as
        (path: string, init?: never) => Promise<unknown>;
      backend.spies.GET.mockImplementation(((path: string, init?: never) => {
        if (path !== TASK_READ_PATH) {
          return originalGet(path, init);
        }
        record(backend, "GET", path, init as never);
        return task(taskId);
      }) as never);
    }

    function installAdmission(backend: Backend, admission: () => Promise<unknown>) {
      const originalPost = backend.spies.POST.getMockImplementation() as
        (path: string, init?: never) => Promise<unknown>;
      backend.spies.POST.mockImplementation(((path: string, init?: never) => {
        if (path !== SUBMIT_TASK_PATH) {
          return originalPost(path, init);
        }
        record(backend, "POST", path, init as never);
        return admission();
      }) as never);
    }

    async function settleWait(ms = 1_800) {
      await act(async () => {
        await new Promise((resolve) => setTimeout(resolve, ms));
      });
    }

    function retryRow() {
      return (screen.getByText("broken.pdf").closest("tr") as HTMLElement);
    }

    it("stops polling a running retry when the workspace unmounts", async () => {
      const backend = installGroupBackend({ fileDetail: (fileId) => fileDetail(fileId) });
      const user = userEvent.setup();
      const successToast = vi.spyOn(toast, "success");
      const errorToast = vi.spyOn(toast, "error");
      const view = await renderOverviewView();
      await openReports(user);

      installAdmission(backend, () => Promise.resolve(accepted({ task_id: "task-observation" })));
      installTaskRead(backend, (taskId) => Promise.resolve(accepted(taskResponse({
        task_id: taskId,
        status: "running",
      }))));

      // Freeze wall-clock time only once the window is loaded, so the polling
      // cadence is observed exactly instead of waited out.
      vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout", "setInterval", "clearInterval", "Date"] });
      await act(async () => {
        fireEvent.click(within(retryRow()).getByRole("button", { name: "Retry" }));
        await vi.advanceTimersByTimeAsync(0);
      });
      await act(async () => {
        await vi.advanceTimersByTimeAsync(0);
      });

      expect(taskReads(backend)).toHaveLength(1);
      const first = taskReads(backend)[0].signal as AbortSignal;
      expect(first.aborted).toBe(false);

      // The running task keeps one poll per settlement interval while mounted.
      await act(async () => {
        await vi.advanceTimersByTimeAsync(1_500);
      });
      expect(taskReads(backend)).toHaveLength(2);
      const inFlight = taskReads(backend)[1].signal as AbortSignal;
      expect(inFlight.aborted).toBe(false);

      const feedbackBefore = successToast.mock.calls.length + errorToast.mock.calls.length;
      await act(async () => {
        view.unmount();
      });

      // The in-flight task read is aborted and nothing polls or reports later.
      expect(inFlight.aborted).toBe(true);
      await act(async () => {
        await vi.advanceTimersByTimeAsync(9_000);
      });
      expect(taskReads(backend)).toHaveLength(2);
      expect(successToast.mock.calls.length + errorToast.mock.calls.length).toBe(feedbackBefore);
    });

    it("never observes a task admitted after the workspace unmounted", async () => {
      const backend = installGroupBackend({ fileDetail: (fileId) => fileDetail(fileId) });
      const user = userEvent.setup();
      const view = await renderOverviewView();
      await openReports(user);

      const admission = deferred<unknown>();
      installAdmission(backend, () => admission.promise);
      installTaskRead(backend, (taskId) => Promise.resolve(accepted(taskResponse({
        task_id: taskId,
        status: "running",
      }))));

      const retry = within(retryRow()).getByRole("button", { name: "Retry" });
      await user.click(retry);
      await waitFor(() => expect(backend.callsFor("POST", SUBMIT_TASK_PATH)).toHaveLength(1));

      await act(async () => {
        view.unmount();
      });
      await act(async () => {
        admission.resolve(accepted({ task_id: "task-observation" }));
      });
      await settleWait();

      // The submitted server task is untouched, but its local observation never
      // starts: no task read, no poll, no feedback.
      expect(taskReads(backend)).toHaveLength(0);
      expect(backend.callsFor("POST", SUBMIT_TASK_PATH)).toHaveLength(1);
    });

    it("stops the previous group's observation and starts a new owner on re-entry", async () => {
      const backend = installGroupBackend({ fileDetail: (fileId) => fileDetail(fileId) });
      const user = userEvent.setup();
      const { router } = await renderOverviewView();
      await openReports(user);

      installAdmission(backend, () => Promise.resolve(accepted({ task_id: "task-alpha" })));
      let alphaReads = 0;
      const originalGet = backend.spies.GET.getMockImplementation() as
        (path: string, init?: never) => Promise<unknown>;
      backend.spies.GET.mockImplementation(((path: string, init?: never) => {
        if (path !== TASK_READ_PATH) {
          return originalGet(path, init);
        }
        record(backend, "GET", path, init as never);
        alphaReads += 1;
        return Promise.resolve(accepted(taskResponse({ task_id: "task-alpha", status: "running" })));
      }) as never);

      await user.click(within(retryRow()).getByRole("button", { name: "Retry" }));
      await waitFor(() => expect(taskReads(backend)).toHaveLength(1));
      const oldSignal = taskReads(backend)[0].signal as AbortSignal;

      await act(async () => {
        await router.navigate("/groups/stock%2Fbeta");
      });
      expect(oldSignal.aborted).toBe(true);
      const readsAfterSwitch = taskReads(backend).length;
      await settleWait();
      expect(taskReads(backend)).toHaveLength(readsAfterSwitch);
      expect(alphaReads).toBe(readsAfterSwitch);

      // Re-entering the group is a new owner: observation works again.
      await act(async () => {
        await router.navigate("/groups/stock%2Falpha?folder=folder-reports");
      });
      expect(await screen.findByText("broken.pdf")).toBeInTheDocument();

      backend.spies.GET.mockImplementation(((path: string, init?: never) => {
        if (path !== TASK_READ_PATH) {
          return originalGet(path, init);
        }
        record(backend, "GET", path, init as never);
        return Promise.resolve(accepted(taskResponse({ task_id: "task-alpha", status: "succeeded" })));
      }) as never);
      await user.click(within(retryRow()).getByRole("button", { name: "Retry" }));

      await waitFor(() => expect(taskReads(backend)).toHaveLength(readsAfterSwitch + 1));
      const freshSignal = taskReads(backend).at(-1)?.signal as AbortSignal;
      expect(freshSignal.aborted).toBe(false);
      expect(oldSignal.aborted).toBe(true);
      await settleWait(300);
      expect(taskReads(backend)).toHaveLength(readsAfterSwitch + 1);
    });

    it("stops observing a created text file after unmount", async () => {
      const backend = installGroupBackend();
      const user = userEvent.setup();
      const view = await renderOverviewView();

      const originalPut = backend.spies.PUT.getMockImplementation() as
        (path: string, init?: never) => Promise<unknown>;
      backend.spies.PUT.mockImplementation(((path: string, init?: never) => {
        if (path !== "/v1/groups/by-path/{group_path}/library/texts") {
          return originalPut(path, init);
        }
        record(backend, "PUT", path, init as never);
        return Promise.resolve(accepted({ task_id: "task-text" }));
      }) as never);
      installTaskRead(backend, (taskId) => Promise.resolve(accepted(taskResponse({
        task_id: taskId,
        status: "running",
      }))));

      await user.click(screen.getByTestId("library-create-menu"));
      await user.click(await screen.findByRole("menuitem", { name: "New Text File" }));
      await user.type(await screen.findByTestId("new-text-title"), "Notes");
      await user.type(screen.getByTestId("new-text-content"), "Body text");
      await user.click(screen.getByRole("button", { name: "Create" }));

      await waitFor(() => expect(taskReads(backend)).toHaveLength(1));
      const signal = taskReads(backend)[0].signal as AbortSignal;
      expect(signal.aborted).toBe(false);

      await act(async () => {
        view.unmount();
      });
      expect(signal.aborted).toBe(true);
      await settleWait();
      expect(taskReads(backend)).toHaveLength(1);
    });

    it("stops observing an uploaded file after unmount", async () => {
      vi.stubGlobal("crypto", webcrypto);
      const backend = installGroupBackend();
      const user = userEvent.setup();
      const view = await renderOverviewView();

      installTaskRead(backend, (taskId) => Promise.resolve(accepted(taskResponse({
        task_id: taskId,
        status: "running",
      }))), "task-upload");

      const file = new File(["upload-body"], "notes.txt", { type: "text/plain" });
      await user.upload(screen.getByTestId("library-upload-input"), file);
      await waitFor(() => expect(taskReads(backend)).toHaveLength(1));
      const signal = taskReads(backend)[0].signal as AbortSignal;

      await act(async () => {
        view.unmount();
      });
      expect(signal.aborted).toBe(true);
      await settleWait();
      expect(taskReads(backend)).toHaveLength(1);
    });

    it("never observes a retry-all task admitted after unmount", async () => {
      const backend = installGroupBackend({
        failedItems: [{
          kind: "file",
          id: "file-failed",
          group_key: "alpha",
          group_path: "stock/alpha",
          visibility: "private",
          parent_folder_id: "folder-reports",
          name: "broken.pdf",
          media_type: "application/pdf",
          size_bytes: 10,
          ingest_status: "failed",
          error_message: "parse failed",
          updated_at: "2026-01-02T00:00:00Z",
          created_at: "2026-01-01T00:00:00Z",
          child_folder_count: 0,
          file_count: 0,
          is_source_folder: false,
          is_source_records_folder: false,
          processing_count: 0,
        }],
        fileDetail: (fileId) => fileDetail(fileId),
      });
      const user = userEvent.setup();
      const view = await renderOverviewView();

      const admission = deferred<unknown>();
      installAdmission(backend, () => admission.promise);
      installTaskRead(backend, (taskId) => Promise.resolve(accepted(taskResponse({
        task_id: taskId,
        status: "running",
      }))));

      await waitFor(() => expect(screen.getByTestId("retry-all-failed")).toBeEnabled());
      await user.click(screen.getByTestId("retry-all-failed"));
      await user.click(within(await screen.findByRole("alertdialog")).getByRole("button", { name: "Retry all" }));
      await waitFor(() => expect(backend.callsFor("POST", SUBMIT_TASK_PATH)).toHaveLength(1));

      await act(async () => {
        view.unmount();
      });
      await act(async () => {
        admission.resolve(accepted({ task_id: "task-retry-all" }));
      });
      await settleWait();

      expect(taskReads(backend)).toHaveLength(0);
    });

    it("stops observing a source sync after unmount", async () => {
      const backend = installGroupBackend();
      const originalGet = backend.spies.GET.getMockImplementation() as
        (path: string, init?: never) => Promise<unknown>;
      backend.spies.GET.mockImplementation(((path: string, init?: never) => {
        const request = init as { params?: { query?: Record<string, unknown> } } | undefined;
        if (path === RESOURCES_PATH && !request?.params?.query?.folder_id) {
          record(backend, "GET", path, request as never);
          const items: LibraryResourceItem[] = [
            {
              ...folderResourceItem(5),
              id: "folder-sources",
              name: "Sources",
              file_count: 1,
              is_source_folder: true,
            },
          ];
          return Promise.resolve(accepted({ items, pagination: pagination(1, items.length) }));
        }
        return originalGet(path, init);
      }) as never);

      const user = userEvent.setup();
      const view = await renderOverviewView();

      const originalPost = backend.spies.POST.getMockImplementation() as
        (path: string, init?: never) => Promise<unknown>;
      backend.spies.POST.mockImplementation(((path: string, init?: never) => {
        if (path !== "/v1/groups/by-path/{group_path}/source-folders/{folder_id}/sync") {
          return originalPost(path, init);
        }
        record(backend, "POST", path, init as never);
        return Promise.resolve(accepted({ task_id: "task-sync" }));
      }) as never);
      installTaskRead(backend, (taskId) => Promise.resolve(accepted(taskResponse({
        task_id: taskId,
        status: "running",
      }))), "task-sync");

      const row = (screen.getAllByText("Sources")
        .find((element) => element.closest("tr")) as HTMLElement).closest("tr") as HTMLElement;
      await user.click(within(row).getByRole("button", { name: "Actions" }));
      await user.click(await screen.findByRole("menuitem", { name: "Sync" }));
      await waitFor(() => expect(taskReads(backend)).toHaveLength(1));
      const signal = taskReads(backend)[0].signal as AbortSignal;

      await act(async () => {
        view.unmount();
      });
      expect(signal.aborted).toBe(true);
      await settleWait();
      expect(taskReads(backend)).toHaveLength(1);
    });
  });

  it("hashes an upload, prepares it once per file and sends one multipart request", async () => {
    vi.stubGlobal("crypto", webcrypto);
    const backend = installGroupBackend();
    const user = userEvent.setup();
    await renderOverview();

    const file = new File(["upload-body"], "notes.txt", { type: "text/plain" });
    await user.upload(screen.getByTestId("library-upload-input"), file);

    await waitFor(() => {
      const prepares = backend.callsFor("POST", "/v1/groups/by-path/{group_path}/library/files/prepare-upload");
      expect(prepares).toHaveLength(1);
      expect(prepares[0].body).toMatchObject({
        folder_id: null,
        filename: "notes.txt",
        media_type: "text/plain",
        size_bytes: 11,
        options: { source_policy: "retain" },
      });
      expect((prepares[0].body as { sha256: string }).sha256).toMatch(/^[a-f0-9]{64}$/);
    });
    await waitFor(() => expect(backend.uploadRequests).toHaveLength(1));
    const form = backend.uploadRequests[0].form;
    const digest = await webcrypto.subtle.digest("SHA-256", new TextEncoder().encode("upload-body"));
    const expectedSha = Array.from(new Uint8Array(digest), (byte) => byte.toString(16).padStart(2, "0")).join("");
    expect(form.getAll("sha256")).toEqual([expectedSha]);
    expect((form.getAll("files") as File[]).map((entry) => entry.name)).toEqual(["notes.txt"]);
    expect(JSON.parse(String(form.get("metadata"))).source_policy).toBe("retain");
  });

  it("skips failed retries whose original source is unavailable and submits the rest", async () => {
    const failedItems = [
      {
        kind: "file" as const,
        id: "file-failed",
        group_key: "alpha",
        group_path: "stock/alpha",
        visibility: "private" as const,
        parent_folder_id: "folder-reports",
        name: "broken.pdf",
        media_type: "application/pdf",
        size_bytes: 10,
        ingest_status: "failed" as const,
        error_message: "parse failed",
        updated_at: "2026-01-02T00:00:00Z",
        created_at: "2026-01-01T00:00:00Z",
        child_folder_count: 0,
        file_count: 0,
        is_source_folder: false,
        is_source_records_folder: false,
        processing_count: 0,
      },
      {
        kind: "file" as const,
        id: "file-unavailable",
        group_key: "alpha",
        group_path: "stock/alpha",
        visibility: "private" as const,
        parent_folder_id: "folder-reports",
        name: "lost.pdf",
        media_type: "application/pdf",
        size_bytes: 10,
        ingest_status: "failed" as const,
        error_message: "parse failed",
        updated_at: "2026-01-02T00:00:00Z",
        created_at: "2026-01-01T00:00:00Z",
        child_folder_count: 0,
        file_count: 0,
        is_source_folder: false,
        is_source_records_folder: false,
        processing_count: 0,
      },
    ];
    const backend = installGroupBackend({
      failedItems,
      fileDetail: (fileId) => fileDetail(fileId),
    });
    const user = userEvent.setup();
    await renderOverview();

    await screen.findByTestId("retry-all-failed");
    await waitFor(() => expect(screen.getByTestId("retry-all-failed")).toBeEnabled());
    await user.click(screen.getByTestId("retry-all-failed"));
    await user.click(within(await screen.findByRole("alertdialog")).getByRole("button", { name: "Retry all" }));

    await waitFor(() => {
      const calls = backend.callsFor("POST", "/v1/tasks");
      expect(calls).toHaveLength(1);
      expect(calls[0].body).toEqual({
        kind: "retry_file_batch",
        group_path: "stock/alpha",
        items: [{ file_id: "file-failed" }],
      });
    });
    expect(backend.callsFor("GET", "/v1/groups/by-path/{group_path}/library/files/{file_id}").length)
      .toBeGreaterThanOrEqual(2);
  });

  it("sends the resource status filter and sort to the scoped resource window", async () => {
    const backend = installGroupBackend();
    const user = userEvent.setup();
    await renderOverview();
    await openReports(user);

    await user.click(screen.getByTestId("library-status-filter"));
    await user.click(await screen.findByRole("option", { name: "Failed" }));

    await waitFor(() => {
      const calls = backend.callsFor("GET", "/v1/groups/by-path/{group_path}/library/resources");
      expect(calls.at(-1)?.params?.query).toMatchObject({ status: "failed", page: 1 });
    });

    await user.click(screen.getByRole("button", { name: "File" }));

    await waitFor(() => {
      const calls = backend.callsFor("GET", "/v1/groups/by-path/{group_path}/library/resources");
      expect(calls.at(-1)?.params?.query).toMatchObject({ sort_by: "name", sort_direction: "asc" });
    });
  });

  it("clears the selected file when navigating back to the group root", async () => {
    installGroupBackend();
    const user = userEvent.setup();
    const router = await renderOverview();
    await openReports(user);

    await user.click(screen.getByText("quarterly.md"));
    await waitFor(() => expect(router.state.location.search).toContain("file=file-ready"));
    await user.keyboard("{Escape}");
    await waitFor(() => expect(screen.queryByTestId("library-preview")).not.toBeInTheDocument());

    await user.click(screen.getByTestId("library-go-up"));

    await waitFor(() => {
      expect(router.state.location.search).not.toContain("file=");
      expect(router.state.location.search).not.toContain("folder=");
    });
  });

  it("creates a source folder from the form-backed configuration dialog", async () => {
    const backend = installGroupBackend();
    const user = userEvent.setup();
    await renderOverview();

    await user.click(screen.getByTestId("library-create-menu"));
    await user.click(await screen.findByRole("menuitem", { name: "New Source Folder" }));
    const dialog = await screen.findByRole("dialog");
    await user.type(within(dialog).getByTestId("source-folder-name"), "Sources");
    await user.type(within(dialog).getByLabelText("Source Key"), "gov_documents");
    await user.click(within(dialog).getByRole("button", { name: "Save" }));

    await waitFor(() => {
      const calls = backend.callsFor("POST", "/v1/groups/by-path/{group_path}/source-folders");
      expect(calls).toHaveLength(1);
      expect(calls[0].body).toMatchObject({
        parent_folder_id: null,
        folder_name: "Sources",
        source_config: {
          source_key: "gov_documents",
          connector_type: "postgres_sql",
          sync_strategy: "cursor",
        },
      });
    });
  });

  it("scopes content search to the selected folder and only shows matching paths", async () => {
    const backend = installGroupBackend({
      searchHits: [
        libraryHit({ chunk_id: "chunk-in", library_path: "Alpha/Reports" }),
        libraryHit({ chunk_id: "chunk-out", title: "Elsewhere", library_path: "Alpha/Other" }),
      ],
    });
    const user = userEvent.setup();
    await renderOverview();
    await openReports(user);

    await user.type(screen.getByTestId("scoped-content-search-input"), "policy{Enter}");

    await waitFor(() => {
      const calls = backend.callsFor("POST", "/v1/search");
      expect(calls).toHaveLength(1);
      expect(calls[0].body).toMatchObject({ query: "policy", group_path: "stock/alpha" });
    });

    const results = await screen.findByTestId("scoped-search-results");
    expect(within(results).getByText("Quarterly")).toBeInTheDocument();
    expect(within(results).queryByText("Elsewhere")).not.toBeInTheDocument();
  });
});
