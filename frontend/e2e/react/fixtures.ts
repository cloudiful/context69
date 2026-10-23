import type { components } from "../../src/generated/openapi";

type Schemas = components["schemas"];

export type AuthUserFixture = Schemas["AuthUserResponse"];
export type AuthMeFixture = Schemas["AuthMeResponse"];
export type RuntimeSettingsFixture = Schemas["RuntimeSettingsResponse"];
export type DoclingSettingsFixture = Schemas["DoclingSettingsResponse"];
export type SearchSettingsFixture = Schemas["SearchSettingsResponse"];
export type TranslationSettingsFixture = Schemas["TranslationSettingsResponse"];
export type PersonalAccessTokenFixture = Schemas["PersonalAccessTokenResponse"];
export type AdminUserFixture = Schemas["AdminUserResponse"];
export type DoclingRemoteTasksFixture = Schemas["DoclingRemoteTasksResponse"];
export type PaginationFixture = Schemas["Pagination"];
export type SearchHitFixture = Schemas["SearchHit"];
export type SearchPaginationFixture = Schemas["SearchPagination"];
export type SearchResponseFixture = Schemas["SearchResponse"];
export type DocumentFixture = Schemas["DocumentResponse"];
export type SourceStatusFixture = Schemas["SourceStatus"];
export type GroupFixture = Schemas["GroupResponse"];
export type GroupMemberFixture = Schemas["GroupMemberResponse"];
export type LibraryTreeFixture = Schemas["LibraryTreeResponse"];
export type LibraryFolderFixture = Schemas["LibraryFolderNode"];
export type LibraryFileFixture = Schemas["LibraryFileSummary"];
export type LibraryResourceFixture = Schemas["LibraryResourceItem"];
export type LibraryFileDetailFixture = Schemas["LibraryFileDetailResponse"];
export type MetadataIndexFixture = Schemas["MetadataIndexResponse"];
export type MetadataIndexPageFixture = Schemas["MetadataIndexPageResponse"];
export type GroupTranslationSettingsFixture = Schemas["GroupTranslationSettingsResponse"];
export type TaskRefFixture = Schemas["TaskRef"];
export type TaskFixture = Schemas["TaskResponse"];
export type TaskItemsFixture = Schemas["TaskItemsResponse"];
export type TaskDiagnoseFixture = Schemas["TaskDiagnoseResponse"];

export const ADMIN_USER: AuthUserFixture = {
  user_id: 1,
  login_name: "admin",
  display_name: "Administrator",
  is_admin: true,
  personal_group_path: "personal-admin",
  personal_group_role: "owner",
};

export const MEMBER_USER: AuthUserFixture = {
  user_id: 2,
  login_name: "planner",
  display_name: "Planner",
  is_admin: false,
  personal_group_path: "personal-planner",
  personal_group_role: "owner",
};

export const AUTH_ME: AuthMeFixture = { user: ADMIN_USER };

export const AUTH_ERROR: Schemas["ApiErrorResponse"] = {
  code: "unauthorized",
  message: "Invalid login name or password.",
};

export const RUNTIME_SETTINGS: RuntimeSettingsFixture = {
  qdrant: {
    url: "http://qdrant:6333",
    collection_name: "context69",
    recreate_on_dimension_mismatch: false,
  },
  embedding: {
    base_url: "http://embedding:8080/v1",
    model: "embed-1",
    dimensions: 768,
    timeout_secs: 30,
    has_api_key: true,
  },
  scheduler: {
    interval_secs: 300,
    run_on_start: true,
    max_concurrency: 4,
    job_id: "context69-sync",
    valkey_url: "redis://valkey:6379/0",
  },
  chunking: { max_chars: 1200, overlap_chars: 200 },
  requires_rebuild: false,
  file_library: {
    storage_root: "/data/library",
    max_upload_size_mb: 128,
    max_upload_request_size_mb: 128,
    ingest_concurrency: 2,
    url_import_concurrency: 2,
    url_import_min_interval_ms: 1000,
    trusted_proxy_enabled: false,
  },
};

