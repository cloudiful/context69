use super::super::github_url::parse_canonical_github_url;
use super::{ForgejoOrigin, github_connection_origin_matches, validate_connection_origin};
use crate::contracts::sources::GitProviderKind;

#[test]
fn forgejo_origin_accepts_root_and_api_base_forms() {
    let root = ForgejoOrigin::from_base_url("https://forgejo.example").unwrap();
    let api = ForgejoOrigin::from_base_url("https://forgejo.example/api/v1/").unwrap();
    assert_eq!(root, api);
    assert_eq!(root.api_base(), "https://forgejo.example/api/v1");
    assert_eq!(
        root.parse_repository_url("https://forgejo.example/octo/repo.git/")
            .unwrap()
            .name,
        "repo"
    );
    assert_eq!(
        root.canonical_repository_url(
            &root
                .parse_repository_url("https://forgejo.example/octo/repo")
                .unwrap()
        ),
        "https://forgejo.example/octo/repo"
    );
}

#[test]
fn forgejo_origin_preserves_a_webroot_and_non_default_port() {
    let origin = ForgejoOrigin::from_base_url("https://forgejo.example:8443/git/api/v1").unwrap();
    assert_eq!(origin.api_base(), "https://forgejo.example:8443/git/api/v1");
    assert_eq!(
        origin
            .parse_repository_url("https://forgejo.example:8443/git/octo/repo")
            .unwrap()
            .owner,
        "octo"
    );
    assert!(
        origin
            .parse_repository_url("https://forgejo.example:8443/octo/repo")
            .is_err(),
        "a URL outside the webroot must not resolve"
    );
    assert!(
        origin
            .parse_repository_url("https://forgejo.example:8443/submarine/octo/repo")
            .is_err(),
        "a webroot prefix must end on a path boundary"
    );
}

#[test]
fn forgejo_origin_rejects_non_https_userinfo_and_traversal() {
    for base in [
        "http://forgejo.example",
        "https://user:pass@forgejo.example",
        "https://forgejo.example/api/v1?x=1",
        "https://forgejo.example/api/v1#frag",
        "https://forgejo.example//evil",
    ] {
        assert!(
            ForgejoOrigin::from_base_url(base).is_err(),
            "must reject {base}"
        );
    }
}

#[test]
fn forgejo_repository_url_must_match_the_connection_origin() {
    assert!(
        validate_connection_origin(
            GitProviderKind::Forgejo,
            "https://forgejo.example",
            "https://forgejo.example/octo/repo"
        )
        .is_ok()
    );
    for url in [
        "https://evil.example/octo/repo",
        "http://forgejo.example/octo/repo",
        "https://forgejo.example:8443/octo/repo",
        "https://forgejo.example/octo/repo/tree/main",
        "https://forgejo.example/octo",
    ] {
        let error =
            validate_connection_origin(GitProviderKind::Forgejo, "https://forgejo.example", url)
                .expect_err("origin mismatch must be refused");
        assert!(
            error.to_string().contains("git_connection_origin_mismatch"),
            "{error}"
        );
    }
}

#[test]
fn github_connection_must_name_the_approved_api_origin() {
    assert!(github_connection_origin_matches("https://api.github.com"));
    assert!(github_connection_origin_matches("https://api.github.com/"));
    assert!(github_connection_origin_matches(
        "https://api.github.com/api/v3"
    ));
    for base in [
        "https://github.com",
        "http://api.github.com",
        "https://api.github.com:8443",
        "https://user@api.github.com",
        "https://api.github.com/other",
    ] {
        assert!(
            !github_connection_origin_matches(base),
            "must reject {base}"
        );
    }
    assert!(
        validate_connection_origin(
            GitProviderKind::GitHub,
            "https://api.github.com",
            "https://github.com/octo/repo"
        )
        .is_ok()
    );
    assert!(
        validate_connection_origin(
            GitProviderKind::GitHub,
            "https://api.github.com",
            "https://gitlab.com/octo/repo"
        )
        .is_err()
    );
}

#[test]
fn unsupported_provider_is_refused() {
    let error = validate_connection_origin(
        GitProviderKind::GitLab,
        "https://gitlab.com",
        "https://gitlab.com/octo/repo",
    )
    .expect_err("only GitHub and Forgejo acquire repositories");
    assert!(error.to_string().contains("git_provider_unsupported"));
}

#[test]
fn the_github_parser_stays_the_single_canonical_form() {
    assert!(parse_canonical_github_url("https://github.com/octo/repo").is_ok());
    assert!(parse_canonical_github_url("https://forgejo.example/octo/repo").is_err());
}
