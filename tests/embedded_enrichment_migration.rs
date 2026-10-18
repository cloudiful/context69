//! Pure contract tests for the automatic embedded-enrichment retirement
//! migration (issue #910 S1).
//!
//! These do not connect to a database. They inspect the authored migration and
//! prove the normalizer semantics the migration depends on, so the destructive
//! properties can be reviewed before the migration is ever applied.

use chrono::Utc;
use context69::domain::SourceRecord;
use context69::normalize::normalize_record;
use serde_json::{Value, json};

#[path = "embedded_enrichment_migration/database.rs"]
mod database;

const MIGRATION: &str = include_str!("../migrations/20261017080707_retire_embedded_enrichment.sql");

fn source(metadata_json: Value) -> SourceRecord {
    SourceRecord {
        external_id: "600519.SH-2026-01-01".to_string(),
        title: "  Company   announcement ".to_string(),
        body_text: "alpha\nbeta".to_string(),
        source_uri: " https://example.test/announcement ".to_string(),
        summary: Some(" short   summary ".to_string()),
        published_at: None,
        updated_at: Utc::now(),
        metadata_json,
    }
}

#[test]
fn stripping_the_embedded_key_preserves_the_current_source_hash() {
    let business = json!({"ts_code": "600519.SH"});
    let merged = json!({
        "ts_code": "600519.SH",
        "extraction": {"stock.news.v1": {"direction": "positive"}}
    });

    // The historical publisher hashed the source before merging enrichment, so
    // the document keeps the business-metadata hash while its stored metadata
    // already carries `extraction`.
    let document_hash = normalize_record(source(business.clone())).record_hash;

    let mut stripped_metadata = merged.clone();
    stripped_metadata
        .as_object_mut()
        .expect("metadata object")
        .remove("extraction");
    let stripped = normalize_record(source(stripped_metadata));
    assert_eq!(stripped.metadata_json, business);
    assert_eq!(
        stripped.record_hash, document_hash,
        "removing extraction and normalizing must reproduce the current hash"
    );

    // Hashing the merged metadata itself does not reproduce the document hash,
    // which is why the migration trusts the immutable canonical snapshot and
    // never recomputes a hash from the merged payload.
    let with_enrichment = normalize_record(source(merged));
    assert_ne!(with_enrichment.record_hash, document_hash);
}

#[test]
fn migration_creates_the_durable_outbox_before_any_data_change() {
    let outbox_table = MIGRATION
        .find("CREATE TABLE IF NOT EXISTS context69.document_metadata_payload_sync")
        .expect("outbox table is created");
    let first_guard = MIGRATION
        .find("canonical snapshot does not match the stripped source metadata")
        .expect("snapshot guard exists");
    assert!(
        outbox_table < first_guard,
        "the outbox must exist even when no document is affected"
    );
    for fragment in [
        "document_id BIGINT NOT NULL REFERENCES context69.documents(id) ON DELETE CASCADE",
        "payload_key TEXT NOT NULL",
        "attempts INTEGER NOT NULL DEFAULT 0",
        "next_attempt_at TIMESTAMPTZ NOT NULL DEFAULT now()",
        "last_error TEXT",
        "completed_at TIMESTAMPTZ",
        "lease_token UUID",
        "lease_expires_at TIMESTAMPTZ",
        "CHECK (btrim(payload_key) <> '')",
        "CHECK (attempts >= 0)",
        "CHECK ((lease_token IS NULL) = (lease_expires_at IS NULL))",
        "uq_document_metadata_payload_sync_pending",
        "(document_id, payload_key)",
        "WHERE completed_at IS NULL",
        "idx_document_metadata_payload_sync_due",
        "(next_attempt_at, id)",
    ] {
        assert!(
            MIGRATION.contains(fragment),
            "outbox contract fragment missing: {fragment}"
        );
    }
}

