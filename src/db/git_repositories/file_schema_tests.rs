//! Schema and query invariants of the generation-scoped code content store
//! (issue #681 work unit 3B2).
//!
//! Raw bytes are stored once per (generation, blob id) with a size derived from
//! the bytes, every reference is confined to one generation, every stored value
//! is shape-checked rather than trusted from the caller, content is writable
//! only while a generation builds, an inconsistent manifest writes nothing at
//! all, and the lexical query answers from the activated generation alone with
//! intact code identifiers. The 3B1 metadata invariants live in
//! `schema_tests.rs`.

const CONTENT_MIGRATION_SQL: &str =
    include_str!("../../../migrations/20261001052706_git_repository_file_chunks.sql");
const REPLACE_FILES_SQL: &str =
    include_str!("../../sql/db/git_repository_files/replace_git_generation_files.sql");
const GET_FILE_SQL: &str =
    include_str!("../../sql/db/git_repository_files/get_git_generation_file.sql");
const LIST_FILES_SQL: &str =
    include_str!("../../sql/db/git_repository_files/list_git_generation_files.sql");
const REPLACE_CHUNKS_SQL: &str =
    include_str!("../../sql/db/git_repository_files/replace_git_file_chunks.sql");
const LIST_CHUNKS_SQL: &str =
    include_str!("../../sql/db/git_repository_files/list_git_generation_chunks.sql");
const LEXICAL_SEARCH_SQL: &str =
    include_str!("../../sql/db/git_repository_files/lexical_search_git_generation_chunks.sql");

/// The fragments each content table body must carry: deduplication in the blob
/// primary key, references confined to one generation, and shape checks on every
/// stored value.
const TABLE_SHAPES: &[(&str, &[&str])] = &[
    (
        "git_generation_blobs",
        &[
            "PRIMARY KEY (generation_key, provider_blob_sha)",
            "content BYTEA NOT NULL",
            "byte_count >= 0 AND byte_count = octet_length(content)",
            "provider_blob_sha ~ '^[0-9a-f]{40}$|^[0-9a-f]{64}$'",
            "byte_count <= 8 * 1024 * 1024",
            "CHECK (context69.git_blob_text_is_safe(content))",
        ],
    ),
    (
        "git_generation_files",
        &[
            "UNIQUE (generation_key, file_key)",
            "UNIQUE (generation_key, path)",
            "FOREIGN KEY (repository_key, generation_key)",
            "REFERENCES context69.git_repository_generations (repository_key, generation_key)",
            "FOREIGN KEY (generation_key, provider_blob_sha)",
            "REFERENCES context69.git_generation_blobs (generation_key, provider_blob_sha)",
            "path <> ''",
            "octet_length(path) <= 512",
            r"path !~ '(^/|\.\.|//|\\|/$)'",
            "path !~ '[[:cntrl:]]'",
            "language ~ '^[a-z0-9][a-z0-9_+#-]{0,31}$'",
            "byte_count <= 8 * 1024 * 1024",
        ],
    ),
    (
        "git_generation_chunks",
        &[
            "UNIQUE (generation_key, chunk_key)",
            "UNIQUE (file_key, chunk_index)",
            "FOREIGN KEY (generation_key, file_key)",
            "REFERENCES context69.git_generation_files (generation_key, file_key)",
            "ON DELETE CASCADE",
            "chunk_index >= 0",
            "start_line >= 1 AND end_line >= start_line",
            "strpos(chunk_text, convert_from(decode('00', 'hex'), 'UTF8')) = 0",
            "octet_length(chunk_text) <= 16 * 1024",
        ],
    ),
];

/// Every query that must answer for one owning group only.
const GROUP_SCOPED_QUERIES: &[&str] = &[
    REPLACE_FILES_SQL,
    GET_FILE_SQL,
    LIST_FILES_SQL,
    REPLACE_CHUNKS_SQL,
    LIST_CHUNKS_SQL,
    LEXICAL_SEARCH_SQL,
];

/// Every provenance column a lexical hit must project with its `!` override, so
/// a result can be attributed to the snapshot and lines it came from.
const HIT_COLUMNS: &str = "repository_key generation_key generation_number ref_name \
                           commit_sha group_visibility path start_line end_line chunk_text score";

/// Every fragment `sql` must contain for the guarantee `subject` to hold.
fn assert_fragments(sql: &str, fragments: &[&str], subject: &str) {
    for fragment in fragments {
        assert!(
            sql.contains(fragment),
            "{subject} must include {fragment:?}"
        );
    }
}

