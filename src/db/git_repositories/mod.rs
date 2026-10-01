//! Persistence for provider-neutral Git repository sources (issue #681 work
//! unit 3B1).
//!
//! Provider connections and repository sources are group-owned: every operation
//! takes the owning group, repository identity is per group, and a group's
//! current visibility is read from `context69.groups` on every query instead of
//! being snapshotted at write time. There is no shared/synthetic owner and no
//! owner-less row.
//!
//! Secret material stays in `context69.internal_secrets`: the stored structs
//! expose `*_secret_key` references and the contract conversions only report
//! whether a credential or webhook secret is configured. Webhook deliveries
//! are keyed by the provider delivery id so a redelivery is recognised rather
//! than enqueued twice.
//!
//! Index generations are metadata only (pinned commit plus coverage counters),
//! and completing a generation and making it active is one atomic database
//! operation.
//!
//! Exact code content of one snapshot lives alongside them (issue #681 work unit
//! 3B2): raw bytes stored once per (generation, provider blob id), a manifest
//! mapping safe repository paths to those bytes, and chunks holding verbatim
//! UTF-8 text with inclusive line ranges. Content is writable only while a
//! generation is building, every content row is confined to one generation by
//! composite foreign keys, and the lexical code query answers from the activated
//! generation only.

mod chunks;
mod connections;
mod enums;
mod file_rows;
mod file_types;
mod files;
mod generation_types;
mod generations;
mod repositories;
mod rows;
mod types;
mod webhooks;

#[cfg(test)]
mod file_schema_tests;
#[cfg(test)]
mod schema_tests;
#[cfg(test)]
mod secret_schema_tests;

pub use file_types::{
    GitChunkReplacement, GitLexicalCodeSearch, GitManifestReplacement, MAX_GIT_CHUNKS_PER_FILE,
    MAX_GIT_FILE_BYTES, MAX_GIT_GENERATION_BYTES, MAX_GIT_LEXICAL_LIMIT, MAX_GIT_LIST_PAGE,
    MAX_GIT_MANIFEST_FILES, MAX_GIT_SEARCH_TERM_LENGTH, NewGitGenerationChunk,
    NewGitGenerationFile, StoredGitGenerationChunk, StoredGitGenerationFile,
};
pub use generation_types::{
    GitGenerationCoverage, NewGitRepositoryGeneration, StoredGitActiveGeneration,
    StoredGitRepositoryGeneration,
};
pub use types::{
    GitCheckpointUpdate, GitGroupOwnership, NewGitProviderConnection, NewGitRepositorySource,
    NewGitWebhookDelivery, NewGitWebhookRegistration, StoredGitProviderConnection,
    StoredGitRepositorySource, StoredGitWebhookDelivery, StoredGitWebhookRegistration,
};

/// Behaviour of the two content helpers whose guarantees are not visible in the
/// migration text: the lexical query's token preparation and the caller-side
/// manifest budget. They live here, beside the module composition, because the
/// helper owners (`chunks`, `files`) have no test file of their own.
#[cfg(test)]
mod content_helper_tests {
    #[test]
    fn code_query_terms_keep_identifiers_whole_and_escape_wildcards() {
        use super::chunks::query_terms;

        // `::`, `.`, `/`, `-`, and `_` are identifier characters, so a qualified
        // name stays one token while call syntax and spaces split, and a
        // wildcard in the caller's text stays literal.
        let terms = query_terms("SafeRef::parse(raw) 50%");
        assert_eq!(terms.tokens, vec!["saferef::parse", "raw", "50"]);
        assert_eq!(terms.phrase_pattern, "%saferef::parse(raw) 50\\%%");
        assert_eq!(
            query_terms("parse_ref parse_ref").tokens.len(),
            1,
            "a repeated token must not be sent twice"
        );
        let blank = query_terms("   ");
        assert!(
            blank.phrase.is_empty() && blank.tokens.is_empty(),
            "a blank query is not a search"
        );
    }

    #[test]
    fn the_manifest_budget_counts_deduplicated_bytes_only() {
        use super::file_types::NewGitGenerationFile;
        use super::files::validate_manifest;

        let entry = |path: &str, content: &str| NewGitGenerationFile {
            path: path.to_string(),
            language: "rust".to_string(),
            provider_blob_sha: content.to_string(),
            content: content.as_bytes().to_vec(),
            line_count: 1,
        };

        // The same bytes at two paths cost one database row, so the caller side
        // must not charge them twice and refuse a repository that stores once.
        let shared = "fn shared() {}\n".repeat(4096);
        assert!(validate_manifest(&[entry("a.rs", &shared), entry("b.rs", &shared)]).is_ok());

        // One file over the per-file ceiling is refused before the statement
        // runs. The 64 MiB per-generation ceiling is exercised end to end
        // against the database in tests/git_file_chunks.rs.
        let over_file = "a".repeat(super::file_types::MAX_GIT_FILE_BYTES + 1);
        let error = validate_manifest(&[entry("big.rs", &over_file)])
            .expect_err("an oversized file must be refused before the statement")
            .to_string();
        assert!(error.contains("git_file_too_large"), "{error}");
    }
}
