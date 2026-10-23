import type { Page, Route } from "@playwright/test";

import {
  ACCESS_TOKEN,
  ADMIN_USER,
  ADMIN_USER_ROW,
  AUTH_ERROR,
  DOCLING_SETTINGS,
  GROUP,
  PREVIEW_DOCUMENT,
  REMOTE_TASKS,
  SOURCE_CONFIG_FILE,
  SOURCE_CONFIG_TEXT,
  WORKSPACE_CHILD_GROUP,
  WORKSPACE_FILES,
  WORKSPACE_GROUP,
  WORKSPACE_MEMBERS,
  WORKSPACE_METADATA,
  WORKSPACE_TRANSLATION,
  WORKSPACE_TREE,
  workspaceFileDetail,
  RUNTIME_SETTINGS,
  QUEUE_ACTIVE_TASK,
  QUEUE_COMPLETED_TASK,
  QUEUE_DIAGNOSE,
  QUEUE_FAILED_TASK,
  QUEUE_ITEMS,
  QUEUE_TRASHED_TASK,
  SEARCH_SETTINGS,
  SOURCE_STATUS,
  TRANSLATION_PROVIDERS,
  TRANSLATION_SETTINGS,
  pagination,
  type AuthUserFixture,
  type DocumentFixture,
  type GroupFixture,
  type GroupMemberFixture,
  type LibraryFileDetailFixture,
  type LibraryResourceFixture,
  type LibraryTreeFixture,
  type MetadataIndexFixture,
  type SearchResponseFixture,
  type SourceStatusFixture,
  type TaskDiagnoseFixture,
  type TaskFixture,
  type TaskItemsFixture,
} from "./fixtures";

/** Origin of the preview app; cross-origin requests are refused, not mocked. */
const ALLOWED_ORIGIN = "http://127.0.0.1:5199";

export interface AppTransportOptions {
  user?: AuthUserFixture;
  /** Other sign-in identities keyed by login name, so an account switch is reachable. */
  accounts?: AuthUserFixture[];
  /** Starts without a cookie session, so the login page is reachable. */
  guest?: boolean;
  loginRejects?: boolean;
  meStatus?: number;
  /** Enables the settings endpoints the settings suite drives. */
  settings?: boolean;
  embeddingTestFails?: boolean;
  /** Fails one settings group so a partial save is reachable. */
  settingsFailures?: { doclingPut?: BackendFailure };
  /** Enables the search, document preview and stream endpoints. */
  search?: SearchMock;
  /** Enables group workspace, library, member, metadata and upload endpoints. */
  workspace?: WorkspaceMock;
  /** Enables generated document reader endpoints, including their error states. */
  documents?: DocumentsMock;
  /** Enables the processing queue list, items, diagnose, actions and SSE. */
  queue?: QueueMock;
}

export interface QueueMock {
  /** Rows of the processing view (everything not succeeded and not trashed). */
  processing?: TaskFixture[];
  completed?: TaskFixture[];
  trash?: TaskFixture[];
  items?: Record<string, TaskItemsFixture>;
  /** Item pages served in order per task id; the last page repeats. */
  itemSequences?: Record<string, TaskItemsFixture[]>;
  /** Processing-window task pages served in order; the last page repeats. */
  listSequence?: TaskFixture[][];
  /**
   * How many processing-window reads answer normally before the mock stops
   * answering at all, so the client-side deadline is the only way out.
   * `undefined` never hangs.
   */
  hangListAfter?: number;
  /** A delayed watch-all snapshot, so a parent transition can follow expansion. */
  streamUpdates?: {
    delayMs?: number;
    tasks: TaskFixture[];
    /** Progress frames written after the snapshot in the same stream. */
    updates?: TaskFixture[];
  };
  diagnose?: Record<string, TaskDiagnoseFixture>;
  /** `snapshot` sends one watch-all frame; `error` fails the transport. */
  stream?: "snapshot" | "error";
  listError?: BackendFailure;
}

export interface DocumentsMock {
  /** Document payloads keyed by the numeric route id. */
  documents?: Record<string, DocumentFixture>;
  /** Failure states keyed by the numeric route id. */
  failures?: Record<string, BackendFailure>;
  /** Login name that may read a document id; any other identity gets `403`. */
  owners?: Record<string, string>;
  fallback?: DocumentFixture;
}

export interface LibraryDeletionMock {
  taskId?: string;
  /** Statuses served by the task read, one per poll; the last status repeats. */
  states?: TaskFixture["status"][];
  /** Fails the delete dispatch with this status instead of accepting it. */
  failure?: BackendFailure;
}

export interface WorkspaceMock {
  group?: GroupFixture;
  children?: GroupFixture[];
  members?: GroupMemberFixture[];
  tree?: LibraryTreeFixture;
  /** Folders prepended to the tree so the resource window has a second page. */
  extraRootFolders?: number;
  /** Task contract for library folder and file deletes. */
  deletion?: LibraryDeletionMock;
  /**
   * Task id for library retry submissions whose reads stay `running`, so a
   * started observation is observable until the workspace leaves.
   */
  runningTaskId?: string;
  /** Delays the library submission answer, so a late admission is reachable. */
  taskAdmissionDelayMs?: number;
  fileDetail?: LibraryFileDetailFixture;
  metadata?: MetadataIndexFixture[];
  translation?: typeof WORKSPACE_TRANSLATION;
  /** File names whose prepare step reports an existing upload. */
  reuseFiles?: string[];
  /** Fails the scoped resource window so the error state is reachable. */
  resourcesError?: BackendFailure;
}