/// Executable statements in `sql`, without their comment lines, so a test can
/// assert that a name appears in a statement rather than only in the prose.
fn statements_only(sql: &str) -> String {
    sql.lines()
        .filter(|line| !line.trim_start().starts_with("--"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Statements in `sql`, ignoring comments, so a test can assert a file is one
/// atomic statement rather than a sequence.
fn statement_count(sql: &str) -> usize {
    statements_only(sql)
        .split(';')
        .filter(|statement| !statement.trim().is_empty())
        .count()
}

/// Body of a `CREATE TABLE IF NOT EXISTS` in `sql`, without the closing paren.
fn table_body<'a>(sql: &'a str, table: &str) -> &'a str {
    let marker = format!("CREATE TABLE IF NOT EXISTS context69.{table} (");
    let start = sql
        .find(&marker)
        .unwrap_or_else(|| panic!("migration must create {table}"));
    let rest = &sql[start + marker.len()..];
    let mut depth = 1_i32;
    for (index, character) in rest.char_indices() {
        match character {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return &rest[..index];
                }
            }
            _ => {}
        }
    }
    panic!("unterminated CREATE TABLE for {table}");
}

#[test]
fn every_content_table_is_shape_checked() {
    for (table, fragments) in TABLE_SHAPES {
        let body = table_body(CONTENT_MIGRATION_SQL, table);
        assert_fragments(body, fragments, table);
        assert!(
            !body.contains("visibility"),
            "{table} must not store a group visibility copy"
        );
        assert!(
            !body.contains("BYTEA") || *table == "git_generation_blobs",
            "{table} must not hold a second copy of the bytes"
        );
    }
    assert_eq!(
        TABLE_SHAPES.len(),
        3,
        "work unit 3B2 must add exactly the three generation-scoped tables"
    );
}

#[test]
fn content_ceilings_and_encoding_are_enforced_by_the_database() {
    assert_eq!(super::file_types::MAX_GIT_FILE_BYTES, 8 * 1024 * 1024);
    assert_eq!(
        super::file_types::MAX_GIT_GENERATION_BYTES,
        64 * 1024 * 1024
    );
    // An aggregate needs more than a check constraint: the trigger locks the
    // generation so concurrent writers cannot each stay under the ceiling, and
    // sums in an AFTER trigger, where the writing statement's own rows are
    // already visible. `chr(0)` is rejected by PostgreSQL, so the NUL test
    // decodes the byte, and invalid encoding fails the check rather than
    // aborting the writing statement.
    assert_fragments(
        CONTENT_MIGRATION_SQL,
        &[
            "stored_bytes > 64 * 1024 * 1024",
            "FOR UPDATE",
            "AFTER INSERT OR UPDATE ON context69.git_generation_blobs",
            "FROM context69.git_generation_blobs b\n    WHERE b.generation_key = NEW.generation_key",
            "CREATE OR REPLACE FUNCTION context69.git_blob_text_is_safe(content BYTEA)",
            "convert_from(content, 'UTF8')",
            "strpos(text_value, convert_from(decode('00', 'hex'), 'UTF8')) = 0",
            "WHEN others THEN\n        RETURN FALSE",
        ],
        "the storage budget and encoding guarantees",
    );
}

#[test]
fn every_content_operation_is_group_scoped() {
    for query in GROUP_SCOPED_QUERIES {
        assert_fragments(
            query,
            &["s.group_id = $1"],
            "every content operation must filter on the owning group",
        );
    }
    assert_fragments(
        LEXICAL_SEARCH_SQL,
        &[
            "JOIN context69.groups g ON g.id =",
            "g.visibility AS group_visibility",
        ],
        "the lexical query",
    );
}

#[test]
fn replace_statements_are_one_atomic_scoped_statement_each() {
    // Storing bytes, retiring files, and inserting a manifest is one statement,
    // so a rejected manifest stores nothing; likewise a file's chunks leave and
    // land together, so a file is never readable with two revisions mixed.
    assert_eq!(
        statement_count(REPLACE_FILES_SQL),
        1,
        "the manifest replacement"
    );
    assert_eq!(
        statement_count(REPLACE_CHUNKS_SQL),
        1,
        "the chunk replacement"
    );
    assert_fragments(
        REPLACE_FILES_SQL,
        &[
            "g.repository_key = $2",
            "g.generation_key = $3",
            "retained_files",
            "retired_files",
            "retired_blobs",
            "ON CONFLICT (generation_key, provider_blob_sha) DO NOTHING",
            "incoming.provider_blob_sha = f.provider_blob_sha",
            "incoming.line_count = f.line_count",
        ],
        "the manifest replacement",
    );
    assert_fragments(
        REPLACE_CHUNKS_SQL,
        &[
            "g.repository_key = $2",
            "f.generation_key = $3",
            "f.file_key = $4",
            "unnest($5::int[], $6::int[], $7::int[], $8::text[])",
        ],
        "the chunk replacement",
    );

    // A completed snapshot is immutable by construction, and the write path
    // refuses one before it runs.
    assert_fragments(
        CONTENT_MIGRATION_SQL,
        &[
            "CREATE OR REPLACE FUNCTION context69.git_require_building_generation()",
            "IF generation_status IS DISTINCT FROM 'building' THEN",
        ],
        "the writability guard",
    );
    for (table, _) in TABLE_SHAPES {
        assert_fragments(
            CONTENT_MIGRATION_SQL,
            &[&format!("BEFORE INSERT OR UPDATE ON context69.{table}")],
            "a content table trigger",
        );
    }
    for query in [REPLACE_FILES_SQL, REPLACE_CHUNKS_SQL] {
        assert_fragments(query, &["g.status = 'building'"], "a write statement");
    }
}

