# context69-sdk

Async Rust SDK for the Context69 HTTP API.

`context69-sdk` is a PAT-only client. Initialize it with a personal access token that starts with `ctx_pat_`, then call the scoped HTTP APIs directly.

## PAT-only initialization

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

## Workspace example

```rust
use context69_sdk::{
    Context69Client,
    contracts::{CreateGroupRequest, GroupKind, Visibility},
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = Context69Client::builder()
        .base_url("http://127.0.0.1:8096")?
        .with_personal_access_token("ctx_pat_example_token")?
        .build()?;

    client
        .workspace()
        .create_group(&CreateGroupRequest {
            parent_group_path: None,
            group_key: "ops".to_string(),
            name: "Operations".to_string(),
            visibility: Visibility::Private,
            kind: Some(GroupKind::Shared),
        })
        .await?;

    client
        .workspace()
        .create_group(&CreateGroupRequest {
            parent_group_path: Some("ops".to_string()),
            group_key: "runbooks".to_string(),
            name: "Runbooks".to_string(),
            visibility: Visibility::Private,
            kind: Some(GroupKind::Shared),
        })
        .await?;

    Ok(())
}
```

## Source and library example

```rust
use chrono::NaiveDate;
use context69_sdk::{
    Context69Client,
    contracts::{
        CreateSourceFolderRequest, CreateTextRequest, LibraryTextContentFormat, SourceConfigInput,
        UpsertLibraryTextRequest,
    },
};
use serde_json::json;
use uuid::Uuid;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = Context69Client::builder()
        .base_url("http://127.0.0.1:8096")?
        .with_personal_access_token("ctx_pat_example_token")?
        .build()?;

    client
        .sources()
        .create_group_source_folder(
            "ops/runbooks",
            &CreateSourceFolderRequest {
                parent_folder_id: None,
                folder_name: "alerts".to_string(),
                source_config: SourceConfigInput {
                    source_key: "alerts".to_string(),
                    display_name: Some("Alerts".to_string()),
                    description: Some("Operational alerts".to_string()),
                    example_queries: vec!["recent paging incidents".to_string()],
                    connection: "warehouse".to_string(),
                    database_url: None,
                    sync_strategy: "incremental".to_string(),
                    connector_type: "postgres_sql".to_string(),
                    base_query: "select * from alerts".to_string(),
                    batch_size: 500,
                    visibility: None,
                },
            },
        )
        .await?;

    client
        .library()
        .create_group_library_text("ops/runbooks", &CreateTextRequest {
            folder_id: None,
            title: "Runbook".to_string(),
            content: "# Step 1\n\nFollow the checklist.".to_string(),
            content_format: LibraryTextContentFormat::Markdown,
            source_uri: Some("https://example.test/runbooks/ops".to_string()),
            summary: Some("Ops reference".to_string()),
        })
        .await?;

    client
        .library()
        .upsert_group_library_text(
            "ops/runbooks",
            &UpsertLibraryTextRequest {
                external_id: "incident-42".to_string(),
                folder_id: None,
                title: "Incident 42".to_string(),
                content: "full text".to_string(),
                content_format: LibraryTextContentFormat::PlainText,
                source_uri: Some("https://example.test/incidents/42".to_string()),
                summary: Some("Postmortem".to_string()),
                published_at: NaiveDate::from_ymd_opt(2026, 7, 1),
                metadata_json: json!({"kind":"postmortem"}),
            },
        )
        .await?;

    client
        .sources()
        .sync_group_source_folder("ops/runbooks", Uuid::parse_str("aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa")?)
        .await?;

    // Multipart upload already supports Markdown files as long as the uploaded
    // part uses a `.md` filename or `text/markdown` content type.

    Ok(())
}
```

## Settings example

```rust
use context69_sdk::Context69Client;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = Context69Client::builder()
        .base_url("http://127.0.0.1:8096")?
        .with_personal_access_token("ctx_pat_example_token")?
        .build()?;

    let docling = client.settings().get_docling_settings().await?;
    println!("docling configured: {}", docling.configured);
    Ok(())
}
```

## API coverage

- `client.workspace()`: user directory, groups, child groups, group members
- `client.sources()`: source connections, global sources, group-scoped source-folder APIs
- `client.library()`: global tree/folder/file/job APIs, global library text, group library tree/folder/file/job APIs, group library text, multipart uploads
- `client.settings()`: runtime, provider accounts, docling, search settings
- `client.search()`: search and document lookup
- Root client: `me()`, `healthz()`

## Scope requirements

PATs must include the scopes required by the APIs you call:

- `workspace`: user directory, groups, memberships
- `sources`: source connections, global sources, group source-folder APIs, sync
- `library`: global library, group library, library text APIs
- `settings`: runtime settings, provider accounts, docling settings, search settings
- `search`: search and document APIs

## Breaking changes

- `with_access_token()` was replaced by `with_personal_access_token()`
- `login()`, `refresh()`, and `logout()` were removed from the main client
- `401 Unauthorized` responses are returned directly; the SDK no longer retries with refresh-cookie logic
- JWT-only APIs are intentionally excluded from this client, including login/refresh/logout, PAT management, and admin-user APIs
- Flat methods like `client.create_group(...)` were replaced by grouped APIs such as `client.workspace().create_group(...)`
