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
//! The no-key case belongs here too: a deployment with no master key must report
//! zero work *without reaching the database*, so it is asserted against a pool
//! that was never connected. No test in this file needs PostgreSQL.

use context69_secret_store::{SecretDatabase, SecretPurpose, SecretStore, key_names};
use sqlx::postgres::PgPoolOptions;

const MASTER_KEY_B64: &str = "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=";

/// A record identifier a frame header could carry. Generic on purpose: the
/// resolver accepts what the key-name bound accepts, not the syntax one writer
/// happens to use today.
const RECORD: &str = "0f7c1a44-2b91-4f0e-9a3d-8c5e6b2d1a07";

/// A record identifier the key-name bound refuses: a space.
const MALFORMED_RECORD: &str = "not a record id";

/// A pool over an address nothing is listening on, so any statement the no-key
/// path issued would fail rather than pass quietly.
///
/// Opened inside the runtime that drives the case — a pool's driver belongs to
/// the runtime that built it — and never connected, with a short acquire
/// timeout so a path that *did* query fails the case promptly instead of waiting
/// out the default.
async fn unreachable_pool() -> sqlx::PgPool {
    PgPoolOptions::new()
        .acquire_timeout(std::time::Duration::from_secs(1))
        .connect_lazy("postgres://c69test@127.0.0.1:1/nowhere")
        .expect("a lazy pool needs no reachable server")
}

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

#[test]
fn a_deployment_without_a_master_key_reports_no_work_and_issues_no_statement() {
    // The pool points at a port nothing is listening on, so a run that opened a
    // transaction or issued a statement would fail here rather than pass
    // quietly. Reaching the assertion is the proof that the no-key path decided
    // before it touched the database.
    for absent in [None, Some(""), Some("   ")] {
        let report = sqlx::test_block_on(async {
            let store = SecretStore::new(SecretDatabase::new(unreachable_pool().await), absent, 1)
                .expect("build a store");
            store.seal_legacy_plaintext().await
        })
        .expect("an unkeyed deployment has nothing to seal");
        assert_eq!(
            format!("{report:?}"),
            "SealedLegacyReport { sealed: 0, batches: 0 }"
        );
    }
}

#[test]
fn a_configured_key_starts_the_run_and_reports_only_a_category() {
    // A store with a key, asked against a database that is not there: it really
    // did start, so it fails as a database failure — and what it reports is the
    // category, carrying no connection detail, no SQL, and no key material.
    let error = sqlx::test_block_on(async {
        let store = SecretStore::new(
            SecretDatabase::new(unreachable_pool().await),
            Some(MASTER_KEY_B64),
            1,
        )
        .expect("a test master key builds a store");
        store.seal_legacy_plaintext().await
    })
    .expect_err("no database");
    assert_eq!(format!("{error:?}"), "DatabaseUnavailable");
    let rendered = format!("{error}");
    for leak in [MASTER_KEY_B64, "127.0.0.1", "SELECT", "internal_secrets"] {
        assert!(!rendered.contains(leak), "{rendered} leaked {leak:?}");
    }
}