export interface BackendFailure {
  status: number;
  message: string;
}

export interface SearchMock {
  stream?: "success" | "http-error" | "error-frame" | "malformed";
  local?: SearchResponseFixture;
  reranked?: SearchResponseFixture;
  post?: SearchResponseFixture | SearchResponseFixture[];
  postError?: BackendFailure;
  document?: DocumentFixture;
  documentError?: BackendFailure;
  sources?: SourceStatusFixture[];
  groups?: GroupFixture[];
}

export interface RecordedRequest {
  method: string;
  path: string;
  body: unknown;
  /** Full query string, including the leading `?`. */
  query: string;
  /** Raw multipart payload text, so form parts can be counted. */
  rawBody?: string;
}

export interface AppTransport {
  calls: RecordedRequest[];
  unmocked: string[];
}

function json(status: number, body: unknown) {
  return {
    status,
    contentType: "application/json",
    headers: { "Cache-Control": "no-store" },
    body: JSON.stringify(body),
  };
}

/**
 * Origin-restricted API mocks typed against the generated contract, so the
 * browser suite exercises the real client without a backend. The mock tracks
 * the cookie session across login, session restore and logout, keeps the saved
 * settings so a save can be reloaded, and records unmocked API paths.
 */
export async function installAppTransport(
  page: Page,
  options: AppTransportOptions = {},
): Promise<AppTransport> {
  let currentUser: AuthUserFixture = options.user ?? ADMIN_USER;
  const calls: RecordedRequest[] = [];
  const unmocked: string[] = [];
  const saved = {
    runtime: structuredClone(RUNTIME_SETTINGS),
    docling: structuredClone(DOCLING_SETTINGS),
    search: structuredClone(SEARCH_SETTINGS),
    translation: structuredClone(TRANSLATION_SETTINGS),
    tokens: [structuredClone(ACCESS_TOKEN)],
    users: [structuredClone(ADMIN_USER_ROW)],
  };
  let signedIn = !options.guest;
  const itemRequests: Record<string, number> = {};
  let listRequests = 0;
  let deletionPolls = 0;
  const searchPosts: SearchResponseFixture[] = options.search
    ? Array.isArray(options.search.post)
      ? [...options.search.post]
      : options.search.post
        ? [options.search.post]
        : []
    : [];

  function sseFrame(event: string, data: unknown): string {
    return `event: ${event}\ndata: ${JSON.stringify(data)}\n\n`;
  }

  async function handleSearch(route: Route, url: URL, method: string): Promise<boolean> {
    const search = options.search;
    if (!search) {
      return false;
    }

    if (url.pathname === "/v1/sources") {
      await route.fulfill(json(200, {
        items: search.sources ?? [SOURCE_STATUS],
        pagination: pagination(1, 100, 1),
      }));
      return true;
    }

    if (url.pathname === "/v1/groups") {
      await route.fulfill(json(200, {
        items: search.groups ?? [GROUP],
        pagination: pagination(1, 100, 1),
      }));
      return true;
    }

    if (url.pathname.startsWith("/v1/documents/")) {
      if (search.documentError) {
        await route.fulfill(json(search.documentError.status, {
          code: "internal",
          message: search.documentError.message,
        }));
        return true;
      }
      await route.fulfill(json(200, search.document ?? PREVIEW_DOCUMENT));
      return true;
    }

    if (url.pathname === "/v1/search/stream" && method === "GET") {
      const mode = search.stream ?? "success";
      if (mode === "http-error") {
        await route.fulfill(json(500, { code: "internal", message: "stream unavailable" }));
        return true;
      }

      let body = "";
      if (mode === "malformed") {
        body = "event: local\ndata: not-json\n\n";
      } else {
        const local = search.local ?? {
          query: url.searchParams.get("query") ?? "",
          items: [],
          pagination: pagination(1, 8, 0),
        };
        body += sseFrame("local", { items: local.items, pagination: local.pagination });
        if (search.reranked) {
          body += sseFrame("reranked", {
            items: search.reranked.items,
            pagination: search.reranked.pagination,
          });
        }
        body += mode === "error-frame"
          ? "event: error\ndata: {}\n\n"
          : "event: done\ndata: {}\n\n";
      }

      await route.fulfill({
        status: 200,
        headers: {
          "Content-Type": "text/event-stream",
          "Cache-Control": "no-store",
        },
        body,
      });
      return true;
    }

    if (url.pathname === "/v1/search" && method === "POST") {
      if (search.postError) {
        await route.fulfill(json(search.postError.status, {
          code: "internal",
          message: search.postError.message,
        }));
        return true;
      }
      const next = searchPosts.shift() ?? {
        query: url.searchParams.get("query") ?? "policy",
        items: [],
        pagination: pagination(1, 8, 0),
      };
      await route.fulfill(json(200, next));
      return true;
    }

    return false;
  }

  async function handleQueue(
    route: Route,
    url: URL,
    method: string,
    body: unknown,
  ): Promise<boolean> {
    const queue = options.queue;
    if (!queue) {
      return false;
    }

    if (url.pathname === "/v1/tasks/stream" && method === "GET" && queue.streamUpdates) {
      const updates = queue.streamUpdates;
      await new Promise((resolve) => setTimeout(resolve, updates.delayMs ?? 500));
      await route.fulfill({
        status: 200,
        headers: {
          "Content-Type": "text/event-stream",
          "Cache-Control": "no-store",
        },
        body: [
          `event: snapshot\ndata: ${JSON.stringify({ tasks: updates.tasks })}\n\n`,
          ...(updates.updates ?? []).map(
            (task) => `event: update\ndata: ${JSON.stringify({ task })}\n\n`,
          ),
        ].join(""),
      });
      return true;
    }

    if (url.pathname === "/v1/tasks/stream" && method === "GET") {
      if (queue.stream === "error") {
        await route.fulfill(json(503, { code: "unavailable", message: "stream unavailable" }));
        return true;
      }
      const tasks = queue.stream === "snapshot"
        ? [...(queue.processing ?? [QUEUE_ACTIVE_TASK, QUEUE_FAILED_TASK])]
        : [];
      await route.fulfill({
        status: 200,
        headers: {
          "Content-Type": "text/event-stream",
          "Cache-Control": "no-store",
        },
        body: `event: snapshot\ndata: ${JSON.stringify({ tasks })}\n\n`,
      });
      return true;
    }

    if (url.pathname === "/v1/tasks" && method === "GET") {
      const hangingView = (url.searchParams.get("view") ?? "processing") === "processing"
        && !url.searchParams.get("kind")
        && !url.searchParams.get("status");
      if (queue.hangListAfter !== undefined && hangingView) {
        const served = listRequests;
        listRequests += 1;
        if (served === queue.hangListAfter) {
          // Never answer this one read: the page must stop waiting on its own
          // deadline, while later reads can still recover normally.
          await new Promise<never>(() => undefined);
        }
      }
      if (queue.listError) {
        await route.fulfill(json(queue.listError.status, {
          code: "internal",
          message: queue.listError.message,
        }));
        return true;
      }
      const view = url.searchParams.get("view") ?? "processing";
      const status = url.searchParams.get("status");
      const kind = url.searchParams.get("kind");
      const stage = url.searchParams.get("stage");
      const waitingReason = url.searchParams.get("waiting_reason");
      const dependencyKey = url.searchParams.get("dependency_key");
      const query = (url.searchParams.get("query") ?? "").trim().toLowerCase();
      const sequence = view === "processing" ? queue.listSequence : undefined;
      const served = sequence ? Math.min(listRequests, sequence.length - 1) : 0;
      if (sequence && queue.hangListAfter === undefined) {
        listRequests += 1;
      }
      const base = sequence?.[served] ?? (view === "completed"
        ? (queue.completed ?? [QUEUE_COMPLETED_TASK])
        : view === "trash"
          ? (queue.trash ?? [QUEUE_TRASHED_TASK])
          : (queue.processing ?? [QUEUE_ACTIVE_TASK, QUEUE_FAILED_TASK]));
      const matches = base.filter((task) => (
        (!status || task.status === status)
        && (!kind || task.kind === kind)
        && (!stage || task.stage === stage)
        && (!waitingReason || task.waiting_reason === waitingReason)
        && (!dependencyKey || task.dependency_key === dependencyKey)
        && (!query || [task.file_name, task.document_title, task.group_path, task.error_summary]
          .some((value) => value?.toLowerCase().includes(query)))
      ));
      const page = Number(url.searchParams.get("page") ?? "1");
      const pageSize = Number(url.searchParams.get("page_size") ?? "25");
      const start = (page - 1) * pageSize;
      await route.fulfill(json(200, {
        items: matches.slice(start, start + pageSize),
        pagination: pagination(page, pageSize, matches.length),
      }));
      return true;
    }

    if (url.pathname === "/v1/tasks/clear" && method === "POST") {
      await route.fulfill(json(200, { deleted_count: 2 }));
      return true;
    }

    const itemsMatch = /^\/v1\/tasks\/([^/]+)\/items$/.exec(url.pathname);
    if (itemsMatch && method === "GET") {
      const taskId = decodeURIComponent(itemsMatch[1]);
      const status = url.searchParams.get("status");
      const sequence = queue.itemSequences?.[taskId];
      const served = sequence ? Math.min(itemRequests[taskId] ?? 0, sequence.length - 1) : 0;
      if (sequence) {
        itemRequests[taskId] = (itemRequests[taskId] ?? 0) + 1;
      }
      const loaded = sequence?.[served] ?? queue.items?.[taskId] ?? QUEUE_ITEMS;
      await route.fulfill(json(200, {
        items: loaded.items.filter((item) => !status || item.status === status),
        next_cursor: loaded.next_cursor ?? null,
      }));
      return true;
    }

    const diagnoseMatch = /^\/v1\/tasks\/([^/]+)\/diagnose$/.exec(url.pathname);
    if (diagnoseMatch && method === "GET") {
      const taskId = decodeURIComponent(diagnoseMatch[1]);
      await route.fulfill(json(200, queue.diagnose?.[taskId] ?? {
        ...QUEUE_DIAGNOSE,
        task: { ...QUEUE_DIAGNOSE.task, task_id: taskId },
      }));
      return true;
    }

    const actionMatch = /^\/v1\/tasks\/([^/]+)\/(retry|rerun|cancel|trash|restore)$/.exec(url.pathname);
    if (actionMatch && method === "POST") {
      const taskId = decodeURIComponent(actionMatch[1]);
      const action = actionMatch[2];
      const source = [
        ...(queue.processing ?? []),
        ...(queue.completed ?? []),
        ...(queue.trash ?? []),
      ].find((task) => task.task_id === taskId);
      if (action === "cancel") {
        await route.fulfill({ status: 204, headers: { "Cache-Control": "no-store" } });
        return true;
      }
      if (action === "retry") {
        await route.fulfill(json(200, { retried_items: 1, task: { task_id: taskId } }));
        return true;
      }
      if (action === "rerun") {
        await route.fulfill(json(200, { task: { task_id: taskId } }));
        return true;
      }
      await route.fulfill(json(200, {
        ...(source ?? QUEUE_FAILED_TASK),
        task_id: taskId,
        deleted_at: action === "trash" ? "2026-01-02T10:00:00Z" : null,
      }));
      return true;
    }

    const deleteMatch = /^\/v1\/tasks\/([^/]+)$/.exec(url.pathname);
    if (deleteMatch && method === "DELETE") {
      await route.fulfill({ status: 204, headers: { "Cache-Control": "no-store" } });
      return true;
    }

    return false;
  }

  async function handleDocuments(route: Route, url: URL, method: string): Promise<boolean> {
    const documents = options.documents;
    if (!documents) {
      return false;
    }
    const match = /^\/v1\/documents\/([^/]+)$/.exec(url.pathname);
    if (!match || method !== "GET") {
      return false;
    }
    const key = decodeURIComponent(match[1]);
    const owner = documents.owners?.[key];
    if (owner && owner !== currentUser.login_name) {
      await route.fulfill(json(403, { code: "forbidden", message: "You do not have access to this document." }));
      return true;
    }
    const failure = documents.failures?.[key];
    if (failure) {
      await route.fulfill(json(failure.status, { code: "reader", message: failure.message }));
      return true;
    }
    const document = documents.documents?.[key] ?? documents.fallback;
    if (!document) {
      await route.fulfill(json(404, { code: "not_found", message: "Document missing" }));
      return true;
    }
    await route.fulfill(json(200, document));
    return true;
  }

  const workspace = options.workspace;
  const workspaceGroup = workspace?.group ?? WORKSPACE_GROUP;
  const workspaceTree: LibraryTreeFixture = structuredClone(workspace?.tree ?? WORKSPACE_TREE);
  if (workspace?.extraRootFolders) {
    const extras: LibraryTreeFixture["root"][] = Array.from(
      { length: workspace.extraRootFolders },
      (_, index) => ({
        group_key: workspaceTree.root.group_key,
        group_path: workspaceTree.root.group_path,
        visibility: workspaceTree.root.visibility,
        folder_id: `folder-extra-${index + 1}`,
        parent_folder_id: null,
        name: `Archive ${index + 1}`,
        path: `Alpha Research/Archive ${index + 1}`,
        processing_count: 0,
        children: [],
        files: [],
      }),
    );
    workspaceTree.root.children = [...extras, ...workspaceTree.root.children];
  }
  let workspaceMembers: GroupMemberFixture[] = [...(workspace?.members ?? WORKSPACE_MEMBERS)];
  let workspaceMetadata: MetadataIndexFixture[] = [...(workspace?.metadata ?? [])];
  let workspaceSourceConfig = JSON.stringify(JSON.parse(SOURCE_CONFIG_TEXT), null, 2);
  let workspaceTranslation = { ...(workspace?.translation ?? WORKSPACE_TRANSLATION) };
  let workspaceChildren = [...(workspace?.children ?? [WORKSPACE_CHILD_GROUP])];
  const taskRef = { task_id: "task-workspace-1" };

  async function fulfillDeletion(route: Route): Promise<void> {
    const deletion = workspace?.deletion;
    if (deletion?.failure) {
      await route.fulfill(json(deletion.failure.status, {
        code: "internal",
        message: deletion.failure.message,
      }));
      return;
    }
    await route.fulfill(json(202, { task_id: deletion?.taskId ?? "task-library-delete" }));
  }

  function workspaceFolder(folderId: string | null): LibraryTreeFixture["root"] {
    if (!folderId) {
      return workspaceTree.root;
    }
    const find = (node: LibraryTreeFixture["root"]): LibraryTreeFixture["root"] | null => {
      if (node.folder_id === folderId) {
        return node;
      }
      for (const child of node.children) {
        const match = find(child);
        if (match) {
          return match;
        }
      }
      return null;
    };
    return find(workspaceTree.root) ?? workspaceTree.root;
  }

  function folderResource(folder: LibraryTreeFixture["root"]): LibraryResourceFixture {
    return {
      kind: "folder",
      id: folder.folder_id as string,
      group_key: folder.group_key,
      group_path: folder.group_path,
      visibility: folder.visibility,
      parent_folder_id: folder.parent_folder_id ?? null,
      name: folder.name,
      created_at: "2026-01-01T08:00:00Z",
      updated_at: "2026-01-02T08:00:00Z",
      child_folder_count: folder.children.length,
      file_count: folder.files.length,
      is_source_folder: folder.files.some((file) => file.filename === "source.json"),
      is_source_records_folder: false,
      processing_count: folder.processing_count,
    };
  }

  function fileResource(folder: LibraryTreeFixture["root"], index: number): LibraryResourceFixture {
    const file = folder.files[index];
    return {
      kind: "file",
      id: file.file_id,
      group_key: file.group_key,
      group_path: file.group_path,
      visibility: file.visibility,
      parent_folder_id: folder.folder_id ?? null,
      name: file.filename,
      media_type: file.media_type,
      size_bytes: file.size_bytes,
      ingest_status: file.ingest_status,
      error_message: file.error_message,
      created_at: file.created_at,
      updated_at: file.updated_at,
      child_folder_count: 0,
      file_count: 0,
      is_source_folder: false,
      is_source_records_folder: false,
      processing_count: 0,
    };
  }

  async function handleWorkspace(
    route: Route,
    url: URL,
    method: string,
    body: unknown,
    rawBody: string,
  ): Promise<boolean> {
    if (!workspace) {
      return false;
    }
    const path = url.pathname;

    if (path === "/v1/groups" && method === "GET") {
      await route.fulfill(json(200, {
        items: [workspaceGroup],
        pagination: pagination(1, 50, 1),
      }));
      return true;
    }
    if (path === "/v1/groups/search") {
      await route.fulfill(json(200, [workspaceGroup, ...workspaceChildren]));
      return true;
    }
    if (path === "/v1/user-directory") {
      await route.fulfill(json(200, [{ user_id: 3, login_name: "auditor", display_name: "Auditor" }]));
      return true;
    }

    const groupMatch = /^\/v1\/groups\/by-path\/([^/]+)(\/.*)?$/.exec(path);
    if (groupMatch) {
      const groupPath = decodeURIComponent(groupMatch[1]);
      const rest = groupMatch[2] ?? "";

      if (rest === "" && method === "GET") {
        await route.fulfill(json(200, { ...workspaceGroup, group_path: groupPath }));
        return true;
      }
      if (rest === "" && method === "PATCH") {
        const patch = (body ?? {}) as Partial<GroupFixture>;
        await route.fulfill(json(200, { ...workspaceGroup, ...patch, group_path: groupPath }));
        return true;
      }
      if (rest === "" && method === "DELETE") {
        await route.fulfill({ status: 204, headers: { "Cache-Control": "no-store" } });
        return true;
      }
      if (rest === "/move") {
        await route.fulfill(json(200, { ...workspaceGroup, parent_group_path: (body as { target_parent_group_path?: string | null })?.target_parent_group_path ?? null }));
        return true;
      }
      if (rest === "/children") {
        await route.fulfill(json(200, {
          items: workspaceChildren,
          pagination: pagination(1, 50, workspaceChildren.length),
        }));
        return true;
      }
      if (rest === "/members" && method === "GET") {
        await route.fulfill(json(200, {
          items: workspaceMembers,
          pagination: pagination(1, 50, workspaceMembers.length),
        }));
        return true;
      }
      if (rest === "/members" && method === "POST") {
        const payload = body as { login_name: string; role: GroupMemberFixture["role"] };
        workspaceMembers = [
          ...workspaceMembers.filter((member) => member.login_name !== payload.login_name),
          { user_id: 9, login_name: payload.login_name, display_name: payload.login_name, role: payload.role },
        ];
        await route.fulfill(json(200, payload));
        return true;
      }
      if (rest.startsWith("/members/") && method === "DELETE") {
        const loginName = decodeURIComponent(rest.slice("/members/".length));
        workspaceMembers = workspaceMembers.filter((member) => member.login_name !== loginName);
        await route.fulfill({ status: 204, headers: { "Cache-Control": "no-store" } });
        return true;
      }
      if (rest === "/library/tree" && method === "GET") {
        await route.fulfill(json(200, workspaceTree));
        return true;
      }
      if (rest === "/library/resources" && method === "GET") {
        if (workspace.resourcesError) {
          await route.fulfill(json(workspace.resourcesError.status, {
            code: "internal",
            message: workspace.resourcesError.message,
          }));
          return true;
        }
        const folderId = url.searchParams.get("folder_id");
        const status = url.searchParams.get("status");
        const recursive = url.searchParams.get("recursive") === "true";
        const folder = workspaceFolder(folderId);
        let items: LibraryResourceFixture[] = [];
        if (status === "failed" && recursive) {
          const failed = workspaceFolder("folder-reports").files
            .map((file, index) => fileResource(workspaceFolder("folder-reports"), index))
            .filter((item) => item.ingest_status === "failed");
          items = failed;
        } else if (folderId) {
          items = folder.files.map((_, index) => fileResource(folder, index));
        } else {
          items = folder.children.map(folderResource);
        }
        const page = Number(url.searchParams.get("page") ?? "1");
        const pageSize = Number(url.searchParams.get("page_size") ?? 50);
        const start = (page - 1) * pageSize;
        await route.fulfill(json(200, {
          items: items.slice(start, start + pageSize),
          pagination: pagination(page, pageSize, items.length),
        }));
        return true;
      }
      const fileMatch = /^\/library\/files\/([^/]+)$/.exec(rest);
      if (fileMatch && method === "GET") {
        const fileId = decodeURIComponent(fileMatch[1]);
        if (fileId === "file-source-config") {
          const detail = workspaceFileDetail(fileId);
          await route.fulfill(json(200, {
            ...detail,
            sections: detail.sections.map((section) => ({
              ...section,
              preview_text: workspaceSourceConfig,
            })),
          }));
          return true;
        }
        await route.fulfill(json(200, workspace.fileDetail && workspace.fileDetail.file_id === fileId
          ? workspace.fileDetail
          : workspaceFileDetail(fileId)));
        return true;
      }
      if (fileMatch && method === "DELETE") {
        await fulfillDeletion(route);
        return true;
      }
      if (rest === "/library/files/prepare-upload") {
        const filename = (body as { filename?: string } | undefined)?.filename ?? "";
        if (workspace.reuseFiles?.includes(filename)) {
          const existing = WORKSPACE_FILES.find((file) => file.filename === filename) ?? {
            ...WORKSPACE_FILES[0],
            file_id: "file-reused",
            filename,
          };
          await route.fulfill(json(200, {
            upload_required: false,
            file: existing,
            task: taskRef,
          }));
          return true;
        }
        await route.fulfill(json(200, { upload_required: true, file: null, task: null }));
        return true;
      }
      if (rest === "/library/files/upload" && method === "POST") {
        await route.fulfill(json(200, taskRef));
        return true;
      }
      if (rest === "/library/folders" && method === "POST") {
        await route.fulfill(json(200, {
          ...workspaceTree.root,
          folder_id: "folder-new",
          name: (body as { name?: string })?.name ?? "New",
          path: "Alpha Research/New",
          children: [],
          files: [],
        }));
        return true;
      }
      const folderMatch = /^\/library\/folders\/([^/]+)$/.exec(rest);
      if (folderMatch && method === "DELETE") {
        await fulfillDeletion(route);
        return true;
      }
      if (rest === "/source-folders" && method === "POST") {
        const payload = body as {
          folder_name: string;
          source_config: { source_key: string };
        };
        const createdFolder: LibraryTreeFixture["root"] = {
          group_key: "alpha",
          group_path: "stock/alpha",
          visibility: "private",
          folder_id: "folder-source-new",
          parent_folder_id: null,
          name: payload.folder_name,
          path: `Alpha Research/${payload.folder_name}`,
          processing_count: 0,
          children: [],
          files: [{
            ...SOURCE_CONFIG_FILE,
            file_id: "file-source-config-new",
            folder_id: "folder-source-new",
            filename: "source.json",
          }],
        };
        workspaceTree.root.children = [...workspaceTree.root.children, createdFolder];
        await route.fulfill(json(200, {
          folder_id: createdFolder.folder_id,
          group_key: "alpha",
          group_path: "stock/alpha",
          name: createdFolder.name,
          source_config: payload.source_config,
        }));
        return true;
      }
      if (rest === "/source-folders/folder-sources/config" && method === "PUT") {
        workspaceSourceConfig = JSON.stringify(body, null, 2);
        await route.fulfill(json(200, { folder_id: "folder-sources", source_config: body }));
        return true;
      }
      if (rest.endsWith("/sync") && rest.startsWith("/source-folders/") && method === "POST") {
        await route.fulfill(json(200, taskRef));
        return true;
      }
      if (rest === "/metadata-indexes/{index_id}/retry" || /^\/metadata-indexes\/[^/]+\/retry$/.test(rest)) {
        workspaceMetadata = workspaceMetadata.map((row) => ({ ...row, status: "ready" }));
        await route.fulfill(json(200, workspaceMetadata[0] ?? WORKSPACE_METADATA));
        return true;
      }
      if (/^\/metadata-indexes\/[^/]+$/.test(rest) && method === "DELETE") {
        const indexId = rest.slice("/metadata-indexes/".length);
        workspaceMetadata = workspaceMetadata.filter((row) => row.index_id !== indexId);
        await route.fulfill({ status: 204, headers: { "Cache-Control": "no-store" } });
        return true;
      }
      if (rest === "/metadata-indexes" && method === "GET") {
        await route.fulfill(json(200, {
          items: workspaceMetadata,
          pagination: pagination(1, 50, workspaceMetadata.length),
        }));
        return true;
      }
      if (rest === "/metadata-indexes" && method === "POST") {
        const payload = body as Partial<MetadataIndexFixture>;
        const created: MetadataIndexFixture = {
          ...WORKSPACE_METADATA,
          ...payload,
          index_id: "66666666-6666-4666-8666-666666666666",
          group_path: groupPath,
          source_key: url.searchParams.get("source_key") ?? "gov_documents",
          status: "ready",
        };
        workspaceMetadata = [created, ...workspaceMetadata];
        await route.fulfill(json(200, created));
        return true;
      }
      if (rest.startsWith("/metadata-indexes/")) {
        await route.fulfill({ status: 204, headers: { "Cache-Control": "no-store" } });
        return true;
      }
      if (rest === "/translation-settings" && method === "GET") {
        await route.fulfill(json(200, workspaceTranslation));
        return true;
      }
      if (rest === "/translation-settings" && method === "PUT") {
        workspaceTranslation = { ...workspaceTranslation, ...(body as object) } as typeof workspaceTranslation;
        await route.fulfill(json(200, workspaceTranslation));
        return true;
      }
    }

    if (path === "/v1/tasks" && method === "POST") {
      if (workspace?.taskAdmissionDelayMs) {
        await new Promise((resolve) => setTimeout(resolve, workspace.taskAdmissionDelayMs));
      }
      await route.fulfill(json(200, workspace?.runningTaskId
        ? { task_id: workspace.runningTaskId }
        : taskRef));
      return true;
    }
    if (path.startsWith("/v1/tasks/") && method === "GET" && workspace?.runningTaskId
      && decodeURIComponent(path.slice("/v1/tasks/".length)) === workspace.runningTaskId) {
      await route.fulfill(json(200, {
        task_id: workspace.runningTaskId,
        kind: "file_batch",
        origin: "manual",
        status: "running",
        stage: "processing",
        waiting_reason: null,
        dependency_key: null,
        failure_stage: null,
        error_summary: null,
        eta_seconds: null,
        progress: {
          total: 1,
          waiting: 0,
          queued: 0,
          running: 1,
          succeeded: 0,
          failed: 0,
          cancelled: 0,
        },
        created_at: "2026-01-01T08:00:00Z",
        updated_at: "2026-01-01T08:00:05Z",
      }));
      return true;
    }
    if (path.startsWith("/v1/tasks/") && method === "GET" && workspace?.deletion) {
      const taskId = decodeURIComponent(path.slice("/v1/tasks/".length));
      if (taskId === (workspace.deletion.taskId ?? "task-library-delete")) {
        const states = workspace.deletion.states ?? ["succeeded"];
        const status = states[Math.min(deletionPolls, states.length - 1)];
        deletionPolls += 1;
        await route.fulfill(json(200, {
          task_id: taskId,
          kind: "file_batch",
          origin: "manual",
          status,
          stage: status === "succeeded" ? "done" : "deleting",
          waiting_reason: null,
          dependency_key: null,
          failure_stage: status === "failed" ? "delete" : null,
          error_summary: status === "failed" ? "The stored object could not be deleted." : null,
          eta_seconds: null,
          progress: {
            total: 1,
            waiting: 0,
            queued: 0,
            running: status === "running" ? 1 : 0,
            succeeded: status === "succeeded" ? 1 : 0,
            failed: status === "failed" ? 1 : 0,
            cancelled: 0,
          },
          created_at: "2026-01-01T08:00:00Z",
          updated_at: "2026-01-01T08:00:05Z",
        }));
        return true;
      }
    }
    if (path.startsWith("/v1/tasks/") && method === "GET") {
      await route.fulfill(json(200, {
        task_id: decodeURIComponent(path.slice("/v1/tasks/".length)),
        kind: "file_batch",
        origin: "manual",
        status: "succeeded",
        stage: "done",
        progress: { total: 1, waiting: 0, queued: 0, running: 0, succeeded: 1, failed: 0, cancelled: 0 },
        created_at: "2026-01-01T08:00:00Z",
        updated_at: "2026-01-01T08:00:01Z",
      }));
      return true;
    }
    if (path === "/v1/search" && method === "POST") {
      const query = (body as { query?: string })?.query ?? "";
      await route.fulfill(json(200, {
        query,
        items: [{
          chunk_id: "chunk-scoped",
          chunk_index: 0,
          chunk_text: "scoped match body",
          document_id: 1,
          external_id: "ext-scoped",
          group_key: "alpha",
          group_path: "stock/alpha",
          source_key: "gov_documents",
          source_uri: "",
          visibility: "private",
          match_reason: "keyword",
          published_at: null,
          title: "Scoped Match",
          score: 1,
          is_library_file: true,
          library_file_id: "file-ready",
          library_path: "Alpha Research/Reports",
        }],
        pagination: pagination(1, 8, 1),
      }));
      return true;
    }

    return false;
  }

  async function handleSettings(route: Route, path: string, method: string, body: unknown): Promise<boolean> {
    if (!options.settings) {
      return false;
    }

    if (path === "/v1/settings/runtime") {
      if (method === "PUT" && body) {
        saved.runtime = { ...saved.runtime, ...(body as object) } as typeof saved.runtime;
      }
      await route.fulfill(json(200, saved.runtime));
      return true;
    }

    if (path === "/v1/settings/docling") {
      if (method === "PUT" && options.settingsFailures?.doclingPut) {
        await route.fulfill(json(options.settingsFailures.doclingPut.status, {
          code: "internal",
          message: options.settingsFailures.doclingPut.message,
        }));
        return true;
      }
      if (method === "PUT" && body) {
        const payload = body as { connection: typeof saved.docling.connection };
        saved.docling = { ...saved.docling, connection: payload.connection };
      }
      await route.fulfill(json(200, saved.docling));
      return true;
    }

    if (path === "/v1/settings/search") {
      if (method === "PUT" && body) {
        const payload = body as Partial<typeof saved.search>;
        saved.search = { ...saved.search, ...payload } as typeof saved.search;
      }
      await route.fulfill(json(200, saved.search));
      return true;
    }

    if (path === "/v1/settings/translation" && method === "PUT" && body) {
      const payload = body as { providers: typeof TRANSLATION_PROVIDERS };
      saved.translation = { providers: payload.providers };
      await route.fulfill(json(200, saved.translation));
      return true;
    }

    if (path === "/v1/settings/translation" && method === "GET") {
      await route.fulfill(json(200, saved.translation));
      return true;
    }

    if (path === "/v1/settings/translation/providers") {
      await route.fulfill(json(200, {
        items: saved.translation.providers,
        pagination: pagination(1, 50, saved.translation.providers.length),
      }));
      return true;
    }

    if (path === "/v1/settings/runtime/embedding/test") {
      if (options.embeddingTestFails) {
        await route.fulfill(json(502, { code: "upstream", message: "embedding refused" }));
      } else {
        await route.fulfill(json(200, {}));
      }
      return true;
    }

    if (path === "/v1/settings/runtime/qdrant/test"
      || path === "/v1/settings/runtime/valkey/test"
      || path === "/v1/settings/runtime/s3/test"
      || path === "/v1/settings/docling/test"
      || path === "/v1/settings/translation/providers/llm/test") {
      await route.fulfill(json(200, path.endsWith("llm/test")
        ? {
          outcome: "success",
          http_status: 200,
          latency_ms: 40,
          llm_api_kind: "openai_chat_completions",
          safe_endpoint: "https://openrouter.ai/api/v1",
        }
        : {}));
      return true;
    }

    if (path === "/v1/settings/runtime/vector-index/rebuild") {
      await route.fulfill(json(200, { task_id: "task-vector-1" }));
      return true;
    }

    if (path === "/v1/tasks") {
      await route.fulfill(json(200, { items: [], pagination: pagination(1, 1, 0) }));
      return true;
    }

    if (path === "/v1/admin/docling/tasks") {
      await route.fulfill(json(200, REMOTE_TASKS));
      return true;
    }

    if (path === "/v1/admin/docling/cleanup") {
      await route.fulfill(json(200, { woken: 1 }));
      return true;
    }

    if (path === "/v1/auth/personal-access-tokens" && method === "GET") {
      await route.fulfill(json(200, {
        items: saved.tokens,
        pagination: pagination(1, 50, saved.tokens.length),
      }));
      return true;
    }

    if (path === "/v1/auth/personal-access-tokens" && method === "POST") {
      const payload = body as { name: string; scopes: (typeof ACCESS_TOKEN)["scopes"]; expires_in_days: number };
      const token = {
        ...ACCESS_TOKEN,
        token_id: "44444444-4444-4444-8444-444444444444",
        name: payload.name,
        scopes: payload.scopes,
      };
      saved.tokens = [token, ...saved.tokens];
      await route.fulfill(json(200, { access_token: "c69_revealed_once", token }));
      return true;
    }

    if (path.startsWith("/v1/auth/personal-access-tokens/")) {
      saved.tokens = saved.tokens.map((token) => ({ ...token, revoked_at: "2026-10-21T08:00:00Z" }));
      await route.fulfill({ status: 204, headers: { "Cache-Control": "no-store" } });
      return true;
    }

    if (path === "/v1/admin/users" && method === "GET") {
      await route.fulfill(json(200, {
        items: saved.users,
        pagination: pagination(1, 50, saved.users.length),
      }));
      return true;
    }

    if (path === "/v1/admin/users" && method === "POST") {
      const payload = body as { login_name: string; display_name: string; is_admin: boolean };
      const created = {
        ...ADMIN_USER_ROW,
        user_id: 9,
        login_name: payload.login_name,
        display_name: payload.display_name,
        is_admin: payload.is_admin,
      };
      saved.users = [...saved.users, created];
      await route.fulfill(json(200, created));
      return true;
    }

    if (path.startsWith("/v1/admin/users/")) {
      await route.fulfill({ status: 204, headers: { "Cache-Control": "no-store" } });
      return true;
    }

    return false;
  }

  await page.route("**/*", async (route) => {
    const request = route.request();
    const url = new URL(request.url());

    if (url.origin !== ALLOWED_ORIGIN) {
      await route.abort("blockedbyclient");
      return;
    }

    if (url.pathname.startsWith("/v1/")) {
      const method = request.method();
      const contentType = request.headers()["content-type"] ?? "";
      const rawBody = request.postDataBuffer()?.toString("utf8") ?? "";
      const body = request.postData() && contentType.includes("application/json")
        ? JSON.parse(request.postData()!) as unknown
        : undefined;
      calls.push({
        method,
        path: url.pathname,
        body,
        query: url.search,
        rawBody: rawBody || undefined,
      });

      if (url.pathname === "/v1/auth/login" && method === "POST") {
        if (options.loginRejects) {
          await route.fulfill(json(401, AUTH_ERROR));
          return;
        }
        const login = (body as { login_name?: string } | undefined)?.login_name;
        currentUser = options.accounts?.find((account) => account.login_name === login) ?? currentUser;
        signedIn = true;
        await route.fulfill({ status: 204, headers: { "Cache-Control": "no-store" } });
        return;
      }

      if (url.pathname === "/v1/auth/me") {
        if (options.meStatus) {
          await route.fulfill(json(options.meStatus, { code: "service_unavailable", message: "Unavailable" }));
          return;
        }
        if (!signedIn) {
          await route.fulfill(json(401, AUTH_ERROR));
          return;
        }
        await route.fulfill(json(200, { user: currentUser }));
        return;
      }

      if (url.pathname === "/v1/auth/logout") {
        signedIn = false;
        await route.fulfill({ status: 204, headers: { "Cache-Control": "no-store" } });
        return;
      }

      if (await handleQueue(route, url, method, body)) {
        return;
      }

      if (await handleDocuments(route, url, method)) {
        return;
      }

      if (await handleWorkspace(route, url, method, body, rawBody)) {
        return;
      }

      if (await handleSearch(route, url, method)) {
        return;
      }

      if (await handleSettings(route, url.pathname, method, body)) {
        return;
      }

      unmocked.push(`${method} ${url.pathname}`);
      await route.fulfill(json(501, { code: "unmocked", message: `unmocked ${method} ${url.pathname}` }));
      return;
    }

    await route.fallback();
  });

  return { calls, unmocked };
}

export interface AuthTransportOptions extends AppTransportOptions {
  user?: AuthUserFixture;
  loginRejects?: boolean;
  meStatus?: number;
}

export async function installAuthTransport(page: Page, options: AuthTransportOptions = {}) {
  await installAppTransport(page, options);
}

/** Records every request origin so the suite can prove nothing left the app. */
export function recordExternalRequests(page: Page) {
  const external: string[] = [];

  page.on("request", (request) => {
    const url = new URL(request.url());
    if (url.origin !== ALLOWED_ORIGIN && url.protocol !== "data:" && url.protocol !== "blob:") {
      external.push(request.url());
    }
  });

  return external;
}
