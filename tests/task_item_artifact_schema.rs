//! Issue #975 P1: authored-schema contract for task-item artifacts.
//!
//! These tests do not connect to a database. They inspect the new migration
//! and the compiled migrator so the additive schema, the exact kind/encoding
//! pairs, and the durable old-object release intent can be reviewed before the
//! migration is ever applied. Fixture-backed trigger and reference behavior
//! belongs to the later phase that introduces artifact writers.

use std::path::{Path, PathBuf};

use context69::db::{MIGRATOR, RETIREMENT_VERSION};

const MIGRATION_SUFFIX: &str = "_task_item_artifacts.sql";

fn artifact_migration() -> (PathBuf, i64, String) {
    let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
    let matches: Vec<PathBuf> = std::fs::read_dir(&directory)
        .expect("read the migrations directory")
        .map(|entry| entry.expect("read a migration entry").path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.ends_with(MIGRATION_SUFFIX))
        })
        .collect();
    assert_eq!(
        matches.len(),
        1,
        "exactly one task-item artifact migration must exist"
    );

    let path = matches.into_iter().next().expect("artifact migration");
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .expect("migration file name");
    let version = file_name
        .strip_suffix(MIGRATION_SUFFIX)
        .expect("migration suffix")
        .parse::<i64>()
        .expect("migration version prefix");
    let sql = std::fs::read_to_string(&path).expect("read the artifact migration");
    (path, version, sql)
}

fn table_definition(sql: &str) -> &str {
    let start = sql
        .find("CREATE TABLE context69.task_item_artifacts (")
        .expect("artifact table is created");
    let end = sql[start..]
        .find("\n);")
        .map(|offset| start + offset)
        .expect("artifact table definition ends");
    &sql[start..end]
}

#[test]
fn the_new_migration_is_compiled_after_the_pinned_retirement() {
    let (_, version, _) = artifact_migration();
    assert!(
        version > RETIREMENT_VERSION,
        "the artifact migration must sort after the immutable retirement migration"
    );
    let compiled = MIGRATOR
        .iter()
        .find(|migration| migration.version == version)
        .expect("the artifact migration is compiled into the shared migrator");
    assert!(
        compiled.description.contains("task item artifacts"),
        "the compiled migration keeps its authored description"
    );
}

#[test]
fn the_reference_table_is_narrow_and_points_at_both_owners() {
    let (_, _, sql) = artifact_migration();
    let table = table_definition(&sql);

    for fragment in [
        "item_id UUID NOT NULL",
        "REFERENCES context69.task_items(id) ON DELETE CASCADE",
        "kind TEXT NOT NULL",
        "storage_object_id UUID NOT NULL",
        "REFERENCES context69.library_storage_objects(id) ON DELETE RESTRICT",
        "encoding TEXT NOT NULL",
        "created_at TIMESTAMPTZ NOT NULL DEFAULT now()",
        "PRIMARY KEY (item_id, kind)",
    ] {
        assert!(
            table.contains(fragment),
            "reference table fragment missing: {fragment}"
        );
    }

    // Registry metadata stays authoritative in library_storage_objects.
    for forbidden in [
        "group_id",
        "sha256",
        "size_bytes",
        "storage_backend",
        "object_key",
        "byte_size",
    ] {
        assert!(
            !table.contains(forbidden),
            "the artifact relation must not duplicate registry metadata: {forbidden}"
        );
    }

    assert!(sql.contains(
        "CREATE INDEX idx_task_item_artifacts_storage_object_id\n    ON context69.task_item_artifacts (storage_object_id);"
    ));
}

#[test]
fn only_the_two_versioned_kind_encoding_pairs_are_allowed() {
    let (_, _, sql) = artifact_migration();
    let table = table_definition(&sql);

    assert!(table.contains("(kind = 'download' AND encoding = 'raw-v1')"));
    assert!(table.contains("(kind = 'sections' AND encoding = 'ingest-sections-json-v1')"));
    assert_eq!(sql.matches("kind = '").count(), 2);
    assert_eq!(sql.matches("encoding = '").count(), 2);
}

#[test]
fn deleting_or_retargeting_a_reference_enqueues_the_old_object() {
    let (_, _, sql) = artifact_migration();

    for fragment in [
        "CREATE OR REPLACE FUNCTION context69.enqueue_task_item_artifact_release()",
        "INSERT INTO context69.library_storage_object_cleanup",
        "(object_id, group_id, sha256, object_key, storage_backend)",
        "FROM context69.library_storage_objects AS object",
        "JOIN context69.groups AS owner ON owner.id = object.group_id",
        "WHERE object.id = OLD.storage_object_id",
        "ON CONFLICT DO NOTHING",
        "RETURN CASE WHEN TG_OP = 'DELETE' THEN OLD ELSE NEW END",
        "AFTER DELETE ON context69.task_item_artifacts",
        "AFTER UPDATE OF storage_object_id ON context69.task_item_artifacts",
        "NEW.storage_object_id IS NOT DISTINCT FROM OLD.storage_object_id",
    ] {
        assert!(
            sql.contains(fragment),
            "release-intent contract fragment missing: {fragment}"
        );
    }

    // Only real registry identities are copied; no token or key is forged.
    for identity in [
        "object.id",
        "object.group_id",
        "object.sha256",
        "object.object_key",
        "object.storage_backend",
    ] {
        assert!(
            sql.contains(identity),
            "the intent must copy the old registry identity: {identity}"
        );
    }

    // A staged object keeps its lease and stays owned by the staging sweep.
    assert!(sql.contains("AND object.staging_lease_until IS NULL"));
}

#[test]
fn the_migration_does_not_touch_payloads_or_existing_object_rows() {
    let (_, _, sql) = artifact_migration();

    for forbidden in [
        "ALTER TABLE",
        "UPDATE context69.task_items",
        "DELETE FROM context69.task_items",
        "UPDATE context69.library_storage_objects",
        "DELETE FROM context69.library_storage_objects",
        "INSERT INTO context69.library_storage_objects",
        "payload",
        "DROP ",
    ] {
        assert!(
            !sql.contains(forbidden),
            "the additive schema migration must not mutate existing data: {forbidden}"
        );
    }
}
