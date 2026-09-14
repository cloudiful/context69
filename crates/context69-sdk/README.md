# context69-sdk

The SDK exposes the high-level Context69 workflow. Callers provision a scope,
submit batch writes, and observe one unified task lifecycle. HTTP transport,
resource handles, metadata-index workers, queues, leases, and polling of file
or URL jobs are intentionally internal to Context69.

## Initialization

```rust,no_run
use context69_sdk::Context69Client;

# async fn example() -> Result<(), Box<dyn std::error::Error>> {
let client = Context69Client::builder()
    .base_url("http://127.0.0.1:8096")?
    .with_personal_access_token("ctx_pat_example")?
    .build()?;
let me = client.me().await?;
println!("hello {}", me.user.login_name);
# Ok(())
# }
```

## Provision and ingest

`ensure_scope` is idempotent. It creates the group when needed, validates an
existing definition, creates declared metadata indexes, and waits until those
indexes are ready.

```rust,no_run
use context69_sdk::{
    Context69Client, FileBatchRequest, ScopeSpec, TextBatchRequest,
};

# async fn example(client: &Context69Client, spec: ScopeSpec, texts: TextBatchRequest,
#     files: FileBatchRequest) -> Result<(), Box<dyn std::error::Error>> {
client.ensure_scope(&spec).await?;
let text_task = client.submit_text_batch(&spec.group_path, &texts).await?;
let file_task = client.submit_file_batch(&spec.group_path, &files).await?;
let text_result = client.wait(text_task.task_id, std::time::Duration::from_secs(600)).await?;
println!("{} text items succeeded", text_result.progress.succeeded);
let file_result = client.wait(file_task.task_id, std::time::Duration::from_secs(1800)).await?;
println!("{} file items failed", file_result.progress.failed);
# Ok(())
# }
```

Use `submit_url_batch` for remote files. A one-item batch is the single-item form;
there is no second single-write API to learn.

Each batch receives a stable SDK-generated `Idempotency-Key` derived from its
endpoint and request body. Repeating the same request returns the original
task. The server also applies each resource's natural identity when a request
is retried.

## Task center

```rust,no_run
use context69_sdk::{Context69Client, TaskItemsOptions, TaskListOptions};

# use std::time::Duration;
# use uuid::Uuid;
# async fn example(client: &Context69Client, task_id: Uuid) -> Result<(), Box<dyn std::error::Error>> {
let task = client.get_task(task_id).await?;
let items = client.list_task_items(task.task_id, &TaskItemsOptions::default()).await?;
let page = client.list_tasks(&TaskListOptions::default()).await?;
if task.progress.failed > 0 {
    let retry = client.retry_task(task.task_id).await?;
    client.wait(retry.task.task_id, Duration::from_secs(600)).await?;
}
client.cancel_task(task.task_id).await?;
# let _ = (items, page);
# Ok(())
# }
```

`get_task` reports lifecycle state, aggregate progress, failure summary,
timestamps, and an estimated remaining time when enough progress exists.
`list_tasks` requires a typed `TaskListOptions` (`view`, `page 1..=10_000`,
`page_size 1..=100`; default `processing`/page 1/size 25) and returns the
canonical `TaskPageResponse`. `list_task_items` takes a bounded
`TaskItemsOptions` (`limit 1..=100`, default 100; `cursor` omitted when
`None`) and returns `next_cursor` continuation. `wait` uses bounded
exponential backoff and retries transient transport/server failures until
its timeout. `purge_trashed_task` permanently deletes trashed history
(`DELETE /v1/tasks/{task_id}`).

## Retrieval

```rust,no_run
use context69_sdk::{BatchGetDocumentsRequest, Context69Client, SearchRequest};

# async fn example(client: &Context69Client, request: SearchRequest)
#     -> Result<(), Box<dyn std::error::Error>> {
let result = client.search(&request).await?;
let next = result.pagination.next_cursor.clone();
let compact = client.search_compact(&request).await?;
assert_eq!(compact.pagination.next_cursor, next);
let document = client.get_document(result.items[0].document_id, None).await?;
let documents = client.get_documents("research/news", &BatchGetDocumentsRequest {
    keys: vec![],
    locale: None,
}).await?;
# let _ = (document, documents, compact);
# Ok(())
# }
```

`search` returns the full `SearchResponse` with `SearchPagination`
(`next_cursor`/`has_more`). `search_compact` is a bounded 320-char snippet
projection that preserves the same pagination instead of dropping it.
Request document detail only when the caller needs metadata or chunks;
`get_documents` preserves one result per requested key.

## Public surface

The public client surface is the ergonomic facade: scope provisioning,
text/URL/file/delete batches via `submit_text_batch`/`submit_url_batch`/
`submit_file_batch`/`submit_delete_batch` (SDK-generated `Idempotency-Key`), task
submit/`get_task`/`list_tasks`/`list_task_items`/wait/retry/rerun/cancel/
trash/restore/`purge_trashed_task`, task maintenance, `search` plus the
bounded `search_compact` projection, document detail/batch detail,
extraction templates, health, and authentication identity. The v0.15 aliases
(`text_batch`, `url_batch`, `file_batch`, `delete_batch`, `task`, `tasks`,
`task_items`, `delete_task`) remain as deprecated shims in this release and
are removed after the v0.16 cutover; new code must use the canonical names.
Every type needed
to construct a call (`TaskListView`, `TaskItemsQuery`, `SortDirection`,
`SourcePolicy`, `IngestOptions`, canonical search/settings types) is
re-exported from `context69_sdk` alone. Ingest callers send
`IngestOptions` (`source_policy: retain | release_after_processing`);
search settings updates send `SecretPatch` (`{op: keep|set|clear}`).

## Raw transport (all 116 operations)

`client.raw()` is the complete low-level surface. Every HTTP operation in
the current OpenAPI appears exactly once in `OPERATIONS` (sorted by
`operation_id` with method, path template, auth, body kind, shared-contract
request/response names, success status, and idempotency). Build a
`RawRequest` with explicit `path_param`/`query`/`query_opt`/`json_body`
helpers and run it with `RawClient::execute`; decode success JSON with
`RawResponse::decode` into shared contract types.

```rust,no_run
use context69_sdk::{Context69Client, RawRequest};

# async fn example(client: &Context69Client) -> Result<(), Box<dyn std::error::Error>> {
let request = RawRequest::new("get_task")?
    .path_param("task_id", "11111111-1111-1111-1111-111111111111");
let raw = client.raw().execute(&request).await?;
let task: context69_sdk::TaskResponse = raw.decode()?;
# let _ = task;
# Ok(())
# }
```

Path segments use the shared transport encoder, queries are sent
explicitly with no hidden defaults, `healthz`/`login`/`logout` need no
token while every other operation sends `Bearer`, and the six
task-submitting operations support a stable `ctx69-sdk-*`
`Idempotency-Key` via `with_auto_idempotency_key`. Limitation: with the
current `reqwest` features the two `multipart/form-data` uploads accept
pre-encoded bytes via `raw_body`, `search_stream` returns SSE bytes for
the caller to parse, and there are no per-operation typed methods by
design; the generic registry plus shared contract types is the surface.
