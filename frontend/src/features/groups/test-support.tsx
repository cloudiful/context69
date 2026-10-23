import { waitFor } from "@testing-library/react";
import { expect, vi } from "vitest";

import { restoreSession } from "@/lib/auth-store";
import { openapiClient } from "@/services/openapi-client";
import type {
  GroupMemberResponse,
  GroupPageResponse,
  GroupResponse,
  LibraryFileDetailResponse,
  LibraryFileSummary,
  LibraryFolderNode,
  LibraryResourceItem,
  LibraryTreeResponse,
  MetadataIndexPageResponse,
  SearchHit,
  TaskRef,
  TaskResponse,
} from "@/services/api";

export const TEST_USER = {
  user_id: 1,
  login_name: "owner",
  display_name: "Owner",
  is_admin: false,
  personal_group_path: "personal-owner",
  personal_group_role: "owner" as const,
};

export const GROUP: GroupResponse = {
  group_id: 1,
  group_key: "alpha",
  group_path: "stock/alpha",
  name: "Alpha",
  visibility: "private",
  kind: "shared",
  current_role: "owner",
  created_at: "2026-01-01T00:00:00Z",
  updated_at: "2026-01-01T00:00:00Z",
};

const FILE_READY: LibraryFileSummary = {
  file_id: "file-ready",
  group_key: "alpha",
  group_path: "stock/alpha",
  visibility: "private",
  folder_id: "folder-reports",
  filename: "quarterly.md",
  media_type: "text/markdown",
  size_bytes: 120,
  ingest_status: "succeeded",
  error_message: null,
  created_at: "2026-01-01T00:00:00Z",
  updated_at: "2026-01-02T00:00:00Z",
  ingested_at: "2026-01-02T00:00:00Z",
};

const FILE_FAILED: LibraryFileSummary = {
  ...FILE_READY,
  file_id: "file-failed",
  filename: "broken.pdf",
  media_type: "application/pdf",
  ingest_status: "failed",
  error_message: "parse failed",
};

const FILE_FAILED_UNAVAILABLE: LibraryFileSummary = {
  ...FILE_FAILED,
  file_id: "file-unavailable",
  filename: "lost.pdf",
};

const REPORTS_FOLDER: LibraryFolderNode = {
  group_key: "alpha",
  group_path: "stock/alpha",
  visibility: "private",
  folder_id: "folder-reports",
  parent_folder_id: null,
  name: "Reports",
  path: "Alpha/Reports",
  processing_count: 0,
  children: [],
  files: [FILE_READY, FILE_FAILED, FILE_FAILED_UNAVAILABLE],
};

const ROOT: LibraryFolderNode = {
  group_key: "alpha",
  group_path: "stock/alpha",
  visibility: "private",
  folder_id: null,
  parent_folder_id: null,
  name: "Alpha",
  path: "Alpha",
  processing_count: 0,
  children: [REPORTS_FOLDER],
  files: [],
};

export const TREE: LibraryTreeResponse = { root: ROOT };

function folderResource(folder: LibraryFolderNode): LibraryResourceItem {
  return {
    kind: "folder",
    id: folder.folder_id as string,
    group_key: folder.group_key,
    group_path: folder.group_path,
    visibility: folder.visibility,
    parent_folder_id: folder.parent_folder_id ?? null,
    name: folder.name,
    updated_at: "2026-01-02T00:00:00Z",
    created_at: "2026-01-01T00:00:00Z",
    child_folder_count: folder.children.length,
    file_count: folder.files.length,
    is_source_folder: false,
    is_source_records_folder: false,
    processing_count: 0,
  };
}

function fileResource(file: LibraryFileSummary, parent: LibraryFolderNode): LibraryResourceItem {
  return {
    kind: "file",
    id: file.file_id,
    group_key: file.group_key,
    group_path: file.group_path,
    visibility: file.visibility,
    parent_folder_id: parent.folder_id ?? null,
    name: file.filename,
    media_type: file.media_type,
    size_bytes: file.size_bytes,
    ingest_status: file.ingest_status,
    error_message: file.error_message,
    updated_at: file.updated_at,
    created_at: file.created_at,
    child_folder_count: 0,
    file_count: 0,
    is_source_folder: false,
    is_source_records_folder: false,
    processing_count: 0,
  };
}

export const MEMBERS: GroupMemberResponse[] = [
  { user_id: 1, login_name: "owner", display_name: "Owner", role: "owner" },
  { user_id: 2, login_name: "viewer", display_name: "Viewer", role: "viewer" },
];

export function pagination(page: number, total: number, pageSize = 50) {
  return {
    page,
    page_size: pageSize,
    total,
    total_pages: Math.max(1, Math.ceil(total / pageSize)),
  };
}

export interface RecordedCall {
  method: string;
  path: string;
  params: { path?: Record<string, string>; query?: Record<string, unknown> } | undefined;
  body: unknown;
  signal?: AbortSignal;
}

