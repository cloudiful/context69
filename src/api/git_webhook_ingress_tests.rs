//! Pure provider-path and header parsing tests for the signed webhook ingress.

use super::{event_name, parse_provider, signature_carriers, single_delivery_id};
use crate::contracts::sources::GitProviderKind;
use crate::services::git_webhook_signature::SignatureCarrier;

#[test]
fn the_provider_path_parses_only_known_kinds() {
    for (path, kind) in [
        ("github", GitProviderKind::GitHub),
        ("forgejo", GitProviderKind::Forgejo),
        ("gitlab", GitProviderKind::GitLab),
        ("generic", GitProviderKind::Generic),
    ] {
        assert_eq!(parse_provider(path), Some(kind), "path {path}");
    }
    for unknown in ["", "GitHub", "github-app", "bitbucket"] {
        assert!(parse_provider(unknown).is_none(), "path {unknown}");
    }
}

#[test]
fn forgejo_aliases_delivery_ids_and_events_are_read_from_their_headers() {
    use axum::http::{HeaderMap, HeaderValue};

    let mut headers = HeaderMap::new();
    headers.insert("x-forgejo-signature", HeaderValue::from_static("aa"));
    headers.insert("x-gitea-signature", HeaderValue::from_static("bb"));
    headers.insert("x-gogs-signature", HeaderValue::from_static("cc"));
    headers.insert("x-hub-signature-256", HeaderValue::from_static("sha256=dd"));
    assert_eq!(
        signature_carriers(&headers),
        vec![
            SignatureCarrier::RawHex("aa"),
            SignatureCarrier::RawHex("bb"),
            SignatureCarrier::RawHex("cc"),
            SignatureCarrier::HubSha256("sha256=dd"),
        ],
        "every recognized carrier is verified, not just the first"
    );

    headers.insert("x-forgejo-delivery", HeaderValue::from_static("delivery-1"));
    headers.insert("x-gitea-delivery", HeaderValue::from_static("delivery-2"));
    assert!(
        single_delivery_id(&headers).is_none(),
        "conflicting provider delivery aliases are never resolved by ordering"
    );
    headers.insert("x-gogs-delivery", HeaderValue::from_static("delivery-1"));
    headers.remove("x-gitea-delivery");
    headers.insert("x-github-delivery", HeaderValue::from_static("delivery-1"));
    assert_eq!(
        single_delivery_id(&headers).as_deref(),
        Some("delivery-1"),
        "identical aliases keep the one idempotency key"
    );

    headers.insert("x-forgejo-event", HeaderValue::from_static("push"));
    headers.insert("x-github-event", HeaderValue::from_static("ping"));
    assert_eq!(
        event_name(&headers).as_deref(),
        Some("push"),
        "the Forgejo family is preferred when both families are present"
    );
}

#[test]
fn a_push_matches_only_its_repository_and_tracked_ref() {
    let payload = |full_name: &str, reference: &str| {
        serde_json::from_value::<super::PushPayload>(serde_json::json!({
            "ref": reference,
            "after": "a".repeat(40),
            "repository": { "full_name": full_name }
        }))
        .expect("a push payload")
    };
    assert!(
        payload("octo/repo", "refs/heads/main").matches("octo", "repo", "refs/heads/main", "main"),
        "the stable full name and tracked ref match"
    );
    assert!(
        payload("OCTO/Repo", "refs/heads/main").matches("octo", "repo", "refs/heads/main", "main"),
        "provider casing is not a different repository"
    );
    assert!(
        payload("octo/repo", "main").matches("octo", "repo", "refs/heads/main", "main"),
        "the documented Gitea short branch form is accepted"
    );
    for case in [
        payload("octo/other", "refs/heads/main"),
        payload("octo/repo", "refs/heads/other"),
        payload("octo/repo", "refs/tags/v1"),
    ] {
        assert!(
            !case.matches("octo", "repo", "refs/heads/main", "main"),
            "a foreign repository or untracked ref must not refresh the source"
        );
    }
    assert!(
        !payload("octo/repo", "refs/heads/main").matches("octo", "repo", "refs/tags/v1", "main"),
        "a short branch form is never inferred for a tag target"
    );
}

#[test]
fn a_head_tracked_source_follows_the_default_branch_push() {
    let payload = |reference: &str, default_branch: Option<&str>| {
        let mut repository = serde_json::json!({ "full_name": "octo/repo" });
        if let Some(default_branch) = default_branch {
            repository["default_branch"] = serde_json::Value::String(default_branch.to_string());
        }
        serde_json::from_value::<super::PushPayload>(serde_json::json!({
            "ref": reference,
            "after": "a".repeat(40),
            "repository": repository
        }))
        .expect("a push payload")
    };
    assert!(
        payload("refs/heads/main", Some("main")).matches("octo", "repo", "HEAD", "main"),
        "a HEAD source follows the authenticated default branch push"
    );
    assert!(
        payload("main", Some("main")).matches("octo", "repo", "HEAD", "main"),
        "the short branch form also matches a HEAD source"
    );
    assert!(
        payload("refs/heads/main", None).matches("octo", "repo", "HEAD", "main"),
        "an absent payload default branch falls back to the stored default"
    );
    assert!(
        !payload("refs/heads/other", Some("main")).matches("octo", "repo", "HEAD", "main"),
        "an unrelated branch push never refreshes a HEAD source"
    );
    assert!(
        !payload("refs/heads/main", Some("bad branch")).matches("octo", "repo", "HEAD", "main"),
        "an unsafe payload default branch never matches"
    );
}
