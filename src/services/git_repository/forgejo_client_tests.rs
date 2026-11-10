use std::sync::{Arc, Mutex};

use async_trait::async_trait;

use super::super::github_client::RepositoryAcquirer;
use super::super::http::{GitHttpTransport, TransportResponse};
use super::super::model::GitAcquisitionLimits;
use super::super::provider_url::ForgejoOrigin;
use super::super::ref_path_safety::{SafeRef, SafeSha};
use super::super::support::{encoded, ok_json};
use super::ForgejoAcquirer;
use crate::domain_errors::DomainError;

const SHA: &str = "0123456789abcdef0123456789abcdef01234567";
const BLOB: &str = "89abcdef0123456789abcdef0123456789abcdef";

fn limits(max_entries: usize) -> GitAcquisitionLimits {
    GitAcquisitionLimits {
        max_blob_bytes: 64,
        max_total_bytes: 160,
        max_response_bytes: 4096,
        max_tree_entries: max_entries,
    }
}

struct ScriptedTransport {
    responses: Mutex<Vec<anyhow::Result<TransportResponse>>>,
    requested: Mutex<Vec<String>>,
}

impl ScriptedTransport {
    fn with_responses(responses: Vec<anyhow::Result<TransportResponse>>) -> Self {
        Self {
            responses: Mutex::new(responses),
            requested: Mutex::new(Vec::new()),
        }
    }

    fn requested(&self) -> Vec<String> {
        self.requested.lock().unwrap().clone()
    }
}

#[async_trait]
impl GitHttpTransport for ScriptedTransport {
    async fn get(&self, url: &str, max_body_bytes: usize) -> anyhow::Result<TransportResponse> {
        self.requested.lock().unwrap().push(url.to_owned());
        let mut responses = self.responses.lock().unwrap();
        if responses.is_empty() {
            panic!("script exhausted");
        }
        let response = responses.remove(0)?;
        if response.body.len() > max_body_bytes {
            return Err(DomainError::payload_too_large("git_response_too_large").into());
        }
        Ok(response)
    }
}

fn acquirer(
    responses: Vec<anyhow::Result<TransportResponse>>,
    limits: GitAcquisitionLimits,
) -> ForgejoAcquirer<ScriptedTransport> {
    ForgejoAcquirer::new(
        ForgejoOrigin::from_base_url("https://forgejo.example/api/v1").unwrap(),
        "https://forgejo.example/octo/repo",
        Arc::new(ScriptedTransport::with_responses(responses)),
        limits,
    )
    .unwrap()
}

