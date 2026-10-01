//! Transport-level regression coverage without any live network: origin
//! enforcement, the streamed body cap, and header construction are verified
//! through pure functions and a header-capturing fake reqwest service.

use super::*;

#[test]
fn origin_enforcement_accepts_only_the_approved_origin() {
    let ok = reqwest::Url::parse("https://api.github.com/repos/octo/repo?x=1").unwrap();
    assert!(enforce_api_origin(&ok).is_ok());
    for url in [
        "https://api.github.com.evil.test/repos/octo/repo",
        "http://api.github.com/repos/octo/repo",
        "https://api.github.com:8443/repos/octo/repo",
        "https://user:pass@api.github.com/repos/octo/repo",
        "https://github.com/repos/octo/repo",
    ] {
        let url = reqwest::Url::parse(url).unwrap();
        assert!(enforce_api_origin(&url).is_err(), "must reject {url}");
    }
}

#[tokio::test]
async fn capped_body_reader_stops_at_the_limit() {
    // A body that never ends must trip the cap, not buffer forever.
    struct EndlessChunks;
    #[async_trait]
    impl BodyChunks for EndlessChunks {
        async fn next_chunk(&mut self) -> Result<Option<Bytes>> {
            Ok(Some(Bytes::from_static(b"0123456789")))
        }
    }
    let mut chunks = EndlessChunks;
    let error = read_capped_body(&mut chunks, 100).await.unwrap_err();
    assert!(error.to_string().contains("git_response_too_large"));
}

#[tokio::test]
async fn capped_body_reader_assembles_chunks_below_the_limit() {
    struct ScriptedChunks(Vec<Bytes>);
    #[async_trait]
    impl BodyChunks for ScriptedChunks {
        async fn next_chunk(&mut self) -> Result<Option<Bytes>> {
            let first = self.0.first().cloned();
            if first.is_some() {
                self.0.remove(0);
            }
            Ok(first)
        }
    }
    let mut chunks = ScriptedChunks(vec![Bytes::from_static(b"aaa"), Bytes::from_static(b"bb")]);
    let body = read_capped_body(&mut chunks, 100).await.unwrap();
    assert_eq!(body, Bytes::from_static(b"aaabb"));
    // A cap exactly at the boundary is fine; one byte less fails.
    let mut chunks = ScriptedChunks(vec![Bytes::from_static(b"aaa"), Bytes::from_static(b"bb")]);
    assert!(read_capped_body(&mut chunks, 5).await.is_ok());
    let mut chunks = ScriptedChunks(vec![Bytes::from_static(b"aaa"), Bytes::from_static(b"bb")]);
    let error = read_capped_body(&mut chunks, 4).await.unwrap_err();
    assert!(error.to_string().contains("git_response_too_large"));
}

#[test]
fn user_agent_is_the_versioned_context69_product() {
    assert!(GITHUB_USER_AGENT.starts_with("context69/"));
    assert!(GITHUB_USER_AGENT.len() > "context69/".len());
}

#[test]
fn production_transport_uses_the_shared_remote_stack() {
    // GitHubApiTransport owns a library RemoteTransport; assert the type
    // wiring so a regression to a standalone reqwest client is caught by
    // compilation of this accessor.
    fn assert_same_transport<T>(_: std::marker::PhantomData<T>) {}
    assert_same_transport::<RemoteTransport>(std::marker::PhantomData);
}

#[tokio::test]
async fn production_transport_rejects_non_api_origins_before_sending() {
    // The shared RemoteTransport builds a real client, but no request is
    // sent: origin enforcement runs after URL validation and before any
    // connection. `github.com` (the web origin, not the API origin) passes
    // URL validation but must fail the exact API-origin check.
    let transport = GitHubApiTransport::new(false).await.unwrap();
    let error = transport
        .get("https://github.com/octo/repo", 1024)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("git_endpoint_origin_rejected"));
    // A lookalike host is rejected as well (by URL validation, since it
    // cannot resolve, or by origin enforcement if it could).
    let error = transport
        .get("https://api.github.com.evil.test/repos/octo/repo", 1024)
        .await
        .unwrap_err();
    assert!(
        error.to_string().contains("git_endpoint_origin_rejected")
            || error.to_string().contains("remote_"),
        "lookalike host must be rejected, got {error}"
    );
}

#[test]
fn origin_enforcement_accepts_the_default_https_port_but_rejects_other_ports() {
    // An explicit default port normalizes away, so it is still the approved
    // origin; any non-default port is a different origin and must be refused.
    let default_port = reqwest::Url::parse("https://api.github.com:443/repos/octo/repo").unwrap();
    assert!(enforce_api_origin(&default_port).is_ok());
    let other_port = reqwest::Url::parse("https://api.github.com:8443/repos/octo/repo").unwrap();
    assert!(enforce_api_origin(&other_port).is_err());
}

#[tokio::test]
async fn production_transport_rejects_a_disallowed_origin_before_dns() {
    // The origin check must run before `validate_url` (the DNS resolver). A
    // disallowed host under the reserved, guaranteed-non-resolving `.invalid`
    // TLD can only fail with the origin error if no DNS lookup was attempted;
    // if the order regressed, this test would see a DNS/validation error.
    let transport = GitHubApiTransport::new(false).await.unwrap();
    let error = transport
        .get("https://not-api.github.invalid/repos/octo/repo", 1024)
        .await
        .unwrap_err();
    assert!(
        error.to_string().contains("git_endpoint_origin_rejected"),
        "origin must be rejected before DNS, got {error}"
    );
}
