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
    contracts::{CreateGroupRequest, CreateProjectRequest, GroupKind, Visibility},
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = Context69Client::builder()
        .base_url("http://127.0.0.1:8096")?
        .with_personal_access_token("ctx_pat_example_token")?
        .build()?;

    client
        .create_group(&CreateGroupRequest {
            parent_group_key: None,
            group_key: "ops".to_string(),
            name: "Operations".to_string(),
            visibility: Visibility::Private,
            kind: Some(GroupKind::Shared),
        })
        .await?;

    client
        .create_project(
            "ops",
            &CreateProjectRequest {
                project_key: "runbooks".to_string(),
                name: "Runbooks".to_string(),
                visibility: Visibility::Private,
            },
        )
        .await?;

    Ok(())
}
```

## Source and library example

```rust
use chrono::NaiveDate;
use context69_sdk::{
    Context69Client,
    contracts::{CreateTextRequest, SourceConfigInput, UpsertLibraryTextRequest},
};
use serde_json::json;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = Context69Client::builder()
        .base_url("http://127.0.0.1:8096")?
        .with_personal_access_token("ctx_pat_example_token")?
        .build()?;

    client
        .create_project_source(
            "ops",
            "runbooks",
            &SourceConfigInput {
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
        )
        .await?;

    client
        .create_library_text(&CreateTextRequest {
            folder_id: None,
            title: "Global Runbook".to_string(),
            content: "step 1".to_string(),
            source_uri: Some("https://example.test/runbooks/global".to_string()),
            summary: Some("Global reference".to_string()),
        })
        .await?;

    client
        .upsert_project_library_text(
            "ops",
            "runbooks",
            &UpsertLibraryTextRequest {
                external_id: "incident-42".to_string(),
                folder_id: None,
                title: "Incident 42".to_string(),
                content: "full text".to_string(),
                source_uri: Some("https://example.test/incidents/42".to_string()),
                summary: Some("Postmortem".to_string()),
                published_at: NaiveDate::from_ymd_opt(2026, 7, 1),
                metadata_json: json!({"kind":"postmortem"}),
            },
        )
        .await?;

    Ok(())
}
```

## API coverage

- Workspace: user directory, groups, projects, group members, project members
- Sources: source connections, global sources, project sources, sync
- Library: global tree/folder/file/job APIs, global library text, project library tree/folder/file/job APIs, project library text, multipart uploads
- Settings: runtime, provider accounts, docling, search settings
- Search: search, document lookup, `me()`, `healthz()`

## Scope requirements

PATs must include the scopes required by the APIs you call:

- `workspace`: user directory, groups, projects, memberships
- `sources`: source connections, global sources, project sources, sync
- `library`: global library, project library, library text APIs
- `settings`: runtime settings, provider accounts, docling settings, search settings
- `search`: search and document APIs

## Breaking changes

- `with_access_token()` was replaced by `with_personal_access_token()`
- `login()`, `refresh()`, and `logout()` were removed from the main client
- `401 Unauthorized` responses are returned directly; the SDK no longer retries with refresh-cookie logic
- JWT-only APIs are intentionally excluded from this client, including login/refresh/logout, PAT management, and admin-user APIs