fn ref_list(sha: &str, kind: &str) -> String {
    format!(r#"[{{"ref":"refs/heads/main","object":{{"type":"{kind}","sha":"{sha}"}}}}]"#)
}

fn tree_page(entries: &[String], truncated: bool, page: u32, total_count: usize) -> String {
    format!(
        r#"{{"sha":"{SHA}","truncated":{truncated},"page":{page},"total_count":{total_count},"tree":[{}]}}"#,
        entries.join(",")
    )
}

fn tree_page_with_sha(
    sha: &str,
    entries: &[String],
    truncated: bool,
    page: u32,
    total_count: usize,
) -> String {
    format!(
        r#"{{"sha":"{sha}","truncated":{truncated},"page":{page},"total_count":{total_count},"tree":[{}]}}"#,
        entries.join(",")
    )
}

fn blob_entry(path: &str, sha: &str, size: u64) -> String {
    format!(r#"{{"type":"blob","path":"{path}","sha":"{sha}","size":{size}}}"#)
}

#[tokio::test]
async fn resolves_the_exact_ref_from_the_provider_list() {
    let transport = Arc::new(ScriptedTransport::with_responses(vec![ok_json(ref_list(
        SHA, "commit",
    ))]));
    let client = ForgejoAcquirer::new(
        ForgejoOrigin::from_base_url("https://forgejo.example/api/v1").unwrap(),
        "https://forgejo.example/octo/repo",
        transport.clone(),
        limits(4),
    )
    .unwrap();
    let sha = client
        .resolve_ref(&SafeRef::parse("refs/heads/main").unwrap())
        .await
        .unwrap();
    assert_eq!(sha.as_str(), SHA);
    assert!(
        transport.requested()[0].ends_with("/git/refs/heads/main"),
        "Forgejo matches the slash-separated short ref: {}",
        transport.requested()[0]
    );
}

#[tokio::test]
async fn follows_truncated_tree_pages_and_accumulates_entries() {
    let transport = Arc::new(ScriptedTransport::with_responses(vec![
        ok_json(tree_page(&[blob_entry("README.md", BLOB, 5)], true, 1, 2)),
        ok_json(tree_page(
            &[blob_entry("src/main.rs", BLOB, 13)],
            true,
            2,
            2,
        )),
    ]));
    let client = ForgejoAcquirer::new(
        ForgejoOrigin::from_base_url("https://forgejo.example").unwrap(),
        "https://forgejo.example/octo/repo",
        transport.clone(),
        limits(4),
    )
    .unwrap();
    let listing = client
        .list_tree(&SafeSha::parse(SHA).unwrap())
        .await
        .unwrap();
    assert_eq!(
        listing
            .files
            .iter()
            .map(|file| file.path.as_str())
            .collect::<Vec<_>>(),
        vec!["README.md", "src/main.rs"]
    );
    let requested = transport.requested();
    assert_eq!(
        requested.len(),
        2,
        "a complete final page marked truncated stops pagination"
    );
    assert!(requested[0].contains("page=1"), "{}", requested[0]);
    assert!(requested[1].contains("page=2"), "{}", requested[1]);
}

#[tokio::test]
async fn a_server_capped_page_size_still_completes() {
    let transport = Arc::new(ScriptedTransport::with_responses(vec![
        ok_json(tree_page(
            &[
                blob_entry("README.md", BLOB, 5),
                blob_entry("src/main.rs", BLOB, 13),
            ],
            true,
            1,
            4,
        )),
        ok_json(tree_page(
            &[
                blob_entry("src/lib.rs", BLOB, 21),
                blob_entry("src/model.rs", BLOB, 34),
            ],
            true,
            2,
            4,
        )),
    ]));
    let client = ForgejoAcquirer::new(
        ForgejoOrigin::from_base_url("https://forgejo.example").unwrap(),
        "https://forgejo.example/octo/repo",
        transport.clone(),
        limits(4),
    )
    .unwrap();
    let listing = client
        .list_tree(&SafeSha::parse(SHA).unwrap())
        .await
        .unwrap();
    assert_eq!(listing.files.len(), 4);
    assert_eq!(
        transport.requested().len(),
        2,
        "the observed server cap, not the requested page size, bounds pagination"
    );
}

#[tokio::test]
async fn an_unfinished_listing_fails_closed_at_the_page_budget() {
    // The first page establishes a five-entry server cap, so ten declared
    // entries need at most three pages. Later pages that keep adding only one
    // entry cannot prove completion within that observed bound.
    let client = acquirer(
        vec![
            ok_json(tree_page(
                &[
                    blob_entry("a.rs", BLOB, 1),
                    blob_entry("b.rs", BLOB, 1),
                    blob_entry("c.rs", BLOB, 1),
                    blob_entry("d.rs", BLOB, 1),
                    blob_entry("e.rs", BLOB, 1),
                ],
                true,
                1,
                10,
            )),
            ok_json(tree_page(&[blob_entry("f.rs", BLOB, 1)], true, 2, 10)),
            ok_json(tree_page(&[blob_entry("g.rs", BLOB, 1)], true, 3, 10)),
        ],
        limits(10),
    );
    let error = client
        .list_tree(&SafeSha::parse(SHA).unwrap())
        .await
        .unwrap_err();
    assert!(error.to_string().contains("git_tree_truncated"), "{error}");
}

#[tokio::test]
async fn the_entry_budget_applies_across_pages() {
    let client = acquirer(
        vec![ok_json(tree_page(
            &[
                blob_entry("a.rs", BLOB, 1),
                blob_entry("b.rs", BLOB, 1),
                blob_entry("c.rs", BLOB, 1),
            ],
            false,
            1,
            3,
        ))],
        limits(2),
    );
    let error = client
        .list_tree(&SafeSha::parse(SHA).unwrap())
        .await
        .unwrap_err();
    assert!(error.to_string().contains("git_tree_too_large"), "{error}");
}

#[tokio::test]
async fn a_malformed_tree_identity_is_rejected() {
    let body = r#"{"sha":"not-a-sha","truncated":false,"total_count":0,"tree":[]}"#;
    let client = acquirer(vec![ok_json(body.to_string())], limits(4));
    let error = client
        .list_tree(&SafeSha::parse(SHA).unwrap())
        .await
        .unwrap_err();
    assert!(error.to_string().contains("git_sha_malformed"), "{error}");
}

#[tokio::test]
async fn the_provider_tree_sha_is_the_tree_object_not_the_commit() {
    // Forgejo reports the tree object's own SHA; the pinned commit stays the
    // identity the listing is recorded under.
    let tree_sha = "ffffffffffffffffffffffffffffffffffffffff";
    let body = format!(r#"{{"sha":"{tree_sha}","truncated":false,"total_count":0,"tree":[]}}"#);
    let client = acquirer(vec![ok_json(body)], limits(4));
    let listing = client
        .list_tree(&SafeSha::parse(SHA).unwrap())
        .await
        .unwrap();
    assert_eq!(listing.commit_sha.as_str(), SHA);
    assert!(listing.files.is_empty());
}

#[tokio::test]
async fn a_tree_without_required_fields_is_rejected() {
    for body in [
        format!(r#"{{"sha":"{SHA}","truncated":false,"total_count":0}}"#),
        format!(r#"{{"sha":"{SHA}","total_count":0,"tree":[]}}"#),
        format!(r#"{{"sha":"{SHA}","truncated":false,"tree":[]}}"#),
    ] {
        let client = acquirer(vec![ok_json(body)], limits(4));
        let error = client
            .list_tree(&SafeSha::parse(SHA).unwrap())
            .await
            .unwrap_err();
        assert!(
            error.to_string().contains("git_response_malformed"),
            "{error}"
        );
    }
}

#[tokio::test]
async fn an_empty_tree_may_be_serialized_as_null() {
    let body = format!(r#"{{"sha":"{SHA}","truncated":false,"total_count":0,"tree":null}}"#);
    let client = acquirer(vec![ok_json(body)], limits(4));
    let listing = client
        .list_tree(&SafeSha::parse(SHA).unwrap())
        .await
        .unwrap();
    assert!(listing.files.is_empty());
}

#[tokio::test]
async fn a_total_count_change_is_rejected() {
    let client = acquirer(
        vec![
            ok_json(tree_page(&[blob_entry("README.md", BLOB, 5)], true, 1, 2)),
            ok_json(tree_page(
                &[blob_entry("src/main.rs", BLOB, 13)],
                true,
                2,
                3,
            )),
        ],
        limits(4),
    );
    let error = client
        .list_tree(&SafeSha::parse(SHA).unwrap())
        .await
        .unwrap_err();
    assert!(
        error.to_string().contains("git_tree_total_mismatch"),
        "{error}"
    );
}

#[tokio::test]
async fn a_partial_final_page_is_rejected() {
    let client = acquirer(
        vec![ok_json(tree_page(
            &[blob_entry("README.md", BLOB, 5)],
            false,
            1,
            2,
        ))],
        limits(4),
    );
    let error = client
        .list_tree(&SafeSha::parse(SHA).unwrap())
        .await
        .unwrap_err();
    assert!(error.to_string().contains("git_tree_incomplete"), "{error}");
}

#[tokio::test]
async fn a_later_page_with_a_different_tree_sha_is_rejected() {
    let other_sha = "ffffffffffffffffffffffffffffffffffffffff";
    let client = acquirer(
        vec![
            ok_json(tree_page_with_sha(
                SHA,
                &[blob_entry("README.md", BLOB, 5)],
                true,
                1,
                2,
            )),
            ok_json(tree_page_with_sha(
                other_sha,
                &[blob_entry("src/main.rs", BLOB, 13)],
                false,
                2,
                2,
            )),
        ],
        limits(4),
    );
    let error = client
        .list_tree(&SafeSha::parse(SHA).unwrap())
        .await
        .unwrap_err();
    assert!(
        error.to_string().contains("git_tree_sha_mismatch"),
        "{error}"
    );
}

#[tokio::test]
async fn a_repeated_page_without_full_coverage_is_rejected() {
    let entry = blob_entry("README.md", BLOB, 5);
    let client = acquirer(
        vec![
            ok_json(tree_page(std::slice::from_ref(&entry), true, 1, 2)),
            ok_json(tree_page(std::slice::from_ref(&entry), false, 2, 2)),
        ],
        limits(4),
    );
    let error = client
        .list_tree(&SafeSha::parse(SHA).unwrap())
        .await
        .unwrap_err();
    assert!(error.to_string().contains("git_tree_incomplete"), "{error}");
}

#[tokio::test]
async fn an_entry_conflicting_with_a_seen_path_is_rejected() {
    let other_blob = "1111111111111111111111111111111111111111";
    let client = acquirer(
        vec![
            ok_json(tree_page(&[blob_entry("README.md", BLOB, 5)], true, 1, 2)),
            ok_json(tree_page(
                &[blob_entry("README.md", other_blob, 5)],
                false,
                2,
                2,
            )),
        ],
        limits(4),
    );
    let error = client
        .list_tree(&SafeSha::parse(SHA).unwrap())
        .await
        .unwrap_err();
    assert!(
        error.to_string().contains("git_tree_duplicate_entry"),
        "{error}"
    );
}

#[tokio::test]
async fn an_oversized_entry_carries_its_too_large_reason() {
    let client = acquirer(
        vec![ok_json(tree_page(
            &[blob_entry("huge.bin", BLOB, 100)],
            false,
            1,
            1,
        ))],
        limits(4),
    );
    let listing = client
        .list_tree(&SafeSha::parse(SHA).unwrap())
        .await
        .unwrap();
    assert_eq!(listing.excluded_files, 1);
    assert_eq!(listing.too_large_files, 1);
}

#[tokio::test]
async fn fetches_and_decodes_a_blob() {
    let client = acquirer(
        vec![ok_json(format!(
            r#"{{"sha":"{BLOB}","size":13,"content":"{}","encoding":"base64"}}"#,
            encoded(b"fn main() {}\n")
        ))],
        limits(4),
    );
    let blob = client
        .fetch_blob(&SafeSha::parse(BLOB).unwrap())
        .await
        .unwrap();
    assert_eq!(blob.bytes.as_ref(), b"fn main() {}\n");
}
