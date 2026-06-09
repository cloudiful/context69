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