export const DOCLING_SETTINGS: DoclingSettingsFixture = {
  configured: true,
  source: "database",
  connection: {
    base_url: "http://docling:5001",
    timeout_secs: 120,
    poll_interval_secs: 2,
    task_timeout_secs: 600,
    max_inflight: 2,
  },
  vlm: {
    has_api_key: false,
    openai_base_url: null,
    picture_description_preset: null,
    picture_description_model: null,
    vlm_pipeline_model: null,
    code_formula_model: null,
  },
};

export const SEARCH_SETTINGS: SearchSettingsFixture = {
  mode: "hybrid",
  rerank_enabled: true,
  rerank_base_url: "https://openrouter.ai/api/v1",
  rerank_model: "cohere/rerank-4-fast",
  candidate_limit: 40,
  timeout_secs: 10,
  vector_weight: 0.55,
  keyword_weight: 0.35,
  has_api_key: true,
};

export const TRANSLATION_PROVIDERS: Schemas["TranslationProviderResponse"][] = [
  {
    provider: "deepl",
    enabled: false,
    priority: 0,
    endpoint: null,
    model: null,
    llm_api_kind: null,
    deepl_plan: "free",
    monthly_character_limit: 500_000,
    has_api_key: true,
    current_month_characters: 1200,
  },
  {
    provider: "llm",
    enabled: true,
    priority: 1,
    endpoint: "https://openrouter.ai/api/v1",
    model: "openai/gpt-4.1-mini",
    llm_api_kind: "openai_chat_completions",
    deepl_plan: null,
    monthly_character_limit: null,
    has_api_key: true,
    current_month_characters: 340,
  },
  {
    provider: "libretranslate",
    enabled: false,
    priority: 2,
    endpoint: null,
    model: null,
    llm_api_kind: null,
    deepl_plan: null,
    monthly_character_limit: null,
    has_api_key: false,
    current_month_characters: 0,
  },
];

export const TRANSLATION_SETTINGS: TranslationSettingsFixture = {
  providers: TRANSLATION_PROVIDERS,
};

export const ACCESS_TOKEN: PersonalAccessTokenFixture = {
  token_id: "11111111-1111-4111-8111-111111111111",
  name: "cli",
  display_prefix: "c69_abc",
  scopes: ["search"],
  created_at: "2026-10-01T08:00:00Z",
  expires_at: "2026-11-01T08:00:00Z",
  last_used_at: null,
  revoked_at: null,
  updated_at: "2026-10-01T08:00:00Z",
};

export const ADMIN_USER_ROW: AdminUserFixture = {
  user_id: 2,
  login_name: "planner",
  display_name: "Planner",
  is_admin: false,
  disabled_at: null,
  created_at: "2026-09-01T08:00:00Z",
  updated_at: "2026-09-01T08:00:00Z",
};

export const REMOTE_TASKS: DoclingRemoteTasksFixture = {
  items: [
    {
      task_id: "remote-1",
      task_type: "convert",
      task_status: "started",
      created_at: "2026-10-10T08:00:00Z",
    },
  ],
  cleanup: [],
  has_more: false,
  remote_error: null,
};

export function pagination(page: number, pageSize: number, total: number): PaginationFixture {
  return {
    page,
    page_size: pageSize,
    total,
    total_pages: Math.max(1, Math.ceil(total / Math.max(1, pageSize))),
  };
}

export function searchHitFixture(
  overrides: Partial<SearchHitFixture> & Pick<SearchHitFixture, "chunk_id" | "title" | "score">,
): SearchHitFixture {
  const { chunk_id, title, score, ...rest } = overrides;
  return {
    chunk_id,
    chunk_index: 0,
    chunk_text: `${title} body text mentions policy details.`,
    document_id: 1,
    external_id: `ext-${chunk_id}`,
    group_key: "group",
    group_path: "personal-admin/default",
    match_reason: "semantic",
    published_at: "2026-10-01T08:00:00Z",
    source_key: "gov_documents",
    source_uri: "https://example.test/doc",
    visibility: "private",
    ...rest,
    title,
    score,
  };
}

