# context69-sdk

Async Rust SDK for the Context69 HTTP API.

Deduplicated uploads can carry typed business metadata:

```rust,no_run
use context69_contracts::LibraryFileUploadMetadata;
use serde_json::json;

# async fn example(client: &context69_sdk::Context69Client) -> Result<(), Box<dyn std::error::Error>> {
client.group("research/news").library().files()
    .upload_bytes_deduplicated_with_metadata(
        None,
        "announcement.pdf",
        "application/pdf",
        std::fs::read("announcement.pdf")?,
        Some(LibraryFileUploadMetadata {
            external_id: Some("announcement-42".into()),
            source_uri: Some("https://example.test/announcements/42".into()),
            published_at: Some("2026-07-12T09:30:00Z".parse()?),
            metadata_json: json!({"ticker": "ACME", "score": 10}),
        }),
    ).await?;
# Ok(())
# }
```

The existing `upload_bytes_deduplicated` method sends no metadata, preserving stored business fields on a deduplication hit.

For public HTTPS file URLs, let Context69 download and ingest asynchronously:

```rust,no_run
use context69_sdk::contracts::{
    ImportLibraryFileFromUrlRequest, LibraryFileUploadMetadata, LibraryUrlImportStatus,
};
use serde_json::json;

# async fn example(client: &context69_sdk::Context69Client) -> Result<(), Box<dyn std::error::Error>> {
let library = client.group("research/news").library();
let job = library.files().import_url(&ImportLibraryFileFromUrlRequest {
    url: "https://files.example.com/announcement.pdf".into(),
    folder_id: None,
    filename: None,
    media_type: None,
    metadata: Some(LibraryFileUploadMetadata {
        external_id: Some("announcement-42".into()),
        source_uri: None,
        published_at: Some("2026-07-12T09:30:00Z".parse()?),
        metadata_json: json!({"ticker": "ACME"}),
    }),
}).await?;

let current = library.url_import_job(job.import_job_id).get().await?;
if current.status == LibraryUrlImportStatus::Failed {
    library.url_import_job(job.import_job_id).retry().await?;
}
# Ok(())
# }
```

Only direct public HTTPS files are accepted. Private-network targets, authenticated downloads, and HTML attachment discovery are intentionally rejected.

`context69-sdk` is a PAT-only client. Initialize it with a personal access token that starts with `ctx_pat_`, then navigate the resource tree to call an API.

## Initialization

```rust
use context69_sdk::Context69Client;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = Context69Client::builder()
        .base_url("http://127.0.0.1:8096")?
        .with_personal_access_token("ctx_pat_example_token")?
        .build()?;

    let me = client.me().await?;
    println!("hello {}", me.user.login_name);
    Ok(())
}
```

The builder rejects empty tokens and non-PAT tokens. Protected APIs return `Error::AuthenticationRequired` if no PAT is configured.

## Groups and nested resources

```rust
use context69_sdk::{
    Context69Client,
    contracts::{CreateGroupRequest, GroupKind, Visibility},
};

# async fn example(client: Context69Client) -> Result<(), context69_sdk::Error> {
client.groups().create(&CreateGroupRequest {
    parent_group_path: None,
    group_key: "ops".to_string(),
    name: "Operations".to_string(),
    visibility: Visibility::Private,
    kind: Some(GroupKind::Shared),
}).await?;

let group = client.group("ops").get().await?;
let members = client.group("ops").members().list().await?;
let tree = client.group("ops").library().tree().await?;
# Ok(())
# }
```

Group handles own the group path, so nested calls do not repeat it:

```rust
# use context69_sdk::Context69Client;
# async fn example(client: Context69Client, folder_id: uuid::Uuid, file_id: uuid::Uuid) -> Result<(), context69_sdk::Error> {
let group = client.group("ops/runbooks");
group.source_folder(folder_id).sync().await?;
let file = group.library().file(file_id).get().await?;
# Ok(())
# }
```

## Sources

```rust
# use context69_sdk::{Context69Client, contracts::SourceConfigInput};
# async fn example(client: Context69Client, request: SourceConfigInput) -> Result<(), context69_sdk::Error> {
client.sources().create(&request).await?;
client.source(&request.source_key).sync().await?;

let connections = client.source_connections().list().await?;
client.source_connection("warehouse").delete().await?;
# Ok(())
# }
```

## Library

```rust
use context69_sdk::{
    Context69Client,
    contracts::{CreateTextRequest, LibraryTextContentFormat},
};

# async fn example(client: Context69Client) -> Result<(), context69_sdk::Error> {
let upload = client.library().texts().create(&CreateTextRequest {
    folder_id: None,
    title: "Runbook".to_string(),
    content: "# Step 1".to_string(),
    content_format: LibraryTextContentFormat::Markdown,
    source_uri: None,
    summary: None,
}).await?;

let tree = client.group("ops/runbooks").library().tree().await?;
# Ok(())
# }
```

Multipart uploads use `reqwest::multipart::Part`:

```rust
# use context69_sdk::Context69Client;
use reqwest::multipart::Part;

# async fn example(client: Context69Client) -> Result<(), context69_sdk::Error> {
client.library().files().upload(
    None,
    vec![Part::text("# Notes").file_name("notes.md")],
).await?;
# Ok(())
# }
```

## Settings and search

```rust
# use context69_sdk::{Context69Client, contracts::SearchRequest};
# async fn example(client: Context69Client, request: SearchRequest) -> Result<(), context69_sdk::Error> {
let runtime = client.settings().runtime().get().await?;
let docling = client.settings().docling().get().await?;
let result = client.search().execute(&request).await?;
let document = client.document(result.hits[0].document_id).get().await?;
# Ok(())
# }
```

## Resource tree

- `client.user_directory()`: user search
- `client.groups()` / `client.group(path)`: groups, children, members
- `client.sources()` / `client.source(key)`: source collection and item operations
- `client.source_connections()` / `client.source_connection(name)`: connection operations
- `client.library()`: personal library folders, texts, files, and jobs
- `client.group(path).library()`: group-scoped library resources
- `client.group(path).source_folders()` / `source_folder(id)`: group source folders
- `client.settings()`: runtime, Docling, and search settings
- `client.search()` / `client.document(id)`: search and document lookup
- root client: `me()` and `healthz()`

## PAT scopes

- `workspace`: user directory, groups, memberships
- `sources`: source connections, sources, group source folders, sync
- `library`: personal and group library resources
- `settings`: runtime, Docling, search settings
- `search`: search and document APIs

## Migrating from 0.4

Version 0.5 removes the old grouped service methods. Common replacements:

```text
client.workspace().list_groups()                         -> client.groups().list()
client.workspace().get_group(path)                      -> client.group(path).get()
client.sources().sync_source(key)                       -> client.source(key).sync()
client.sources().sync_group_source_folder(path, id)     -> client.group(path).source_folder(id).sync()
client.library().get_library_tree()                     -> client.library().tree()
client.library().get_group_library_file(path, id)       -> client.group(path).library().file(id).get()
client.settings().get_docling_settings()                -> client.settings().docling().get()
client.search().search(request)                         -> client.search().execute(&request)
client.search().get_document(id)                        -> client.document(id).get()
```

Browser login, session logout, PAT management, and admin-user APIs remain intentionally excluded. HTTP `401 Unauthorized` responses are returned directly; the SDK does not use browser session cookies.
