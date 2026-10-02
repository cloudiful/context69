//! The purpose and key-name catalogue.
//!
//! The catalogue is what stops an unowned purpose from being invented at a call
//! site, so these tests assert the catalogue itself: that every entry is a
//! distinct bounded identifier, that every stored purpose parses back, and that a
//! key name is refused before storage when a dynamic identifier would make it
//! malformed. No database, no cipher, and no secret value is involved.

use context69_secret_store::{
    LONGEST_KEY_NAME, LONGEST_PURPOSE, SecretKeyName, SecretPurpose, SecretStoreError, key_names,
};

/// Key names a frame header and a row key can never carry. The first is empty;
/// the rest are outside the identifier character set.
const MALFORMED: [&str; 8] = [
    "",
    "with space",
    "with\nnewline",
    "with\0nul",
    "with\"quote",
    "with/slash",
    "with:colon",
    "üñïçø∂é",
];

fn key(name: &str) -> SecretKeyName {
    SecretKeyName::new(name).expect("valid key name")
}

/// The error kind as a comparable label. The error enum deliberately has no
/// `PartialEq` (it wraps `anyhow::Error`), so tests assert on the kind and read
/// the rendered message separately.
fn kind(error: &SecretStoreError) -> &'static str {
    match error {
        SecretStoreError::MasterKeyNotConfigured => "master_key_not_configured",
        SecretStoreError::MasterKeyRejected(_) => "master_key_rejected",
        SecretStoreError::InvalidKeyName => "invalid_key_name",
        SecretStoreError::InvalidPurpose => "invalid_purpose",
        SecretStoreError::PurposeMismatch => "purpose_mismatch",
        SecretStoreError::SecretNotFound => "secret_not_found",
        SecretStoreError::Cipher(_) => "cipher",
        SecretStoreError::Database(_) => "database",
    }
}

#[test]
fn every_catalogue_entry_is_a_distinct_bounded_purpose() {
    let mut seen: Vec<&str> = Vec::new();
    for purpose in SecretPurpose::ALL {
        let name = purpose.as_str();
        assert!(
            name.len() <= LONGEST_PURPOSE,
            "purpose {name:?} is longer than the documented bound"
        );
        assert!(
            SecretPurpose::parse(name).is_some(),
            "purpose {name:?} does not parse back"
        );
        assert!(
            !seen.contains(&name),
            "purpose {name:?} is used by more than one category"
        );
        seen.push(name);
    }
    assert_eq!(
        seen.len(),
        SecretPurpose::ALL.len(),
        "the catalogue must have no duplicates"
    );
}

#[test]
fn the_catalogue_covers_every_in_scope_category() {
    // One entry per persisted reversible category the plan enumerates. A new
    // category must be added here too, so this test is the catalogue's contract.
    let expected = [
        "browser_session.signing_key",
        "embedding.api_key",
        "search.api_key",
        "docling.vlm_api_key",
        "translation.api_key",
        "source_connection.database_url",
        "runtime_s3.secret_key",
        "git_provider.token",
        "github_app.private_key",
        "git_webhook.signing_secret",
    ];
    for name in expected {
        assert!(
            SecretPurpose::parse(name).is_some(),
            "category {name:?} is missing from the catalogue"
        );
    }
    assert_eq!(expected.len(), SecretPurpose::ALL.len());
}

#[test]
fn a_singleton_category_names_its_key_and_a_record_category_does_not() {
    for purpose in SecretPurpose::ALL {
        match purpose.singleton_key_name() {
            Some(name) => {
                assert!(purpose.is_singleton(), "{purpose} has a singleton key name");
                assert_eq!(Some(name), purpose.key_name_prefix().or(Some(name)));
                // The published constant must itself be a usable key name.
                assert_eq!(key(name).as_str(), name, "{purpose}");
            }
            None => {
                assert!(!purpose.is_singleton(), "{purpose} has no singleton key");
                assert!(
                    purpose.key_name_prefix().is_some(),
                    "{purpose} must name a prefix for its per-record keys"
                );
            }
        }
    }
}

#[test]
fn the_browser_session_row_key_is_unchanged() {
    // Renaming this orphans every deployed signing key, so it is pinned.
    assert_eq!(
        key_names::BROWSER_SESSION_SIGNING_KEY,
        "browser_session_signing_key_v2"
    );
    assert_eq!(
        SecretPurpose::BrowserSessionSigningKey.singleton_key_name(),
        Some("browser_session_signing_key_v2")
    );
    assert_eq!(
        SecretPurpose::BrowserSessionSigningKey.to_string(),
        "browser_session.signing_key"
    );
}

#[test]
fn a_malformed_key_name_is_refused_before_it_can_reach_storage() {
    for name in MALFORMED {
        assert_eq!(
            kind(&SecretKeyName::new(name).expect_err(name)),
            "invalid_key_name",
            "key name {name:?}"
        );
        assert_eq!(
            kind(&SecretKeyName::try_from(name).expect_err(name)),
            "invalid_key_name",
            "key name {name:?}"
        );
    }
    assert_eq!(
        kind(&SecretKeyName::new(&"k".repeat(LONGEST_KEY_NAME + 1)).expect_err("oversized")),
        "invalid_key_name"
    );
    assert!(SecretKeyName::new(&"k".repeat(LONGEST_KEY_NAME)).is_ok());
}

#[test]
fn a_malformed_record_identifier_is_refused_before_it_can_reach_storage() {
    let prefix = key_names::SOURCE_CONNECTION_DATABASE_URL_PREFIX;
    for record in MALFORMED {
        // The prefix is well formed, so the whole failure is the record's.
        assert!(SecretKeyName::new(prefix).is_ok(), "prefix must stay valid");
        assert_eq!(
            kind(&SecretKeyName::with_prefix(prefix, record).expect_err(record)),
            "invalid_key_name",
            "record {record:?}"
        );
    }
    // A record that overflows the bound is refused too, not truncated.
    assert_eq!(
        kind(
            &SecretKeyName::with_prefix(prefix, &"c".repeat(LONGEST_KEY_NAME))
                .expect_err("overflow")
        ),
        "invalid_key_name"
    );
    assert_eq!(
        SecretKeyName::with_prefix(prefix, &"c".repeat(LONGEST_KEY_NAME - prefix.len()))
            .expect("fits")
            .as_str(),
        format!("{prefix}{}", "c".repeat(LONGEST_KEY_NAME - prefix.len()))
    );
}

#[test]
fn a_built_key_name_is_usable_as_a_row_key() {
    let built =
        SecretKeyName::with_prefix(key_names::GIT_PROVIDER_TOKEN_PREFIX, "acme.github.main")
            .expect("built key name");
    assert_eq!(built.as_str(), "git_provider.token.acme.github.main");
    assert_eq!(built.to_string(), built.as_str());
    assert_eq!(
        SecretKeyName::new(built.as_str()).expect("round trip"),
        built
    );
}

#[test]
fn an_unknown_stored_purpose_is_refused_rather_than_assumed() {
    for name in [
        "",
        "not_a_purpose",
        "with space",
        "browser_session.signing_key ",
        "BROWSER_SESSION.SIGNING_KEY",
    ] {
        assert_eq!(
            kind(&SecretPurpose::validate_name(name).expect_err(name)),
            "invalid_purpose",
            "purpose {name:?}"
        );
    }
    for purpose in SecretPurpose::ALL {
        SecretPurpose::validate_name(purpose.as_str())
            .unwrap_or_else(|error| panic!("{purpose} must validate: {error}"));
    }
}