export interface GroupBackendOptions {
  group?: GroupResponse;
  groups?: GroupResponse[];
  children?: GroupResponse[];
  members?: GroupMemberResponse[];
  tree?: LibraryTreeResponse;
  fileDetail?: (fileId: string) => LibraryFileDetailResponse;
  failedItems?: LibraryResourceItem[];
  searchHits?: SearchHit[];
  metadata?: MetadataIndexPageResponse;
  translation?: Record<string, unknown>;
}

function jsonResponse(data: unknown, status = 200) {
  return new Response(JSON.stringify(data), {
    status,
    headers: { "Content-Type": "application/json" },
  });
}

function ok<T>(data: T) {
  return Promise.resolve({ data, response: { ok: true, status: 200 } });
}

function fail(status: number, message: string) {
  return Promise.resolve({
    error: { message },
    response: { ok: false, status },
  });
}

export function installGroupBackend(options: GroupBackendOptions = {}) {
  const calls: RecordedCall[] = [];
  const group = options.group ?? GROUP;
  let tree: LibraryTreeResponse = options.tree ?? TREE;
  let members = [...(options.members ?? MEMBERS)];
  const uploadRequests: Array<{ url: string; form: FormData }> = [];

  vi.stubGlobal("fetch", vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = String(input);
    if (url.includes("/v1/auth/me")) {
      return jsonResponse({ user: TEST_USER });
    }
    if (url.includes("/library/files/upload")) {
      uploadRequests.push({ url, form: init?.body as FormData });
      return jsonResponse({ task_id: "task-upload" });
    }
    return jsonResponse({ message: "unmocked fetch" }, 404);
  }));

  function resourceItems(folderId: string | null): LibraryResourceItem[] {
    if (folderId === "folder-reports") {
      return REPORTS_FOLDER.files.map((file) => fileResource(file, REPORTS_FOLDER));
    }
    return ROOT.children.map(folderResource);
  }

  function dispatch(method: string, path: string, init?: {
    body?: unknown;
    params?: RecordedCall["params"];
    signal?: AbortSignal;
  }) {
    calls.push({ method, path, params: init?.params, body: init?.body, signal: init?.signal });
    const params = init?.params?.path ?? {};
    const query = (init?.params?.query ?? {}) as Record<string, unknown>;
    const groupPath = params.group_path;

    if (path === "/v1/groups" && method === "GET") {
      return ok({ items: options.groups ?? [group], pagination: pagination(1, 1) });
    }
    if (path === "/v1/groups" && method === "POST") {
      return ok({ ...group, group_id: 99, ...(init?.body as object) });
    }
    if (path === "/v1/groups/search") {
      return ok(options.groups ?? [group]);
    }
    if (path === "/v1/user-directory") {
      return ok([{ user_id: 2, login_name: "viewer", display_name: "Viewer" }]);
    }
    if (path === "/v1/groups/by-path/{group_path}" && method === "GET") {
      return ok({ ...group, group_path: groupPath ?? group.group_path });
    }
    if (path === "/v1/groups/by-path/{group_path}" && method === "PATCH") {
      return ok({ ...group, ...(init?.body as object) });
    }
    if (path === "/v1/groups/by-path/{group_path}" && method === "DELETE") {
      return ok(undefined);
    }
    if (path === "/v1/groups/by-path/{group_path}/move") {
      return ok({ ...group, parent_group_path: null });
    }
    if (path === "/v1/groups/by-path/{group_path}/children") {
      const items = options.children ?? [];
      return ok({ items, pagination: pagination(1, items.length) } satisfies GroupPageResponse);
    }
    if (path === "/v1/groups/by-path/{group_path}/members" && method === "GET") {
      return ok({ items: members, pagination: pagination(1, members.length) });
    }
    if (path === "/v1/groups/by-path/{group_path}/members" && method === "POST") {
      const payload = init?.body as GroupMemberResponse;
      members = [...members.filter((member) => member.login_name !== payload.login_name), {
        user_id: 99,
        login_name: payload.login_name,
        display_name: payload.login_name,
        role: payload.role,
      }];
      return ok(payload);
    }
    if (path === "/v1/groups/by-path/{group_path}/members/{login_name}" && method === "DELETE") {
      members = members.filter((member) => member.login_name !== params.login_name);
      return ok(undefined);
    }
    if (path === "/v1/groups/by-path/{group_path}/library/tree") {
      return ok(tree);
    }
    if (path === "/v1/groups/by-path/{group_path}/library/resources") {
      if (query.status === "failed" && query.recursive === true) {
        const failed = options.failedItems ?? [];
        return ok({
          items: query.page_size === 1 ? failed.slice(0, 1) : failed,
          pagination: pagination(1, failed.length, Number(query.page_size ?? 50)),
        });
      }
      const items = resourceItems((query.folder_id as string | null | undefined) ?? null);
      return ok({ items, pagination: pagination(1, items.length) });
    }
    if (path === "/v1/groups/by-path/{group_path}/library/files/{file_id}" && method === "GET") {
      const fileId = params.file_id ?? "";
      if (options.fileDetail) {
        return ok(options.fileDetail(fileId));
      }
      return ok(fileDetail(fileId));
    }
    if (path === "/v1/groups/by-path/{group_path}/library/files/{file_id}" && method === "DELETE") {
      return ok(undefined);
    }
    if (path === "/v1/groups/by-path/{group_path}/library/files/prepare-upload") {
      return ok({ upload_required: true, file: null, task: null });
    }
    if (path === "/v1/groups/by-path/{group_path}/library/folders" && method === "POST") {
      return ok({ ...REPORTS_FOLDER, folder_id: "folder-new", name: (init?.body as { name: string }).name });
    }
    if (path === "/v1/groups/by-path/{group_path}/library/folders/{folder_id}" && method === "DELETE") {
      tree = { root: ROOT };
      return ok(undefined);
    }
    if (path === "/v1/tasks" && method === "POST") {
      return ok({ task_id: "task-batch" } satisfies TaskRef);
    }
    if (path === "/v1/tasks/{task_id}") {
      return ok({
        task_id: params.task_id ?? "task",
        kind: "file_batch",
        origin: "manual",
        status: "succeeded",
        stage: "done",
        waiting_reason: null,
        created_at: "2026-01-01T00:00:00Z",
        updated_at: "2026-01-01T00:00:00Z",
        started_at: "2026-01-01T00:00:00Z",
        finished_at: "2026-01-01T00:00:01Z",
        progress: { total: 1, queued: 0, running: 0, succeeded: 1, failed: 0, cancelled: 0, waiting: 0 },
      } satisfies TaskResponse);
    }
    if (path === "/v1/search" && method === "POST") {
      return ok({ query: String((init?.body as { query?: string })?.query ?? ""), items: options.searchHits ?? [], pagination: pagination(1, options.searchHits?.length ?? 0) });
    }
    if (path === "/v1/groups/by-path/{group_path}/metadata-indexes" && method === "GET") {
      return ok(options.metadata ?? { items: [], pagination: pagination(1, 0) });
    }
    if (path === "/v1/groups/by-path/{group_path}/metadata-indexes" && method === "POST") {
      return ok({ index_id: "index-1", status: "building", ...(init?.body as object) });
    }
    if (path === "/v1/groups/by-path/{group_path}/metadata-indexes/{index_id}/retry") {
      return ok(undefined);
    }
    if (path === "/v1/groups/by-path/{group_path}/metadata-indexes/{index_id}" && method === "DELETE") {
      return ok(undefined);
    }
    if (path === "/v1/groups/by-path/{group_path}/translation-settings" && method === "GET") {
      return ok(options.translation ?? {
        enabled: true,
        default_target_locales: ["zh-CN"],
        source_locale: "en-US",
        glossary: [],
        queued_count: 0,
        running_count: 0,
        succeeded_count: 3,
        failed_count: 0,
      });
    }
    if (path === "/v1/groups/by-path/{group_path}/translation-settings" && method === "PUT") {
      return ok(init?.body);
    }
    return fail(404, `unmocked ${method} ${path}`);
  }

  const spies = {
    GET: vi.spyOn(openapiClient, "GET"),
    POST: vi.spyOn(openapiClient, "POST"),
    PUT: vi.spyOn(openapiClient, "PUT"),
    PATCH: vi.spyOn(openapiClient, "PATCH"),
    DELETE: vi.spyOn(openapiClient, "DELETE"),
  };

  spies.GET.mockImplementation(((path: string, init?: never) => dispatch("GET", path, init)) as never);
  spies.POST.mockImplementation(((path: string, init?: never) => dispatch("POST", path, init)) as never);
  spies.PUT.mockImplementation(((path: string, init?: never) => dispatch("PUT", path, init)) as never);
  spies.PATCH.mockImplementation(((path: string, init?: never) => dispatch("PATCH", path, init)) as never);
  spies.DELETE.mockImplementation(((path: string, init?: never) => dispatch("DELETE", path, init)) as never);

  return {
    calls,
    uploadRequests,
    callsFor(method: string, path: string) {
      return calls.filter((call) => call.method === method && call.path === path);
    },
    spies,
  };
}