export function searchPageFixture(
  query: string,
  items: SearchHitFixture[],
  overrides: Partial<SearchPaginationFixture> = {},
): SearchResponseFixture {
  return {
    query,
    items,
    pagination: {
      page: 1,
      page_size: 8,
      total: items.length,
      total_pages: 1,
      has_more: false,
      ...overrides,
    },
  };
}

export const PREVIEW_DOCUMENT: DocumentFixture = {
  document_id: 1,
  external_id: "ext-doc-1",
  group_key: "group",
  group_path: "personal-admin/default",
  record_hash: "hash-1",
  source_key: "gov_documents",
  source_uri: "https://example.test/doc",
  title: "Preview Document",
  summary: "Stored summary for the preview.",
  updated_at: "2026-10-01T08:00:00Z",
  visibility: "private",
  chunks: [
    {
      chunk_id: "chunk-doc-1",
      chunk_index: 0,
      record_hash: "hash-1",
      text: "Stored document body about policy.",
    },
  ],
};

export const SOURCE_STATUS: SourceStatusFixture = {
  base_query: "",
  batch_size: 100,
  connection: "primary",
  connector_type: "postgres_sql",
  display_name: "Government Documents",
  group_key: "group",
  group_path: "personal-admin/default",
  has_database_url: true,
  origin_status: "connected",
  source_key: "gov_documents",
  sync_strategy: "full_scan",
  visibility: "private",
};

export const GROUP: GroupFixture = {
  group_id: 1,
  group_key: "group",
  group_path: "personal-admin/default",
  kind: "personal",
  name: "Default",
  visibility: "private",
  created_at: "2026-01-01T08:00:00Z",
  updated_at: "2026-01-01T08:00:00Z",
};

export const WORKSPACE_GROUP: GroupFixture = {
  group_id: 11,
  group_key: "alpha",
  group_path: "stock/alpha",
  kind: "shared",
  name: "Alpha Research",
  visibility: "private",
  current_role: "owner",
  created_at: "2026-01-01T08:00:00Z",
  updated_at: "2026-01-02T08:00:00Z",
};

export const WORKSPACE_CHILD_GROUP: GroupFixture = {
  group_id: 12,
  group_key: "beta",
  group_path: "stock/alpha/beta",
  kind: "shared",
  name: "Beta Desk",
  visibility: "public",
  current_role: "viewer",
  created_at: "2026-01-01T08:00:00Z",
  updated_at: "2026-01-02T08:00:00Z",
};

export const WORKSPACE_FILES: LibraryFileFixture[] = [
  {
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
    created_at: "2026-01-01T08:00:00Z",
    updated_at: "2026-01-02T08:00:00Z",
    ingested_at: "2026-01-02T08:00:00Z",
  },
  {
    file_id: "file-failed",
    group_key: "alpha",
    group_path: "stock/alpha",
    visibility: "private",
    folder_id: "folder-reports",
    filename: "broken.pdf",
    media_type: "application/pdf",
    size_bytes: 90,
    ingest_status: "failed",
    error_message: "parse failed",
    created_at: "2026-01-01T08:00:00Z",
    updated_at: "2026-01-02T08:00:00Z",
    ingested_at: null,
  },
];

export const SOURCE_CONFIG_TEXT = JSON.stringify({
  source_key: "gov_documents",
  display_name: "Government Documents",
  description: "",
  example_queries: ["policy"],
  connection: "primary",
  sync_strategy: "cursor",
  connector_type: "postgres_sql",
  base_query: "SELECT 1",
  batch_size: 200,
}, null, 2);