#[test]
fn zero_affected_documents_still_converge_obsolete_labels_and_index_definitions() {
    // The independent convergence section is the only place outside the guarded
    // `affected > 0` block that mutates state; it always runs.
    let independent = MIGRATION
        .find("Independent convergence that does not depend on any document")
        .expect("independent cleanup marker");
    let else_branch = MIGRATION
        .find("enqueued := 0;")
        .expect("guarded block else branch");
    let guarded_end = else_branch
        + MIGRATION[else_branch..]
            .find("END IF;")
            .expect("guarded block ends");
    assert!(
        independent > guarded_end,
        "independent cleanup must follow the guarded document block"
    );

    let stage = MIGRATION
        .find("UPDATE context69.tasks SET failure_stage = NULL WHERE failure_stage = 'extraction'")
        .expect("task label cleanup");
    let items = MIGRATION
        .find("UPDATE context69.task_items SET failure_stage = NULL WHERE failure_stage = 'extraction'")
        .expect("item label cleanup");
    let indexes = MIGRATION
        .find("DELETE FROM context69.metadata_index_definitions")
        .expect("index definition cleanup");
    assert!(
        stage > independent && items > independent && indexes > independent,
        "label and index cleanup must be unconditional"
    );

    // It narrows exactly to the retired label and retired index paths.
    assert!(MIGRATION.contains("WHERE failure_stage = 'extraction'"));
    assert!(
        MIGRATION.contains("WHERE field_path = 'extraction' OR field_path LIKE 'extraction.%'")
    );
    // Unrelated columns/rows are never enumerated or rewritten.
    for forbidden in [
        "SET status =",
        "SET error_message =",
        "SET error_summary =",
        "SET queued_count =",
        "SET failed_count =",
        "DELETE FROM context69.metadata_index_definitions\nWHERE field_path NOT",
    ] {
        assert!(
            !MIGRATION.contains(forbidden),
            "cleanup must not widen beyond the retired label/index: {forbidden}"
        );
    }

    // The cache bump is conditional on something having changed, and the
    // outbox enqueue stays inside the affected-document block.
    assert!(
        MIGRATION.contains("IF affected > 0 OR removed_indexes > 0 THEN"),
        "search generation must only bump when the rollout changed something"
    );
    let enqueue = MIGRATION
        .find("INSERT INTO context69.document_metadata_payload_sync (document_id, payload_key)")
        .expect("outbox enqueue");
    assert!(
        enqueue < guarded_end,
        "only affected documents enqueue payload cleanup jobs"
    );
}

#[test]
fn migration_reads_the_existing_source_authority_and_never_rewrites_it() {
    for fragment in [
        "v.metadata_json IS NOT DISTINCT FROM (d.metadata_json - 'extraction')",
        "AND v.title IS NOT DISTINCT FROM d.title",
        "AND v.summary IS NOT DISTINCT FROM d.summary",
        "AND v.source_uri IS NOT DISTINCT FROM d.source_uri",
        "btrim(v.body_text) = ''",
        "NOT EXISTS (SELECT 1 FROM context69.document_chunks c WHERE c.document_id = d.id)",
        "c.record_hash <> d.record_hash",
    ] {
        assert!(
            MIGRATION.contains(fragment),
            "canonical-source guard missing: {fragment}"
        );
    }

    // History, canonical snapshots, chunks, imports, and task rows are never
    // deleted or hash-rewritten by the migration.
    for forbidden in [
        "DELETE FROM context69.documents",
        "DELETE FROM context69.document_versions",
        "DELETE FROM context69.document_chunks",
        "DELETE FROM context69.document_results",
        "DELETE FROM context69.tasks",
        "DELETE FROM context69.task_items",
        "DELETE FROM context69.task_attempts",
        "SET record_hash",
    ] {
        assert!(
            !MIGRATION.contains(forbidden),
            "migration must not rewrite or delete source history: {forbidden}"
        );
    }
}

