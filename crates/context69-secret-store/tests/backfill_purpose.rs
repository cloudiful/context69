//! Which stored key the catalogue can name, and what happens without a key.
//!
//! The startup sealing backfill classifies every legacy row it claims by asking
//! the purpose catalogue which owner its key implies. That question has exactly
//! two acceptable answers — the owner, or nothing — so both halves are asserted
//! here without a database: each of the six fixed singleton names and each of
//! the four record-scoped prefixes resolves to the purpose that owns it, and
//! anything the catalogue does not own resolves to nothing at all.
//!
//! Exhaustiveness is the other half. The resolver walks [`SecretPurpose::ALL`],
//! so a catalogue entry that neither matches a fixed name nor declares a prefix
//! would be silently unclassifiable, and a stored value under it could never be
//! sealed. The catalogue test therefore asserts every purpose is reachable
//! through the resolver, which is what makes the sweep total over the catalogue.
//!
//! These are deterministic in-memory catalogue units; no database is required.

use context69_secret_store::{SecretPurpose, key_names};

/// A record identifier a frame header could carry. Generic on purpose: the
/// resolver accepts what the key-name bound accepts, not the syntax one writer
/// happens to use today.
const RECORD: &str = "0f7c1a44-2b91-4f0e-9a3d-8c5e6b2d1a07";

/// A record identifier the key-name bound refuses: a space.
const MALFORMED_RECORD: &str = "not a record id";

// --- the six fixed singleton names --------------------------------------------

#[test]
fn every_fixed_singleton_name_resolves_to_its_purpose() {
    let expected = [
        (
            SecretPurpose::BrowserSessionSigningKey,
            key_names::BROWSER_SESSION_SIGNING_KEY,
        ),
        (SecretPurpose::EmbeddingApiKey, key_names::EMBEDDING_API_KEY),
        (SecretPurpose::SearchApiKey, key_names::SEARCH_API_KEY),
        (
            SecretPurpose::DoclingVlmApiKey,
            key_names::DOCLING_VLM_API_KEY,
        ),
        (
            SecretPurpose::TranslationProviderApiKey,
            key_names::TRANSLATION_PROVIDER_API_KEY,
        ),
        (
            SecretPurpose::RuntimeS3SecretKey,
            key_names::RUNTIME_S3_SECRET_KEY,
        ),
    ];
    assert_eq!(expected.len(), 6, "the catalogue fixes six singleton names");

    for (purpose, key) in expected {
        assert_eq!(key_name_of(purpose), key, "{purpose}");
        assert_eq!(SecretPurpose::owning_purpose(key), Some(purpose), "{key}");
    }
}

// --- the four record-scoped prefixes ------------------------------------------

#[test]
fn every_record_scoped_prefix_resolves_to_its_purpose_with_any_valid_suffix() {
    let expected = [
        (
            SecretPurpose::SourceConnectionDatabaseUrl,
            key_names::SOURCE_CONNECTION_DATABASE_URL_PREFIX,
        ),
        (
            SecretPurpose::GitProviderToken,
            key_names::GIT_PROVIDER_TOKEN_PREFIX,
        ),
        (
            SecretPurpose::GitHubAppPrivateKey,
            key_names::GITHUB_APP_PRIVATE_KEY_PREFIX,
        ),
        (
            SecretPurpose::GitWebhookSigningSecret,
            key_names::GIT_WEBHOOK_SIGNING_SECRET_PREFIX,
        ),
    ];
    assert_eq!(expected.len(), 4, "the catalogue fixes four prefixes");

    for (purpose, prefix) in expected {
        assert_eq!(key_prefix_of(purpose), prefix, "{purpose}");
        // The suffix is whatever a caller's record identifier looks like, not
        // the syntax one writer produces today: a UUID, a Git group's derived
        // token, and a synthetic marker all have to resolve.
        for record in [RECORD, "g42.9f86d081884c7d659a2feaa0c55ad015", "row-1"] {
            let key = format!("{prefix}{record}");
            assert_eq!(SecretPurpose::owning_purpose(&key), Some(purpose), "{key}");
        }
    }
}

fn key_name_of(purpose: SecretPurpose) -> &'static str {
    purpose
        .singleton_key_name()
        .unwrap_or_else(|| panic!("{purpose} has no fixed name"))
}

fn key_prefix_of(purpose: SecretPurpose) -> &'static str {
    purpose
        .key_name_prefix()
        .unwrap_or_else(|| panic!("{purpose} has no prefix"))
}

// --- exhaustive, and refusals -------------------------------------------------

#[test]
fn the_resolver_covers_every_purpose_in_the_catalogue() {
    // The sweep can only seal what the catalogue can name. A purpose that is
    // neither a fixed name nor a prefixed family would be permanently
    // unclassifiable, and this is where that would be caught.
    for purpose in SecretPurpose::ALL {
        let resolved = purpose.singleton_key_name().map_or_else(
            || {
                let prefix = purpose
                    .key_name_prefix()
                    .unwrap_or_else(|| panic!("{purpose} is neither fixed nor record-scoped"));
                SecretPurpose::owning_purpose(&format!("{prefix}{RECORD}"))
            },
            SecretPurpose::owning_purpose,
        );
        assert_eq!(resolved, Some(purpose), "{purpose} is unreachable");
    }
}

#[test]
fn a_key_name_the_catalogue_does_not_own_resolves_to_nothing() {
    // The refusal is the whole point: a name outside the catalogue has no owner
    // that can be proven, so it must not be guessed into one.
    for unknown in [
        "not_a_catalogue_key",
        "embedding_api_key_v2",
        "browser_session_signing_key_v1",
        "runtime_s3.access_key",
        "git_provider.token",
        "source_connection.database_url.",
        "git_webhook.signing_secret.",
        "",
    ] {
        assert_eq!(SecretPurpose::owning_purpose(unknown), None, "{unknown}");
    }
}

#[test]
fn a_prefixed_key_with_an_unusable_record_resolves_to_nothing() {
    for prefix in [
        key_names::SOURCE_CONNECTION_DATABASE_URL_PREFIX,
        key_names::GIT_PROVIDER_TOKEN_PREFIX,
        key_names::GITHUB_APP_PRIVATE_KEY_PREFIX,
        key_names::GIT_WEBHOOK_SIGNING_SECRET_PREFIX,
    ] {
        // Empty, whitespace, a character the frame refuses, and one long enough
        // to overflow the key-name bound with its prefix.
        let key = format!("{prefix}{MALFORMED_RECORD}");
        assert_eq!(SecretPurpose::owning_purpose(&key), None, "{key}");
        assert_eq!(
            SecretPurpose::owning_purpose(prefix),
            None,
            "a bare prefix is an unclaimed category, not a record"
        );
        let too_long = "a".repeat(200);
        assert_eq!(
            SecretPurpose::owning_purpose(&format!("{prefix}{too_long}")),
            None
        );
    }
}

// --- the no-key path ----------------------------------------------------------
