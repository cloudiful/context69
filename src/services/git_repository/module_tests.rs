//! Module-level flow tests for the acquisition foundation (issue #681
//! phase 3A): a complete resolve → tree → blob walk driven by a scripted
//! transport, plus the provider-neutral trait surface consumed downstream.

use std::sync::Arc;
use std::sync::Mutex;

use async_trait::async_trait;

use super::github_client::{GitHubAcquirer, RepositoryAcquirer};
use super::github_transport::{GitHttpTransport, TransportResponse};
use super::model::GitAcquisitionLimits;
use super::ref_path_safety::SafeRef;
use bytes::Bytes;

/// Scripted transport pairing requested URLs with canned responses.
struct FlowTransport {
    script: Mutex<Vec<(String, TransportResponse)>>,
}

impl FlowTransport {
    fn new(script: Vec<(&str, u16, String)>) -> Self {
        Self {
            script: Mutex::new(
                script
                    .into_iter()
                    .map(|(url, status, body)| {
                        (
                            url.to_owned(),
                            TransportResponse {
                                status,
                                headers: vec![(
                                    "content-type".to_owned(),
                                    "application/json".to_owned(),
                                )],
                                body: body.into_bytes().into(),
                            },
                        )
                    })
                    .collect(),
            ),
        }
    }
}

#[async_trait]
impl GitHttpTransport for FlowTransport {
    async fn get(&self, url: &str, _max_body_bytes: usize) -> anyhow::Result<TransportResponse> {
        let mut script = self.script.lock().unwrap();
        let index = script
            .iter()
            .position(|(pattern, _)| url == pattern.as_str())
            .ok_or_else(|| {
                anyhow::anyhow!(crate::domain_errors::DomainError::internal(format!(
                    "unexpected acquisition request: {url}"
                )))
            })?;
        let (_, response) = script.remove(index);
        Ok(response)
    }
}

const COMMIT_SHA: &str = "1111111111111111111111111111111111111111";
const BLOB_SHA: &str = "2222222222222222222222222222222222222222";

#[tokio::test]
async fn full_flow_walks_ref_tree_and_blob_in_order() {
    let payload = b"# hello\n";
    let encoded = {
        use base64::Engine;
        base64::engine::general_purpose::STANDARD.encode(payload)
    };
    let transport = Arc::new(FlowTransport::new(vec![
        (
            "https://api.github.com/repos/octo/repo/git/ref/heads%2Fmain",
            200,
            format!(
                r#"{{"ref":"refs/heads/main","object":{{"type":"commit","sha":"{COMMIT_SHA}"}}}}"#
            ),
        ),
        (
            &format!("https://api.github.com/repos/octo/repo/git/trees/{COMMIT_SHA}?recursive=1"),
            200,
            format!(
                r#"{{"sha":"{COMMIT_SHA}","truncated":false,"tree":[{{"type":"blob","path":"README.md","sha":"{BLOB_SHA}","size":8}}]}}"#
            ),
        ),
        (
            &format!("https://api.github.com/repos/octo/repo/git/blobs/{BLOB_SHA}"),
            200,
            format!(r#"{{"sha":"{BLOB_SHA}","size":8,"content":"{encoded}","encoding":"base64"}}"#),
        ),
    ]));
    let acquirer = GitHubAcquirer::new(
        "https://github.com/octo/repo",
        transport,
        GitAcquisitionLimits::default(),
    )
    .unwrap();
    assert_eq!(acquirer.coordinates().owner, "octo");

    let acquirer: Box<dyn RepositoryAcquirer> = Box::new(acquirer);
    let commit = acquirer
        .resolve_ref(&SafeRef::parse("refs/heads/main").unwrap())
        .await
        .unwrap();
    assert_eq!(commit.as_str(), COMMIT_SHA);
    let listing = acquirer.list_tree(&commit).await.unwrap();
    assert_eq!(listing.files.len(), 1);
    // The listing is pinned to the requested commit identity, not a tree id.
    assert_eq!(listing.commit_sha, commit);
    let blob = acquirer
        .fetch_blob(&listing.files[0].blob_sha)
        .await
        .unwrap();
    assert_eq!(blob.bytes, Bytes::from_static(payload));
}

#[tokio::test]
async fn facade_head_flow_pins_the_default_branch_commit_and_blob() {
    // The whole 3A surface through the `Box<dyn RepositoryAcquirer>` facade:
    // HEAD resolves through repository metadata to the default branch commit,
    // the listing is pinned to that commit, and the blob decodes. The scripted
    // transport fails on any unexpected URL, so the request order is asserted.
    const DEFAULT_SHA: &str = "3333333333333333333333333333333333333333";
    const BLOB_SHA: &str = "4444444444444444444444444444444444444444";
    let payload = b"fn main() {}\n";
    let encoded = {
        use base64::Engine;
        base64::engine::general_purpose::STANDARD.encode(payload)
    };
    let transport = Arc::new(FlowTransport::new(vec![
        (
            "https://api.github.com/repos/octo/repo",
            200,
            r#"{"default_branch":"main"}"#.to_string(),
        ),
        (
            "https://api.github.com/repos/octo/repo/git/ref/heads%2Fmain",
            200,
            format!(
                r#"{{"ref":"refs/heads/main","object":{{"type":"commit","sha":"{DEFAULT_SHA}"}}}}"#
            ),
        ),
        (
            &format!("https://api.github.com/repos/octo/repo/git/trees/{DEFAULT_SHA}?recursive=1"),
            200,
            format!(
                r#"{{"sha":"{DEFAULT_SHA}","truncated":false,"tree":[{{"type":"blob","path":"src/main.rs","sha":"{BLOB_SHA}","size":{}}}]}}"#,
                payload.len()
            ),
        ),
        (
            &format!("https://api.github.com/repos/octo/repo/git/blobs/{BLOB_SHA}"),
            200,
            format!(
                r#"{{"sha":"{BLOB_SHA}","size":{},"content":"{encoded}","encoding":"base64"}}"#,
                payload.len()
            ),
        ),
    ]));
    let acquirer = GitHubAcquirer::new(
        "https://github.com/octo/repo",
        transport,
        GitAcquisitionLimits::default(),
    )
    .unwrap();

    let acquirer: Box<dyn RepositoryAcquirer> = Box::new(acquirer);
    let commit = acquirer
        .resolve_ref(&SafeRef::parse("HEAD").unwrap())
        .await
        .unwrap();
    assert_eq!(commit.as_str(), DEFAULT_SHA);
    let listing = acquirer.list_tree(&commit).await.unwrap();
    assert_eq!(listing.commit_sha, commit);
    assert_eq!(listing.files.len(), 1);
    assert_eq!(listing.files[0].path.as_str(), "src/main.rs");
    let blob = acquirer
        .fetch_blob(&listing.files[0].blob_sha)
        .await
        .unwrap();
    assert_eq!(blob.bytes.as_ref(), payload);
}