#[test]
fn migration_fails_closed_on_malformed_or_unsafe_cohorts() {
    for fragment in [
        "jsonb_typeof(d.metadata_json -> 'extraction') <> 'object'",
        "entry.key NOT IN ('stock.news.v1', 'stock.disclosure.v1')",
        "jsonb_typeof(entry.value) <> 'object'",
        "octet_length(entry.value::text) + 4096 > 262144",
        "JOIN context69.library_file_documents mapping ON mapping.document_id = d.id",
        "file.metadata_json ? 'extraction' OR mapping.section_metadata_json ? 'extraction'",
        "item.status IN ('queued', 'running', 'waiting')",
        "job.status IN ('pending', 'running')",
        "affected data exists but no active admin can own the imported records",
        "RAISE EXCEPTION 'context69 enrichment retirement blocked: document % %'",
    ] {
        assert!(
            MIGRATION.contains(fragment),
            "fail-closed guard missing: {fragment}"
        );
    }
    // The document guards and imports are wrapped in one `affected > 0` block,
    // so a database without affected data needs no admin and mutates no
    // document, while the independent label/index convergence still runs.
    assert!(
        MIGRATION.contains("IF affected > 0 THEN"),
        "document work must be guarded by the affected count"
    );
    assert!(
        !MIGRATION.contains("IF affected = 0 THEN"),
        "an empty cohort must not return early and skip independent cleanup"
    );
    let guarded = MIGRATION
        .find("IF affected > 0 THEN")
        .expect("guarded block");
    let admin_lookup = MIGRATION.find("min(u.id)").expect("admin lookup");
    let document_strip = MIGRATION
        .find("SET metadata_json = d.metadata_json - 'extraction'")
        .expect("metadata strip");
    assert!(
        guarded < admin_lookup && guarded < document_strip,
        "the admin lookup and strip belong inside the guarded block"
    );
}

#[test]
fn imported_rows_are_deterministic_and_verified_before_the_strip() {
    for fragment in [
        "'context69.enrichment-import'",
        "'imported-enrichment'",
        "jsonb_build_array(d.id, entry.key, d.record_hash, entry.value)::text",
        "sha256(convert_to(",
        "'sqlx-retire:'",
        "min(u.id)",
        "WHERE u.is_admin",
        "AND u.disabled_at IS NULL",
        "ON CONFLICT (document_id, author_user_id, producer, idempotency_key) DO NOTHING",
        "existing imported result has a different envelope",
        "r.result = e.result",
        "imported result count mismatch",
        "'workflow_version', 'context69-enrichment-migration-20261017080707'",
        "'run_id', 'context69-enrichment-retirement-sqlx'",
    ] {
        assert!(
            MIGRATION.contains(fragment),
            "import contract fragment missing: {fragment}"
        );
    }

    let import_insert = MIGRATION
        .find("INSERT INTO context69.document_results")
        .expect("import insert");
    let strip = MIGRATION
        .find("SET metadata_json = d.metadata_json - 'extraction'")
        .expect("metadata strip");
    assert!(
        import_insert < strip,
        "imported results must be durable before the source key is removed"
    );
}

#[test]
fn migration_removes_only_the_retired_key_label_and_index_ownership() {
    for fragment in [
        "UPDATE context69.documents d",
        "SET metadata_json = d.metadata_json - 'extraction'",
        "AND d.metadata_json ? 'extraction'",
        "IF stripped <> affected THEN",
        "UPDATE context69.tasks SET failure_stage = NULL WHERE failure_stage = 'extraction'",
        "UPDATE context69.task_items SET failure_stage = NULL WHERE failure_stage = 'extraction'",
        "DELETE FROM context69.metadata_index_definitions",
        "field_path = 'extraction' OR field_path LIKE 'extraction.%'",
        "INSERT INTO context69.document_metadata_payload_sync (document_id, payload_key)",
        "'metadata_index.extraction'",
        "INSERT INTO context69.search_generations (scope, generation, updated_at)",
        "SET generation = context69.search_generations.generation + 1",
    ] {
        assert!(
            MIGRATION.contains(fragment),
            "retirement step missing: {fragment}"
        );
    }
}
