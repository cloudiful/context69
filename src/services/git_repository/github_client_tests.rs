use super::super::ref_path_safety::{SafeRef, SafeSha};
use super::super::resolve::MAX_TAG_DEREF_DEPTH;
use super::{
    GitAcquisitionLimits, GitHttpTransport, GitHubAcquirer, RepositoryAcquirer, TransportResponse,
};
use crate::domain_errors::DomainError;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use bytes::Bytes;

const SHA: &str = "0123456789abcdef0123456789abcdef01234567";

fn limits() -> GitAcquisitionLimits {
    GitAcquisitionLimits {
        max_blob_bytes: 64,
        max_total_bytes: 160,
        max_response_bytes: 4096,
        max_tree_entries: 4,
    }
}

fn ref_body(sha: &str, kind: &str) -> String {
    format!(r#"{{"ref":"refs/heads/main","object":{{"type":"{kind}","sha":"{sha}"}}}}"#)
}

fn tree_body(entries: &[String], truncated: bool) -> String {
    format!(
        r#"{{"sha":"{SHA}","truncated":{truncated},"tree":[{}]}}"#,
        entries.join(",")
    )
}

fn blob_entry(path: &str, sha: &str, size: u64) -> String {
    format!(r#"{{"type":"blob","path":"{path}","sha":"{sha}","size":{size}}}"#)
}

fn dir_entry(path: &str, sha: &str) -> String {
    format!(r#"{{"type":"tree","path":"{path}","sha":"{sha}","size":0}}"#)
}

fn blob_body(content: &str, size: u64, sha: &str) -> String {
    format!(r#"{{"sha":"{sha}","size":{size},"content":"{content}","encoding":"base64"}}"#)
}

/// Scripted transport: one canned response per call, plus a record of every
/// requested URL. Never touches the network.
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
        // Mirror the production transport contract: the byte cap is applied
        // inside the transport, before any parser sees the body.
        if response.body.len() > max_body_bytes {
            return Err(DomainError::payload_too_large("git_response_too_large").into());
        }
        Ok(response)
    }
}

fn encoded(content: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(content)
}

fn acquirer(
    responses: Vec<anyhow::Result<TransportResponse>>,
    limits: GitAcquisitionLimits,
) -> GitHubAcquirer<ScriptedTransport> {
    GitHubAcquirer::new(
        "https://github.com/octo/repo",
        Arc::new(ScriptedTransport::with_responses(responses)),
        limits,
    )
    .unwrap()
}

fn ok_json(body: String) -> anyhow::Result<TransportResponse> {
    Ok(TransportResponse {
        status: 200,
        headers: vec![("content-type".into(), "application/json".into())],
        body: body.into_bytes().into(),
    })
}

#[tokio::test]
async fn resolves_ref_to_commit_sha() {
    let client = acquirer(vec![ok_json(ref_body(SHA, "commit"))], limits());
    let sha = client
        .resolve_ref(&SafeRef::parse("refs/heads/main").unwrap())
        .await
        .unwrap();
    assert_eq!(sha.as_str(), SHA);
}

#[tokio::test]
async fn ref_request_uses_the_provider_short_ref_form() {
    // GitHub's `git/ref/{ref}` endpoint 404s on the `refs/`-prefixed form;
    // the request must carry the stripped path (`heads/main`).
    let transport = Arc::new(ScriptedTransport::with_responses(vec![ok_json(ref_body(
        SHA, "commit",
    ))]));
    let client =
        GitHubAcquirer::new("https://github.com/octo/repo", transport.clone(), limits()).unwrap();
    client
        .resolve_ref(&SafeRef::parse("refs/heads/main").unwrap())
        .await
        .unwrap();
    let requested = transport.requested.lock().unwrap().clone();
    assert_eq!(
        requested.first().map(String::as_str),
        Some("https://api.github.com/repos/octo/repo/git/ref/heads%2Fmain"),
        "the refs/ prefix must be stripped for the provider"
    );
}

#[tokio::test]
async fn head_resolves_through_the_metadata_default_branch() {
    // HEAD must resolve via the repository's `default_branch` metadata, not
    // a positional branch listing: the metadata says `trunk`, so exactly the
    // `heads/trunk` ref is requested and its commit is returned even when
    // other branches would resolve elsewhere.
    const TRUNK_SHA: &str = "3333333333333333333333333333333333333333";
    let metadata = r#"{"name":"repo","default_branch":"trunk"}"#;
    let trunk_ref =
        format!(r#"{{"ref":"refs/heads/trunk","object":{{"type":"commit","sha":"{TRUNK_SHA}"}}}}"#);
    let transport = Arc::new(ScriptedTransport::with_responses(vec![
        ok_json(metadata.into()),
        ok_json(trunk_ref),
    ]));
    let client =
        GitHubAcquirer::new("https://github.com/octo/repo", transport.clone(), limits()).unwrap();
    let sha = client
        .resolve_ref(&SafeRef::parse("HEAD").unwrap())
        .await
        .unwrap();
    assert_eq!(sha.as_str(), TRUNK_SHA);
    assert_ne!(
        sha.as_str(),
        SHA,
        "resolution must follow default_branch metadata, not a branch listing"
    );
    let requested = transport.requested.lock().unwrap().clone();
    assert_eq!(
        requested.as_slice(),
        [
            "https://api.github.com/repos/octo/repo",
            "https://api.github.com/repos/octo/repo/git/ref/heads%2Ftrunk",
        ],
        "HEAD must consult metadata, then the default branch ref only"
    );
}

#[tokio::test]
async fn head_fails_closed_on_missing_or_invalid_default_branch_metadata() {
    // Missing metadata field.
    let transport = Arc::new(ScriptedTransport::with_responses(vec![ok_json(
        r#"{"name":"repo"}"#.to_string(),
    )]));
    let client = GitHubAcquirer::new("https://github.com/octo/repo", transport, limits()).unwrap();
    assert!(
        client
            .resolve_ref(&SafeRef::parse("HEAD").unwrap())
            .await
            .is_err()
    );

    // Empty branch name.
    let transport = Arc::new(ScriptedTransport::with_responses(vec![ok_json(
        r#"{"name":"repo","default_branch":""}"#.to_string(),
    )]));
    let client = GitHubAcquirer::new("https://github.com/octo/repo", transport, limits()).unwrap();
    assert!(
        client
            .resolve_ref(&SafeRef::parse("HEAD").unwrap())
            .await
            .is_err()
    );

    // Traversal-shaped branch name never reaches a second request.
    let transport = Arc::new(ScriptedTransport::with_responses(vec![ok_json(
        r#"{"name":"repo","default_branch":"../../admin"}"#.to_string(),
    )]));
    let client =
        GitHubAcquirer::new("https://github.com/octo/repo", transport.clone(), limits()).unwrap();
    assert!(
        client
            .resolve_ref(&SafeRef::parse("HEAD").unwrap())
            .await
            .is_err()
    );
    assert_eq!(
        transport.requested.lock().unwrap().len(),
        1,
        "no ref request may follow a rejected default branch"
    );
}

#[tokio::test]
async fn annotated_tag_refs_dereference_to_the_commit() {
    const TAG_OBJECT_SHA: &str = "5555555555555555555555555555555555555555";
    const COMMIT_SHA: &str = "6666666666666666666666666666666666666666";
    // The ref points at the tag object; `git/tags/{tag}` reports the tag's
    // target object and kind, which resolves straight to the commit.
    let ref_body = format!(
        r#"{{"ref":"refs/tags/v1.0.0","object":{{"type":"tag","sha":"{TAG_OBJECT_SHA}"}}}}"#
    );
    let tag_body = format!(
        r#"{{"sha":"{TAG_OBJECT_SHA}","object":{{"sha":"{COMMIT_SHA}","type":"commit"}}}}"#
    );
    let transport = Arc::new(ScriptedTransport::with_responses(vec![
        ok_json(ref_body),
        ok_json(tag_body),
    ]));
    let client =
        GitHubAcquirer::new("https://github.com/octo/repo", transport.clone(), limits()).unwrap();
    let sha = client
        .resolve_ref(&SafeRef::parse("refs/tags/v1.0.0").unwrap())
        .await
        .unwrap();
    assert_eq!(sha.as_str(), COMMIT_SHA);
    let requested = transport.requested.lock().unwrap().clone();
    assert_eq!(
        requested.as_slice(),
        [
            "https://api.github.com/repos/octo/repo/git/ref/tags%2Fv1.0.0",
            &format!("https://api.github.com/repos/octo/repo/git/tags/{TAG_OBJECT_SHA}"),
        ],
        "the tag object must be dereferenced through git/tags"
    );
}

#[tokio::test]
async fn annotated_tag_chain_dereferences_until_the_commit() {
    const TAG_A: &str = "7777777777777777777777777777777777777777";
    const TAG_B: &str = "8888888888888888888888888888888888888888";
    const COMMIT_SHA: &str = "9999999999999999999999999999999999999999";
    let ref_body =
        format!(r#"{{"ref":"refs/tags/v2","object":{{"type":"tag","sha":"{TAG_A}"}}}}"#,);
    let tag_a = format!(r#"{{"sha":"{TAG_A}","object":{{"sha":"{TAG_B}","type":"tag"}}}}"#);
    let tag_b = format!(r#"{{"sha":"{TAG_B}","object":{{"sha":"{COMMIT_SHA}","type":"commit"}}}}"#);
    let transport = Arc::new(ScriptedTransport::with_responses(vec![
        ok_json(ref_body),
        ok_json(tag_a),
        ok_json(tag_b),
    ]));
    let client = GitHubAcquirer::new("https://github.com/octo/repo", transport, limits()).unwrap();
    let sha = client
        .resolve_ref(&SafeRef::parse("refs/tags/v2").unwrap())
        .await
        .unwrap();
    assert_eq!(sha.as_str(), COMMIT_SHA);
}

#[tokio::test]
async fn malformed_or_oversized_tag_chains_fail_closed() {
    const TAG_A: &str = "7777777777777777777777777777777777777777";
    // A tag pointing at a tree never resolves to a commit: fail instead of
    // indexing a tree as repository content.
    let ref_body =
        format!(r#"{{"ref":"refs/tags/weird","object":{{"type":"tag","sha":"{TAG_A}"}}}}"#);
    let tag_to_tree = format!(
        r#"{{"sha":"{TAG_A}","object":{{"sha":"{}","type":"tree"}}}}"#,
        "b".repeat(40)
    );
    let transport = Arc::new(ScriptedTransport::with_responses(vec![
        ok_json(ref_body),
        ok_json(tag_to_tree),
    ]));
    let client = GitHubAcquirer::new("https://github.com/octo/repo", transport, limits()).unwrap();
    let error = client
        .resolve_ref(&SafeRef::parse("refs/tags/weird").unwrap())
        .await
        .unwrap_err();
    assert!(error.to_string().contains("git_tag_not_commit"));

    // A malformed target SHA inside the tag response fails closed.
    let ref_body =
        format!(r#"{{"ref":"refs/tags/bad","object":{{"type":"tag","sha":"{TAG_A}"}}}}"#);
    let tag_to_malformed =
        format!(r#"{{"sha":"{TAG_A}","object":{{"sha":"not-a-sha","type":"commit"}}}}"#);
    let transport = Arc::new(ScriptedTransport::with_responses(vec![
        ok_json(ref_body),
        ok_json(tag_to_malformed),
    ]));
    let client = GitHubAcquirer::new("https://github.com/octo/repo", transport, limits()).unwrap();
    assert!(
        client
            .resolve_ref(&SafeRef::parse("refs/tags/bad").unwrap())
            .await
            .is_err()
    );

    // A tag chain deeper than the bounded depth fails instead of walking
    // an unbounded request loop.
    let ref_body =
        format!(r#"{{"ref":"refs/tags/loop","object":{{"type":"tag","sha":"{TAG_A}"}}}}"#);
    let nth_tag = |sha: &str, target_index: usize| {
        format!(
            r#"{{"sha":"{sha}","object":{{"sha":"{}","type":"tag"}}}}"#,
            SafeSha::parse(&format!("{:040x}", target_index))
                .unwrap()
                .as_str()
        )
    };
    let mut responses = vec![ok_json(ref_body)];
    // First response matches the ref's tag SHA; each following tag points
    // at the next generated SHA, so the chain never reaches a commit.
    responses.push(ok_json(nth_tag(TAG_A, 1)));
    for index in 1..MAX_TAG_DEREF_DEPTH + 3 {
        let sha = SafeSha::parse(&format!("{:040x}", index))
            .unwrap()
            .as_str()
            .to_owned();
        responses.push(ok_json(nth_tag(&sha, index + 1)));
    }
    let transport = Arc::new(ScriptedTransport::with_responses(responses));
    let client = GitHubAcquirer::new("https://github.com/octo/repo", transport, limits()).unwrap();
    let error = client
        .resolve_ref(&SafeRef::parse("refs/tags/loop").unwrap())
        .await
        .unwrap_err();
    assert!(error.to_string().contains("git_tag_chain_too_deep"));
}

#[tokio::test]
async fn requested_urls_hit_only_the_approved_host() {
    let transport = Arc::new(ScriptedTransport::with_responses(vec![
        ok_json(ref_body(SHA, "commit")),
        ok_json(tree_body(&[blob_entry("README.md", SHA, 4)], false)),
        ok_json(blob_body(&encoded(b"hi"), 2, SHA)),
    ]));
    let client = GitHubAcquirer::new("https://github.com/octo/repo", transport, limits()).unwrap();
    let sha = client
        .resolve_ref(&SafeRef::parse("refs/heads/main").unwrap())
        .await
        .unwrap();
    let listing = client.list_tree(&sha).await.unwrap();
    client.fetch_blob(&listing.files[0].blob_sha).await.unwrap();
    let requested = client.transport.requested.lock().unwrap().clone();
    assert_eq!(listing.files.len(), 1);
    assert_eq!(requested.len(), 3);
    assert!(
        requested
            .iter()
            .all(|url| url.starts_with("https://api.github.com/repos/octo/repo/")),
        "all requests must hit the approved host: {requested:?}"
    );
}

#[tokio::test]
async fn rejects_ref_pointing_at_a_tree_object() {
    // A ref whose object is a tree/blob is not resolvable to a commit.
    let client = acquirer(vec![ok_json(ref_body(SHA, "tree"))], limits());
    let error = client
        .resolve_ref(&SafeRef::parse("refs/heads/main").unwrap())
        .await
        .unwrap_err();
    assert!(error.to_string().contains("git_ref_not_commit"));
}

#[tokio::test]
async fn tree_listing_skips_non_blob_entries() {
    let entries = [
        dir_entry("src", SHA),
        blob_entry("README.md", SHA, 11),
        format!(r#"{{"type":"commit","path":"sub","sha":"{SHA}"}}"#),
    ];
    let client = acquirer(vec![ok_json(tree_body(&entries, false))], limits());
    let commit = SafeSha::parse(SHA).unwrap();
    let listing = client.list_tree(&commit).await.unwrap();
    assert_eq!(listing.files.len(), 1);
    assert_eq!(listing.files[0].path.as_str(), "README.md");
    assert_eq!(listing.files[0].blob_sha.as_str(), SHA);
    assert_eq!(listing.files[0].size, Some(11));
    // The listing is pinned to the requested commit identity, which is what
    // downstream provenance must carry.
    assert_eq!(listing.commit_sha, commit);
}

#[tokio::test]
async fn tree_listing_rejects_an_echoed_commit_that_is_not_the_requested_one() {
    // The provider echoes the requested id; a response claiming a different
    // commit is not this commit's coverage and must not be returned.
    const OTHER_COMMIT: &str = "1234567890abcdef1234567890abcdef12345678";
    let entries = [blob_entry("README.md", SHA, 11)];
    let body = format!(
        r#"{{"sha":"{OTHER_COMMIT}","truncated":false,"tree":[{}]}}"#,
        entries.join(",")
    );
    let client = acquirer(vec![ok_json(body)], limits());
    let error = client
        .list_tree(&SafeSha::parse(SHA).unwrap())
        .await
        .unwrap_err();
    assert!(error.to_string().contains("git_tree_commit_mismatch"));
}

#[tokio::test]
async fn rejects_truncated_trees() {
    let client = acquirer(
        vec![ok_json(tree_body(
            &[blob_entry("README.md", SHA, 11)],
            true,
        ))],
        limits(),
    );
    let error = client
        .list_tree(&SafeSha::parse(SHA).unwrap())
        .await
        .unwrap_err();
    assert!(error.to_string().contains("git_tree_truncated"));
}

#[tokio::test]
async fn rejects_trees_with_too_many_entries() {
    let entries: Vec<String> = (0..5)
        .map(|index| blob_entry(&format!("f{index}.txt"), SHA, 1))
        .collect();
    let client = acquirer(vec![ok_json(tree_body(&entries, false))], limits());
    let error = client
        .list_tree(&SafeSha::parse(SHA).unwrap())
        .await
        .unwrap_err();
    assert!(error.to_string().contains("git_tree_too_large"));
}

#[tokio::test]
async fn rejects_tree_entries_with_unsafe_paths_or_sizes() {
    // A traversal path excludes only its own entry, never the listing.
    let client = acquirer(
        vec![ok_json(tree_body(
            &[
                blob_entry("../escape", SHA, 1),
                blob_entry("ok.txt", SHA, 1),
            ],
            false,
        ))],
        limits(),
    );
    let listing = client
        .list_tree(&SafeSha::parse(SHA).unwrap())
        .await
        .unwrap();
    assert_eq!(listing.files.len(), 1);
    assert_eq!(listing.files[0].path.as_str(), "ok.txt");
    assert_eq!(listing.excluded_files, 1, "the unsafe path is counted");

    // Declared size exceeds the per-blob budget (64): the entry is dropped
    // from the listing up front.
    let client = acquirer(
        vec![ok_json(tree_body(
            &[
                blob_entry("big.bin", SHA, 65),
                blob_entry("small.txt", SHA, 1),
            ],
            false,
        ))],
        limits(),
    );
    let listing = client
        .list_tree(&SafeSha::parse(SHA).unwrap())
        .await
        .unwrap();
    assert_eq!(listing.files.len(), 1);
    assert_eq!(listing.files[0].path.as_str(), "small.txt");
    assert_eq!(listing.excluded_files, 1, "the oversized entry is counted");

    // Symlink blobs (mode 120000) are excluded: their content is a link
    // target, not file content.
    let client = acquirer(
        vec![ok_json(tree_body(
            &[format!(
                r#"{{"type":"blob","path":"link","sha":"{SHA}","size":1,"mode":"120000"}}"#
            )],
            false,
        ))],
        limits(),
    );
    let listing = client
        .list_tree(&SafeSha::parse(SHA).unwrap())
        .await
        .unwrap();
    assert!(listing.files.is_empty());
    assert_eq!(listing.excluded_files, 1, "the symlink is counted");

    // Blob entries must declare a size the client can verify.
    let client = acquirer(
        vec![ok_json(tree_body(
            &[format!(r#"{{"type":"blob","path":"a","sha":"{SHA}"}}"#)],
            false,
        ))],
        limits(),
    );
    assert!(
        client
            .list_tree(&SafeSha::parse(SHA).unwrap())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn fetches_and_decodes_blobs() {
    let payload = b"hello git";
    let client = acquirer(
        vec![ok_json(blob_body(
            &encoded(payload),
            payload.len() as u64,
            SHA,
        ))],
        limits(),
    );
    let blob = client
        .fetch_blob(&SafeSha::parse(SHA).unwrap())
        .await
        .unwrap();
    assert_eq!(blob.bytes.as_ref(), payload);
    assert_eq!(blob.declared_size, payload.len() as u64);
}

#[tokio::test]
async fn rejects_blobs_that_fail_metadata_checks() {
    let payload = b"hello git";

    // Wrong encoding.
    let body = format!(r#"{{"sha":"{SHA}","size":9,"content":"hello git","encoding":"utf-8"}}"#);
    let client = acquirer(vec![ok_json(body)], limits());
    assert!(
        client
            .fetch_blob(&SafeSha::parse(SHA).unwrap())
            .await
            .is_err()
    );

    // Declared size mismatch after decode.
    let client = acquirer(
        vec![ok_json(blob_body(&encoded(payload), 99, SHA))],
        limits(),
    );
    assert!(
        client
            .fetch_blob(&SafeSha::parse(SHA).unwrap())
            .await
            .is_err()
    );

    // Response SHA differs from the requested SHA.
    let other_sha = "f".repeat(40);
    let client = acquirer(
        vec![ok_json(blob_body(
            &encoded(payload),
            payload.len() as u64,
            &other_sha,
        ))],
        limits(),
    );
    assert!(
        client
            .fetch_blob(&SafeSha::parse(SHA).unwrap())
            .await
            .is_err()
    );

    // Oversized declared size.
    let client = acquirer(
        vec![ok_json(blob_body(
            &encoded(payload),
            u64::from(u32::MAX),
            SHA,
        ))],
        limits(),
    );
    assert!(
        client
            .fetch_blob(&SafeSha::parse(SHA).unwrap())
            .await
            .is_err()
    );

    // Missing size.
    let body = format!(
        r#"{{"sha":"{SHA}","content":"{}","encoding":"base64"}}"#,
        encoded(payload)
    );
    let client = acquirer(vec![ok_json(body)], limits());
    assert!(
        client
            .fetch_blob(&SafeSha::parse(SHA).unwrap())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn aggregate_budget_fails_later_blobs() {
    // 55 decoded bytes fit `max_blob_bytes = 64` (encoded 76 + 55 = 131 fits
    // the derived 7/3 decoder budget), but three of them cross
    // `max_total_bytes = 160`.
    let limits = limits();
    let payload = |byte: u8| vec![byte; 55];
    let client = acquirer(
        vec![
            ok_json(blob_body(&encoded(&payload(b'a')), 55, SHA)),
            ok_json(blob_body(&encoded(&payload(b'b')), 55, SHA)),
            ok_json(blob_body(&encoded(&payload(b'c')), 55, SHA)),
        ],
        limits,
    );
    let sha = SafeSha::parse(SHA).unwrap();
    assert!(client.fetch_blob(&sha).await.is_ok());
    assert!(client.fetch_blob(&sha).await.is_ok());
    let error = client.fetch_blob(&sha).await.unwrap_err();
    let domain = error
        .chain()
        .find_map(|cause| cause.downcast_ref::<DomainError>())
        .unwrap();
    assert!(matches!(domain, DomainError::PayloadTooLarge(_)));
}

#[tokio::test]
async fn response_gates_reject_non_json_and_bad_status() {
    // Non-JSON content type.
    let client = acquirer(
        vec![Ok(TransportResponse {
            status: 200,
            headers: vec![("content-type".into(), "text/html".into())],
            body: br#"<html>"#.as_slice().to_vec().into(),
        })],
        limits(),
    );
    assert!(
        client
            .resolve_ref(&SafeRef::parse("refs/heads/main").unwrap())
            .await
            .is_err()
    );

    // 404 must surface as a bounded upstream error, not transport noise.
    let client = acquirer(
        vec![Ok(TransportResponse {
            status: 404,
            headers: vec![("content-type".into(), "application/json".into())],
            body: br#"{"message":"Not Found"}"#.as_slice().to_vec().into(),
        })],
        limits(),
    );
    let error = client
        .resolve_ref(&SafeRef::parse("refs/heads/main").unwrap())
        .await
        .unwrap_err();
    assert!(error.to_string().contains("git_http_status_404"));
}

#[tokio::test]
async fn malformed_json_and_oversized_bodies_fail_closed() {
    let client = acquirer(vec![ok_json("{not json".into())], limits());
    assert!(
        client
            .resolve_ref(&SafeRef::parse("refs/heads/main").unwrap())
            .await
            .is_err()
    );

    // The transport owns the byte cap. The scripted transport mirrors the
    // production contract: a body beyond the requested cap fails there, so
    // the client sees the capped failure and no oversized body can reach
    // the parser.
    let huge = vec![b' '; 8192];
    let transport = Arc::new(ScriptedTransport::with_responses(vec![Ok(
        TransportResponse {
            status: 200,
            headers: vec![("content-type".into(), "application/json".into())],
            body: huge.into(),
        },
    )]));
    let client = GitHubAcquirer::new("https://github.com/octo/repo", transport, limits()).unwrap();
    let error = client
        .resolve_ref(&SafeRef::parse("refs/heads/main").unwrap())
        .await
        .unwrap_err();
    let domain = error
        .chain()
        .find_map(|cause| cause.downcast_ref::<DomainError>())
        .unwrap();
    assert!(matches!(domain, DomainError::PayloadTooLarge(_)));
}

#[tokio::test]
async fn redirect_statuses_are_errors_not_followed() {
    let client = acquirer(
        vec![Ok(TransportResponse {
            status: 302,
            headers: vec![("location".into(), "https://evil.test/x".into())],
            body: Bytes::new(),
        })],
        limits(),
    );
    let error = client
        .resolve_ref(&SafeRef::parse("refs/heads/main").unwrap())
        .await
        .unwrap_err();
    assert!(error.to_string().contains("git_http_status_302"));
}

#[tokio::test]
async fn client_binds_only_canonical_github_urls() {
    let error = GitHubAcquirer::new(
        "https://github.com.evil.test/octo/repo",
        Arc::new(ScriptedTransport::with_responses(Vec::new())),
        limits(),
    )
    .err()
    .expect("lookalike host must be rejected");
    assert!(error.to_string().contains("invalid_git_repository_url"));
}

#[tokio::test]
async fn head_resolves_a_slash_containing_default_branch_through_the_short_ref_form() {
    // Default branches are branch names, not single path segments: a slash
    // must survive as an encoded separator (`heads%2Frelease%2F1.0`), and the
    // commit the branch points at is what HEAD must return.
    const RELEASE_SHA: &str = "4444444444444444444444444444444444444444";
    let metadata = r#"{"default_branch":"release/1.0"}"#;
    let ref_body = format!(
        r#"{{"ref":"refs/heads/release/1.0","object":{{"type":"commit","sha":"{RELEASE_SHA}"}}}}"#
    );
    let transport = Arc::new(ScriptedTransport::with_responses(vec![
        ok_json(metadata.into()),
        ok_json(ref_body),
    ]));
    let client =
        GitHubAcquirer::new("https://github.com/octo/repo", transport.clone(), limits()).unwrap();
    let sha = client
        .resolve_ref(&SafeRef::parse("HEAD").unwrap())
        .await
        .unwrap();
    assert_eq!(sha.as_str(), RELEASE_SHA);
    assert_eq!(
        transport.requested.lock().unwrap().as_slice(),
        [
            "https://api.github.com/repos/octo/repo",
            "https://api.github.com/repos/octo/repo/git/ref/heads%2Frelease%2F1.0",
        ]
    );
}

#[tokio::test]
async fn tag_response_echoing_a_different_tag_sha_is_rejected() {
    // The tag leg must validate the provider's response identity, exactly
    // like the tree and blob legs: a tag response describing a different tag
    // object is not this ref's target.
    const TAG_OBJECT_SHA: &str = "5555555555555555555555555555555555555555";
    const OTHER_SHA: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const COMMIT_SHA: &str = "6666666666666666666666666666666666666666";
    let ref_body =
        format!(r#"{{"ref":"refs/tags/v1","object":{{"type":"tag","sha":"{TAG_OBJECT_SHA}"}}}}"#);
    let tag_body =
        format!(r#"{{"sha":"{OTHER_SHA}","object":{{"sha":"{COMMIT_SHA}","type":"commit"}}}}"#);
    let client = acquirer(vec![ok_json(ref_body), ok_json(tag_body)], limits());
    let error = client
        .resolve_ref(&SafeRef::parse("refs/tags/v1").unwrap())
        .await
        .unwrap_err();
    assert!(error.to_string().contains("git_tag_sha_mismatch"));
}

#[tokio::test]
async fn tag_pointing_at_a_blob_fails_closed() {
    // A tag chain ending at a blob is not repository file content and must
    // never be indexed; the tree case is covered separately.
    const TAG_OBJECT_SHA: &str = "5555555555555555555555555555555555555555";
    let ref_body = format!(
        r#"{{"ref":"refs/tags/weird","object":{{"type":"tag","sha":"{TAG_OBJECT_SHA}"}}}}"#
    );
    let tag_body = format!(
        r#"{{"sha":"{TAG_OBJECT_SHA}","object":{{"sha":"{}","type":"blob"}}}}"#,
        "c".repeat(40)
    );
    let client = acquirer(vec![ok_json(ref_body), ok_json(tag_body)], limits());
    let error = client
        .resolve_ref(&SafeRef::parse("refs/tags/weird").unwrap())
        .await
        .unwrap_err();
    assert!(error.to_string().contains("git_tag_not_commit"));
}

#[tokio::test]
async fn tree_listing_counts_every_exclusion_reason_once() {
    // One provider listing mixing every drop reason: a directory (skipped,
    // never counted), a kept file, a symlink, an unservable declared size,
    // and a traversal path. The kept file is the only entry returned and the
    // other three file entries are counted as exclusions.
    let entries = [
        dir_entry("src", SHA),
        blob_entry("README.md", SHA, 4),
        format!(r#"{{"type":"blob","path":"link","sha":"{SHA}","size":1,"mode":"120000"}}"#),
        blob_entry("huge.bin", SHA, 65),
        blob_entry("../escape", SHA, 1),
    ];
    let client = acquirer(
        vec![ok_json(tree_body(&entries, false))],
        GitAcquisitionLimits {
            max_tree_entries: 8,
            ..limits()
        },
    );
    let listing = client
        .list_tree(&SafeSha::parse(SHA).unwrap())
        .await
        .unwrap();
    assert_eq!(listing.files.len(), 1);
    assert_eq!(listing.files[0].path.as_str(), "README.md");
    assert_eq!(listing.excluded_files, 3);
}

#[tokio::test]
async fn blob_at_the_exact_per_blob_limit_decodes_and_one_byte_over_fails() {
    // A 64-byte payload is exactly `max_blob_bytes`: its encoded + decoded
    // charge (88 + 64 = 152) must fit the derived decoder budget (157). One
    // byte more must be rejected by the declared-length gate.
    let payload = vec![b'z'; 64];
    let client = acquirer(
        vec![ok_json(blob_body(&encoded(&payload), 64, SHA))],
        limits(),
    );
    let blob = client
        .fetch_blob(&SafeSha::parse(SHA).unwrap())
        .await
        .unwrap();
    assert_eq!(blob.bytes.as_ref(), payload);
    assert_eq!(blob.declared_size, 64);

    let over = vec![b'z'; 65];
    let client = acquirer(vec![ok_json(blob_body(&encoded(&over), 65, SHA))], limits());
    assert!(
        client
            .fetch_blob(&SafeSha::parse(SHA).unwrap())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn tree_entry_at_the_blob_budget_is_kept_and_one_byte_over_is_excluded() {
    // `limits().max_blob_bytes == 64`: the boundary itself is servable, the
    // next byte is not. The exclusion must not be off by one in either
    // direction.
    let entries = [
        blob_entry("at.bin", SHA, 64),
        blob_entry("over.bin", SHA, 65),
    ];
    let client = acquirer(vec![ok_json(tree_body(&entries, false))], limits());
    let listing = client
        .list_tree(&SafeSha::parse(SHA).unwrap())
        .await
        .unwrap();
    assert_eq!(listing.files.len(), 1);
    assert_eq!(listing.files[0].path.as_str(), "at.bin");
    assert_eq!(listing.excluded_files, 1);
}

/// Transport that answers every request with the same 55-byte blob, so a
/// concurrent burst can be forced onto the aggregate reservation.
struct FixedBlobTransport {
    body: String,
}

#[async_trait]
impl GitHttpTransport for FixedBlobTransport {
    async fn get(&self, _url: &str, _max_body_bytes: usize) -> anyhow::Result<TransportResponse> {
        tokio::task::yield_now().await;
        Ok(TransportResponse {
            status: 200,
            headers: vec![("content-type".into(), "application/json".into())],
            body: self.body.clone().into_bytes().into(),
        })
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_blob_fetches_never_exceed_the_aggregate_budget() {
    // max_total_bytes = 160 and each decoded blob is 55 bytes, so exactly two
    // reservations fit (3 * 55 = 165 > 160). If the reserve were a non-atomic
    // load/store, concurrent fetches could lose a charge and admit a third.
    let payload = vec![b'x'; 55];
    let body = blob_body(&encoded(&payload), 55, SHA);
    let client = Arc::new(
        GitHubAcquirer::new(
            "https://github.com/octo/repo",
            Arc::new(FixedBlobTransport { body }),
            limits(),
        )
        .unwrap(),
    );
    let mut handles = Vec::new();
    for _ in 0..16 {
        let client = client.clone();
        handles.push(tokio::spawn(async move {
            client
                .fetch_blob(&SafeSha::parse(SHA).unwrap())
                .await
                .is_ok()
        }));
    }
    let mut successes = 0;
    for handle in handles {
        if handle.await.unwrap() {
            successes += 1;
        }
    }
    assert_eq!(
        successes, 2,
        "the aggregate budget must admit exactly two 55-byte blobs"
    );
}