export function fileDetail(fileId: string): LibraryFileDetailResponse {
  const failed = fileId === "file-failed";
  const unavailable = fileId === "file-unavailable";
  return {
    file_id: fileId,
    group_key: "alpha",
    group_path: "stock/alpha",
    visibility: "private",
    folder_id: "folder-reports",
    folder_path: "Reports",
    filename: failed ? "broken.pdf" : unavailable ? "lost.pdf" : "quarterly.md",
    media_type: "text/markdown",
    size_bytes: 120,
    sha256: "a".repeat(64),
    ingest_status: failed ? "failed" : "succeeded",
    error_message: failed ? "parse failed" : null,
    created_at: "2026-01-01T00:00:00Z",
    updated_at: "2026-01-02T00:00:00Z",
    ingested_at: "2026-01-02T00:00:00Z",
    source_available: !unavailable,
    sections: [{
      document_id: 1,
      section_key: "body",
      section_label: "Body",
      sort_order: 0,
      title: "Body",
      preview_text: "Preview body text",
      content_format: "markdown",
    }],
  };
}

export async function authenticate() {
  await restoreSession();
}

export async function waitForGroupShell() {
  await waitFor(() => {
    expect(document.querySelector("[data-testid='group-shell']")).not.toBeNull();
  });
}
