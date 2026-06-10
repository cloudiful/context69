# context69-sdk

Async Rust SDK for the Context69 HTTP API.

## Usage

```rust
use context69_sdk::{Context69Client, contracts::SearchRequest};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = Context69Client::builder()
        .base_url("http://127.0.0.1:8096")?
        .build()?;

    client.login("admin", "secret").await?;

    let results = client.search(SearchRequest {
        query: "incident response".to_string(),
        limit: 8,
        source_key: None,
        group_key: None,
        project_key: None,
        published_after: None,
        published_before: None,
    }).await?;

    println!("hits={}", results.hits.len());
    Ok(())
}
```

The SDK stores the refresh cookie internally, injects the Bearer access token automatically,
and retries one time with `refresh()` when a protected request returns `401 Unauthorized`.

## Manual Access Token

```rust
use context69_sdk::Context69Client;

let client = Context69Client::builder()
    .base_url("http://127.0.0.1:8096")?
    .with_access_token("your-access-token")
    .build()?;

let scoped = client.with_access_token("another-access-token");
let groups = scoped.list_groups().await?;
println!("groups={}", groups.len());
```

## Create Project Library Text

```rust
use context69_sdk::{
    Context69Client,
    contracts::CreateTextRequest,
};

let client = Context69Client::builder()
    .base_url("http://127.0.0.1:8096")?
    .with_access_token("your-access-token")
    .build()?;

let response = client
    .create_project_library_text(
        "public",
        "default-public",
        &CreateTextRequest {
            folder_id: None,
            title: "Runbook".to_string(),
            content: "step 1".to_string(),
            source_uri: Some("https://example.test/runbook".to_string()),
            summary: Some("Ops notes".to_string()),
        },
    )
    .await?;

println!("files={}", response.files.len());
```

## Upsert Project Library Text And Sync Source

```rust
use chrono::NaiveDate;
use context69_sdk::{
    Context69Client,
    contracts::UpsertLibraryTextRequest,
};
use serde_json::json;

let client = Context69Client::builder()
    .base_url("http://127.0.0.1:8096")?
    .with_access_token("your-access-token")
    .build()?;

let upload = client
    .upsert_project_library_text(
        "public",
        "default-public",
        &UpsertLibraryTextRequest {
            external_id: "gov-doc-1".to_string(),
            folder_id: None,
            title: "Government Notice".to_string(),
            content: "full text".to_string(),
            source_uri: Some("https://example.test/gov-doc-1".to_string()),
            summary: Some("Short summary".to_string()),
            published_at: NaiveDate::from_ymd_opt(2026, 6, 10),
            metadata_json: json!({"category":"notice"}),
        },
    )
    .await?;

let sync = client.sync_source("gov_documents").await?;

println!("jobs={} changed={}", upload.jobs.len(), sync.records_changed);
```
