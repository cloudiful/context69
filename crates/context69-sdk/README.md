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
let text_task = client.text_batch(&spec.group_path, &texts).await?;
let file_task = client.file_batch(&spec.group_path, &files).await?;
let text_result = client.wait(text_task.task_id, std::time::Duration::from_secs(600)).await?;
println!("{} text items succeeded", text_result.progress.succeeded);
let file_result = client.wait(file_task.task_id, std::time::Duration::from_secs(1800)).await?;
println!("{} file items failed", file_result.progress.failed);
# Ok(())
# }
```

Use `url_batch` for remote files. A one-item batch is the single-item form;
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
text/URL/file batches (with SDK-generated `Idempotency-Key`), task
submit/`get_task`/`list_tasks`/`list_task_items`/wait/retry/rerun/cancel/
trash/restore/`purge_trashed_task`, task maintenance, `search` plus the
bounded `search_compact` projection, document detail/batch detail,
extraction templates, health, and authentication identity. Every type needed
to construct a call (`TaskListView`, `TaskItemsQuery`, `SortDirection`,
`SourcePolicy`, `IngestOptions`, canonical search/settings types) is
re-exported from `context69_sdk` alone. The complete low-level transport
covering all 116 HTTP operations is Task 4b remaining work and is not
claimed here; low-level REST handles and `authorized_request` are not part
of this SDK.