export const SOURCE_CONFIG_FILE: LibraryFileFixture = {
  file_id: "file-source-config",
  group_key: "alpha",
  group_path: "stock/alpha",
  visibility: "private",
  folder_id: "folder-sources",
  filename: "source.json",
  media_type: "application/json",
  size_bytes: 240,
  ingest_status: "succeeded",
  error_message: null,
  created_at: "2026-01-01T08:00:00Z",
  updated_at: "2026-01-02T08:00:00Z",
  ingested_at: "2026-01-02T08:00:00Z",
};

export const WORKSPACE_TREE: LibraryTreeFixture = {
  root: {
    group_key: "alpha",
    group_path: "stock/alpha",
    visibility: "private",
    folder_id: null,
    parent_folder_id: null,
    name: "Alpha Research",
    path: "Alpha Research",
    processing_count: 0,
    children: [
      {
        group_key: "alpha",
        group_path: "stock/alpha",
        visibility: "private",
        folder_id: "folder-reports",
        parent_folder_id: null,
        name: "Reports",
        path: "Alpha Research/Reports",
        processing_count: 0,
        children: [],
        files: WORKSPACE_FILES,
      },
      {
        group_key: "alpha",
        group_path: "stock/alpha",
        visibility: "private",
        folder_id: "folder-sources",
        parent_folder_id: null,
        name: "Sources",
        path: "Alpha Research/Sources",
        processing_count: 0,
        children: [],
        files: [SOURCE_CONFIG_FILE],
      },
    ],
    files: [],
  },
};

export const WORKSPACE_FILE_DETAIL: LibraryFileDetailFixture = {
  file_id: "file-ready",
  group_key: "alpha",
  group_path: "stock/alpha",
  visibility: "private",
  folder_id: "folder-reports",
  folder_path: "Reports",
  filename: "quarterly.md",
  media_type: "text/markdown",
  size_bytes: 120,
  sha256: "a".repeat(64),
  ingest_status: "succeeded",
  error_message: null,
  created_at: "2026-01-01T08:00:00Z",
  updated_at: "2026-01-02T08:00:00Z",
  ingested_at: "2026-01-02T08:00:00Z",
  source_available: true,
  sections: [
    {
      document_id: 1,
      section_key: "body",
      section_label: "Body",
      sort_order: 0,
      title: "Body",
      preview_text: "Preview body from the browser suite.",
      content_format: "markdown",
    },
  ],
};

export const WORKSPACE_MEMBERS: GroupMemberFixture[] = [
  { user_id: 1, login_name: "admin", display_name: "Administrator", role: "owner" },
  { user_id: 2, login_name: "planner", display_name: "Planner", role: "viewer" },
];

export const WORKSPACE_METADATA: MetadataIndexFixture = {
  index_id: "55555555-5555-4555-8555-555555555555",
  group_path: "stock/alpha",
  source_key: "gov_documents",
  path: "$.region",
  data_type: "keyword",
  value_kind: "scalar",
  sortable: false,
  status: "ready",
  processed_documents: 3,
  total_documents: 3,
  error_message: null,
  created_at: "2026-01-01T08:00:00Z",
  updated_at: "2026-01-02T08:00:00Z",
};

export const WORKSPACE_TRANSLATION: GroupTranslationSettingsFixture = {
  enabled: true,
  default_target_locales: ["zh-CN"],
  source_locale: "en-US",
  glossary: [],
  queued_count: 0,
  running_count: 0,
  succeeded_count: 2,
  failed_count: 0,
};