#[test]
fn an_inconsistent_manifest_writes_nothing_at_all() {
    assert_fragments(
        REPLACE_FILES_SQL,
        &[
            "SELECT 'blob_bytes'::TEXT AS kind",
            "SELECT 'duplicate_path'::TEXT",
        ],
        "the conflict report",
    );
    assert!(
        REPLACE_FILES_SQL
            .matches("NOT EXISTS (SELECT 1 FROM manifest_conflicts)")
            .count()
            >= 4,
        "every write must be skipped while a conflict is reported, so a refused \
         manifest leaves the generation as it was"
    );
    // Bytes a described file still needs are never removed, so the rows inserted
    // in the same statement always resolve their content.
    let retired_blobs = REPLACE_FILES_SQL
        .split("retired_blobs AS (")
        .nth(1)
        .expect("the statement retires bytes");
    assert_fragments(
        retired_blobs,
        &[
            "NOT EXISTS (\n          SELECT 1\n          FROM incoming_files incoming",
            "incoming.provider_blob_sha = b.provider_blob_sha",
        ],
        "the byte retirement",
    );
}

#[test]
fn lexical_code_search_serves_the_active_generation_with_intact_identifiers() {
    // Only an activated, ready generation answers, visibility comes from the
    // groups row, and the result is ordered and bounded.
    assert_fragments(
        LEXICAL_SEARCH_SQL,
        &[
            "FROM context69.git_repository_active_generations ag",
            "gen.status = 'ready'",
            "JOIN active_generation gen ON gen.generation_key = c.generation_key",
            "(r.group_visibility = 'public' OR r.group_id = ANY($8::bigint[]))",
            "ORDER BY \"score!\" DESC, path ASC, chunk_index ASC",
            "LIMIT $9",
            "lower(c.chunk_text) LIKE $4 ESCAPE '\\'",
            "cardinality($5::text[]) > 0",
            "FROM unnest($5::text[]) AS term(value)",
        ],
        "the lexical query",
    );
    for column in HIT_COLUMNS.split_whitespace() {
        assert!(
            LEXICAL_SEARCH_SQL.contains(&format!("{column} AS \"{column}!\"")),
            "a hit must project {column} so results stay attributable"
        );
    }
    assert_fragments(
        LEXICAL_SEARCH_SQL,
        &["END AS \"matched!\""],
        "a hit must say why it matched",
    );
    // The trigram index is an accelerator only: it exists where the extension
    // does, and the query stays correct on a sequential scan without it.
    let guarded = CONTENT_MIGRATION_SQL
        .split("DO $$")
        .nth(1)
        .expect("the trigram index is created behind an extension check");
    assert_fragments(
        guarded,
        &[
            "IF EXISTS (SELECT 1 FROM pg_extension WHERE extname = 'pg_trgm') THEN",
            "USING gin (lower(chunk_text) gin_trgm_ops)",
        ],
        "the guarded index creation",
    );
    assert!(
        !statements_only(LEXICAL_SEARCH_SQL).contains("gin_trgm_ops")
            && !LEXICAL_SEARCH_SQL.contains("@@"),
        "the query must stay correct on a sequential scan"
    );

    // Code is not prose: the caller's text is never re-tokenised or stemmed, and
    // the prose term index is not reused.
    let lower = LEXICAL_SEARCH_SQL.to_ascii_lowercase();
    for forbidden in "context69.document_chunks keyword_terms library_storage_objects \
                      regexp_replace regexp_split to_tsvector plainto_tsquery similarity("
        .split_whitespace()
    {
        assert!(
            !lower.contains(forbidden),
            "code identifiers must not be rewritten: found {forbidden}"
        );
    }
}
