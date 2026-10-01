use super::{
    MAX_REF_LENGTH, MAX_TREE_PATH_DEPTH, MAX_TREE_PATH_LENGTH, SafeRef, SafeSha, SafeTreePath,
    TreePathRejection, classify_tree_path,
};

#[test]
fn accepts_well_formed_refs() {
    for value in [
        "HEAD",
        "refs/heads/main",
        "refs/tags/v1.0.0",
        "refs/heads/feature/x_1",
    ] {
        let parsed = SafeRef::parse(value).unwrap();
        assert_eq!(parsed.as_str(), value, "{value}");
    }
}

#[test]
fn api_path_strips_refs_prefix_and_keeps_head() {
    assert_eq!(
        SafeRef::parse("refs/heads/main").unwrap().api_path(),
        "heads/main"
    );
    assert_eq!(
        SafeRef::parse("refs/tags/v1.0.0").unwrap().api_path(),
        "tags/v1.0.0"
    );
    assert_eq!(SafeRef::parse("HEAD").unwrap().api_path(), "HEAD");
}

#[test]
fn rejects_refs_that_are_not_under_refs_or_head() {
    for value in [
        "main",
        "heads/main",
        "refs",
        "ref/heads/main",
        "--upload-pack=/bin/sh",
        "-",
        "",
        "   ",
    ] {
        assert!(SafeRef::parse(value).is_err(), "must reject {value:?}");
    }
}

#[test]
fn rejects_traversal_and_separator_injection() {
    for value in [
        "refs/heads/../secrets",
        "refs//heads/main",
        "refs/heads//",
        "refs/heads/main/",
        "refs/heads/..",
        "refs/heads/@{0}",
        "refs/heads/ma in",
        "refs/heads/main\t",
        "refs/heads/main\n",
        "refs/heads/main%2F",
    ] {
        assert!(SafeRef::parse(value).is_err(), "must reject {value:?}");
    }
}

#[test]
fn rejects_lock_suffix_and_leading_dot_segments() {
    for value in ["refs/heads/main.lock", "refs/.hidden", "refs/heads/."] {
        assert!(SafeRef::parse(value).is_err(), "must reject {value:?}");
    }
}

#[test]
fn rejects_oversized_refs() {
    let long = format!("refs/heads/{}", "a".repeat(MAX_REF_LENGTH));
    assert!(SafeRef::parse(&long).is_err());
}

#[test]
fn accepts_real_world_repository_paths() {
    for value in [
        "README.md",
        "src/lib.rs",
        "docs/deep/file name.txt",
        ".github/CODEOWNERS",
        ".github/PULL_REQUEST_TEMPLATE.md",
        ".github/workflows/stale.yml",
        ".gitignore",
        "Cargo.lock",
        "libs/lockfile/tests/registry_data/@a__pkg@2.1.5.json",
        "tests/registry/jsr/@deno/deploy/meta.json",
        "naïve/ünïcode/résumé.md",
        "表/語/文件.txt",
    ] {
        let parsed = SafeTreePath::parse(value)
            .unwrap_or_else(|why| panic!("must accept {value:?}, rejected with {why:?}"));
        assert_eq!(parsed.as_str(), value, "{value}");
    }
}

#[test]
fn rejects_traversal_and_absolute_paths() {
    for value in [
        "/etc/passwd",
        "./README.md",
        "../README.md",
        "src/../..",
        "src//lib.rs",
        "src/",
        "C:\\repo\\file",
        "src\\lib.rs",
    ] {
        let why = classify_tree_path(value).unwrap_err();
        assert!(
            matches!(
                why,
                TreePathRejection::Traversal | TreePathRejection::Absolute
            ),
            "{value:?} must be traversal/absolute, got {why:?}"
        );
    }
    // The empty path is out of bounds rather than a traversal attempt.
    assert_eq!(classify_tree_path(""), Err(TreePathRejection::OutOfBounds));
}

#[test]
fn rejects_control_and_oversized_paths() {
    assert_eq!(
        classify_tree_path("bad\u{0}name"),
        Err(TreePathRejection::Control)
    );
    assert_eq!(
        classify_tree_path("bad\nname"),
        Err(TreePathRejection::Control)
    );
    assert_eq!(
        classify_tree_path(&"a".repeat(MAX_TREE_PATH_LENGTH + 1)),
        Err(TreePathRejection::OutOfBounds)
    );
    let deep = (0..MAX_TREE_PATH_DEPTH + 1)
        .map(|_| "d")
        .collect::<Vec<_>>()
        .join("/");
    assert_eq!(
        classify_tree_path(&deep),
        Err(TreePathRejection::OutOfBounds)
    );
    let at_depth_limit = (0..MAX_TREE_PATH_DEPTH)
        .map(|_| "d")
        .collect::<Vec<_>>()
        .join("/");
    assert_eq!(classify_tree_path(&at_depth_limit), Ok(()));
}

#[test]
fn tree_path_length_boundary_is_inclusive() {
    // The bound is a hard ceiling: exactly the maximum length is accepted and
    // one byte more is out of bounds.
    let at_limit = "a".repeat(MAX_TREE_PATH_LENGTH);
    assert_eq!(SafeTreePath::parse(&at_limit).unwrap().as_str(), at_limit);
    let over_limit = "a".repeat(MAX_TREE_PATH_LENGTH + 1);
    assert_eq!(
        classify_tree_path(&over_limit),
        Err(TreePathRejection::OutOfBounds)
    );
}

#[test]
fn accepts_lowercase_hex_shas_of_both_lengths() {
    for value in ["a".repeat(40), "0123456789abcdef".repeat(4)] {
        let parsed = SafeSha::parse(&value).unwrap();
        assert_eq!(parsed.as_str(), value.as_str());
    }
}

#[test]
fn rejects_malformed_shas_as_upstream_errors() {
    for value in [
        "A".repeat(40),
        "g".repeat(40),
        "a".repeat(39),
        "a".repeat(41),
        "a".repeat(63),
        String::new(),
        "../../etc/passwd".to_owned(),
        "  ".to_owned(),
    ] {
        let error = SafeSha::parse(&value).unwrap_err();
        let message = error.to_string();
        assert!(
            message.contains("git_sha_malformed"),
            "{value:?} must be git_sha_malformed, got {message}"
        );
    }
}