export function workspaceFileDetail(fileId: string): LibraryFileDetailFixture {
  if (fileId === SOURCE_CONFIG_FILE.file_id) {
    return {
      ...WORKSPACE_FILE_DETAIL,
      file_id: fileId,
      folder_id: "folder-sources",
      folder_path: "Sources",
      filename: "source.json",
      media_type: "application/json",
      sections: [{
        document_id: 2,
        section_key: "source",
        section_label: "source.json",
        sort_order: 0,
        title: "source.json",
        preview_text: SOURCE_CONFIG_TEXT,
        content_format: "plain_text",
      }],
    };
  }
  return {
    ...WORKSPACE_FILE_DETAIL,
    file_id: fileId,
    filename: WORKSPACE_FILES.find((file) => file.file_id === fileId)?.filename ?? "file",
    ingest_status: fileId === "file-failed" ? "failed" : "succeeded",
  };
}

export const READER_CHUNKS: Schemas["DocumentChunkResponse"][] = Array.from({ length: 12 }, (_, index) => ({
  chunk_id: `reader-chunk-${index + 1}`,
  chunk_index: index,
  record_hash: "reader-hash",
  text: `## Block ${index + 1}\n\nStored **reader** body ${index + 1} about cyber policy.`,
}));

export const READER_DOCUMENT: DocumentFixture = {
  document_id: 42,
  source_key: "gov_documents",
  group_key: "alpha",
  group_path: "stock/alpha",
  external_id: "ext-reader-42",
  record_hash: "reader-hash",
  visibility: "private",
  title: "Cyber Policy Update",
  summary: "Stored summary about cyber policy and its scope.",
  source_uri: "https://example.test/doc",
  published_at: "2026-01-01",
  updated_at: "2026-01-02T08:30:00Z",
  metadata_json: {},
  content_locale: "zh-CN",
  requested_locale: "en-US",
  translation_status: "succeeded",
  is_fallback: false,
  library_path: "Alpha Research/Reports",
  library_section_label: "Body",
  is_library_file: false,
  library_file_id: null,
  chunks: READER_CHUNKS,
};

export const LIBRARY_DOCUMENT: DocumentFixture = {
  ...READER_DOCUMENT,
  document_id: 43,
  title: "Quarterly Filing",
  is_library_file: true,
  library_file_id: "file-ready",
  group_path: "stock/alpha",
  chunks: [{
    chunk_id: "library-chunk-1",
    chunk_index: 0,
    record_hash: "reader-hash",
    text: "Quarterly filing body.",
  }],
};

export const STALE_DOCUMENT: DocumentFixture = {
  ...READER_DOCUMENT,
  document_id: 44,
  title: "Changing Document",
  summary: null,
  chunks: [
    { chunk_id: "stale-ok", chunk_index: 0, record_hash: "reader-hash", text: "current stored block" },
    { chunk_id: "stale-old", chunk_index: 1, record_hash: "older-hash", text: "stale secret block" },
    { chunk_id: "stale-empty", chunk_index: 2, record_hash: "reader-hash", text: "   " },
  ],
};

export const HOSTILE_DOCUMENT: DocumentFixture = {
  ...READER_DOCUMENT,
  document_id: 45,
  title: "Hostile Content",
  summary: null,
  chunks: [{
    chunk_id: "hostile-chunk",
    chunk_index: 0,
    record_hash: "reader-hash",
    text: [
      "<script>window.__p3cE2eXss = 1</script>",
      '<img src=x onerror="window.__p3cE2eXss = 2">',
      "[click me](javascript:window.__p3cE2eXss = 4)",
      "[safe](https://example.test/ok)",
      "[data](data:text/html;base64,PHNjcmlwdD53aW5kb3cuX19wM2NFMmVYc3M9NTwvc2NyaXB0Pg==)",
    ].join("\n\n"),
  }],
};


const TASK_PROGRESS = { total: 4, queued: 0, running: 1, waiting: 0, succeeded: 2, failed: 1, cancelled: 0 };

export function queueTaskFixture(
  overrides: Partial<TaskFixture> & Pick<TaskFixture, "task_id">,
): TaskFixture {
  return {
    kind: "file_batch",
    origin: "manual",
    status: "running",
    group_path: "stock/alpha",
    source_key: null,
    stage: "indexing",
    waiting_reason: null,
    dependency_key: null,
    failure_stage: null,
    error_summary: null,
    eta_seconds: null,
    created_at: "2026-01-01T08:00:00Z",
    updated_at: "2026-01-01T08:05:00Z",
    progress: { ...TASK_PROGRESS },
    ...overrides,
  };
}

