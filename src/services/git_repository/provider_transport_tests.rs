use super::ProviderApiOrigin;

#[test]
fn provider_origin_accepts_only_its_exact_https_identity() {
    let origin = ProviderApiOrigin::parse("https://forgejo.example/git/api/v1").unwrap();
    for url in [
        "https://forgejo.example/git/api/v1/repos/octo/repo",
        "https://forgejo.example/git/api/v1/repos/octo/repo?page=2",
    ] {
        assert!(
            origin.enforces(&reqwest::Url::parse(url).unwrap()),
            "must accept {url}"
        );
    }
    for url in [
        "https://forgejo.example.evil/api/v1/repos/octo/repo",
        "http://forgejo.example/git/api/v1/repos/octo/repo",
        "https://forgejo.example:8443/git/api/v1/repos/octo/repo",
        "https://user:pass@forgejo.example/git/api/v1/repos/octo/repo",
        "https://forgejo.example/api/v1/repos/octo/repo",
        "https://forgejo.example/git/other/repos/octo/repo",
    ] {
        assert!(
            !origin.enforces(&reqwest::Url::parse(url).unwrap()),
            "must reject {url}"
        );
    }
}

#[test]
fn provider_origin_parse_rejects_non_https_and_userinfo() {
    for base in [
        "http://forgejo.example/api/v1",
        "https://user:pass@forgejo.example/api/v1",
        "https://forgejo.example/api/v1?x=1",
        "https://forgejo.example/api/v1#frag",
    ] {
        assert!(
            ProviderApiOrigin::parse(base).is_err(),
            "must reject {base}"
        );
    }
}
