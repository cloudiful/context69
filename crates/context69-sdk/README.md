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
# use context69_sdk::Context69Client;
# use std::time::Duration;
# use uuid::Uuid;
# async fn example(client: &Context69Client, task_id: Uuid) -> Result<(), Box<dyn std::error::Error>> {
let task = client.task(task_id).await?;
let items = client.task_items(task.task_id, None).await?;
if task.progress.failed > 0 {
    let retry = client.retry_task(task.task_id).await?;
    client.wait(retry.task.task_id, Duration::from_secs(600)).await?;
}
client.cancel_task(task.task_id).await?;
# let _ = items;
# Ok(())
# }
```

`task` reports lifecycle state, aggregate progress, failure summary, timestamps,
and an estimated remaining time when enough progress exists. `task_items`
returns independent status, resource id, failure stage, error message, attempt
count, and retryability for every item. `wait` uses bounded exponential backoff
and retries transient transport/server failures until its timeout.

## Retrieval

```rust,no_run
use context69_sdk::{BatchGetDocumentsRequest, Context69Client, SearchRequest};

# async fn example(client: &Context69Client, request: SearchRequest)
#     -> Result<(), Box<dyn std::error::Error>> {
let result = client.search_compact(&request).await?;
let document = client.get_document(result.hits[0].document_id, None).await?;
let documents = client.get_documents("research/news", &BatchGetDocumentsRequest {
    keys: vec![],
    locale: None,
}).await?;
# let _ = (document, documents);
# Ok(())
# }
```

Search intentionally returns compact hits. Request document detail only when
the caller needs metadata or chunks; `get_documents` preserves one result per
requested key.

## Public surface

The public client surface is limited to scope provisioning, text/URL/file
batches, task submit/get/list/items/wait/retry/cancel, compact search, document
detail/batch detail, health, and authentication identity. Low-level REST
handles and `authorized_request` are not part of this SDK.