export const QUEUE_ACTIVE_TASK = queueTaskFixture({
  task_id: "task-queue-active",
  file_name: "field-notes.pdf",
  document_title: "Field Notes",
});

export const QUEUE_FAILED_TASK = queueTaskFixture({
  task_id: "task-queue-failed",
  status: "failed",
  file_name: "policy-review.pdf",
  document_title: "Policy Review",
  failure_stage: "indexing",
  error_summary: "Embedding provider timed out",
  progress: { total: 4, queued: 0, running: 0, waiting: 0, succeeded: 2, failed: 2, cancelled: 0 },
});

export const QUEUE_COMPLETED_TASK = queueTaskFixture({
  task_id: "task-queue-completed",
  status: "succeeded",
  stage: "finalize",
  file_name: "annual-report.pdf",
  document_title: "Annual Report",
  dependency_key: "embedding",
  progress: { total: 4, queued: 0, running: 0, waiting: 0, succeeded: 4, failed: 0, cancelled: 0 },
});

export const QUEUE_TRASHED_TASK = queueTaskFixture({
  task_id: "task-queue-trashed",
  status: "cancelled",
  file_name: "old-draft.pdf",
  deleted_at: "2026-01-02T09:00:00Z",
});

export const QUEUE_ITEMS: TaskItemsFixture = {
  items: [
    {
      item_id: "queue-item-1",
      ordinal: 1,
      status: "failed",
      attempt_count: 2,
      retryable: false,
      file_name: "policy-review.pdf",
      error_message: "Embedding provider timed out",
      progress: { attempt: 2, stage: "indexing", total_chunks: 10, completed_chunks: 4 },
      created_at: "2026-01-01T08:00:00Z",
    },
    {
      item_id: "queue-item-2",
      ordinal: 2,
      status: "succeeded",
      attempt_count: 1,
      retryable: false,
      file_name: "policy-appendix.pdf",
      progress: { attempt: 1, stage: "finalizing", total_chunks: 3, completed_chunks: 3 },
      created_at: "2026-01-01T08:01:00Z",
    },
  ],
  next_cursor: null,
};

export const QUEUE_DIAGNOSE: TaskDiagnoseFixture = {
  consistency: {
    consistent: false,
    mismatches: ["status"],
    open_attempt_count: 1,
    near_exhaustion_item_count: 1,
  },
  dependency_gates: [{
    dependency_key: "embedding",
    failure_count: 2,
    last_error: "connection refused",
    last_success_at: null,
    last_transition_at: "2026-01-01T08:04:00Z",
    next_probe_at: "2026-01-01T08:06:00Z",
    state: "half_open",
  }],
  items: [{
    item_id: "queue-item-1",
    ordinal: 1,
    status: "failed",
    attempt_count: 2,
    retryable: false,
    stage: "indexing",
    waiting_reason: "dependency",
    dependency_key: "embedding",
    next_attempt_at: "2026-01-01T08:07:00Z",
    lease_expires_at: null,
    active_attempt: null,
    error_message: "Embedding provider timed out",
    created_at: "2026-01-01T08:00:00Z",
  }],
  items_truncated: false,
  observed_at: "2026-01-01T08:08:00Z",
  task: {
    task_id: "task-queue-failed",
    kind: "file_batch",
    status: "failed",
    stage: "indexing",
    created_at: "2026-01-01T08:00:00Z",
    updated_at: "2026-01-01T08:05:00Z",
    waiting_reason: null,
    dependency_key: "embedding",
    progress: { total: 4, queued: 0, running: 0, waiting: 0, succeeded: 2, failed: 2, cancelled: 0 },
  },
};
