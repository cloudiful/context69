-- Generation-scoped Git code content: deduplicated blobs, path manifest, and
-- line-anchored chunks (issue #681 work unit 3B2).
--
-- Additive only: no existing table, column, or constraint is rewritten, and no
-- applied migration changes. A generation row stays metadata; the exact
-- content of one snapshot lives here, scoped to the generation that holds it.
--
-- Code is not prose. Raw bytes are stored once per (generation, provider blob
-- id) so an unchanged file costs nothing, file rows map a safe repository path
-- to those bytes, and chunk rows carry verbatim UTF-8 text with inclusive
-- 1-based line ranges so a result can cite the lines it came from. Whitespace,
-- line endings, and punctuation are stored exactly as the commit held them:
-- nothing is stemmed, lower-cased, or split into prose search terms.
--
-- Content may only be written while a generation is building, and every content
-- row is confined to one generation by composite foreign keys, so a chunk can
-- never be reached through a file of another snapshot and a completed snapshot
-- is immutable.
--
-- Bytes stay in these generation-scoped tables rather than
-- context69.library_storage_objects: that table's reference and orphan sweeps
-- only account for two other tables, so code bytes written there would never be
-- reclaimed.

-- Immutable predicate for storable code bytes: the content must decode as
-- UTF-8 and must not contain NUL, so a chunk can be cut from the stored bytes
-- and read back unchanged. It returns FALSE instead of raising, which is what a
-- CHECK constraint needs to reject the row.
CREATE OR REPLACE FUNCTION context69.git_blob_text_is_safe(content BYTEA)
RETURNS BOOLEAN
LANGUAGE plpgsql
IMMUTABLE
STRICT
AS $$
DECLARE
    text_value TEXT;
BEGIN
    -- convert_from raises on malformed input, so a blob that is not UTF-8
    -- reaches the handler and is reported as unstorable instead of aborting the
    -- writing statement.
    text_value := convert_from(content, 'UTF8');
    -- A NUL is exactly one 0x00 byte in UTF-8, so searching the decoded text
    -- for it is an exact byte test.
    RETURN strpos(text_value, convert_from(decode('00', 'hex'), 'UTF8')) = 0;
EXCEPTION
    WHEN others THEN
        RETURN FALSE;
END
$$;

-- Raw bytes, stored once per (generation, provider blob id). The primary key is
-- the deduplication: a file whose bytes repeat inside one snapshot costs one
-- row, and the same content in two generations stays independent, so a
-- superseded snapshot keeps serving its own bytes.
CREATE TABLE IF NOT EXISTS context69.git_generation_blobs (
    generation_key UUID NOT NULL
        REFERENCES context69.git_repository_generations(generation_key) ON DELETE CASCADE,
    provider_blob_sha TEXT NOT NULL,
    byte_count BIGINT NOT NULL,
    content BYTEA NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT git_generation_blobs_pkey
        PRIMARY KEY (generation_key, provider_blob_sha),
    -- Object ids are fixed-length lowercase hex, so a stored id can never carry
    -- path structure into a later request.
    CONSTRAINT chk_git_generation_blobs_sha
        CHECK (provider_blob_sha ~ '^[0-9a-f]{40}$|^[0-9a-f]{64}$'),
    -- The declared size is the stored size: a caller cannot overstate a blob to
    -- slip past the per-file ceiling.
    CONSTRAINT chk_git_generation_blobs_count
        CHECK (byte_count >= 0 AND byte_count = octet_length(content)),
    -- Per-file ceiling, mirrored by MAX_GIT_FILE_BYTES in
    -- src/db/git_repositories/file_types.rs.
    CONSTRAINT chk_git_generation_blobs_size
        CHECK (byte_count <= 8 * 1024 * 1024),
    -- Valid UTF-8 without NUL, the encoding guarantee chunk text relies on.
    CONSTRAINT chk_git_generation_blobs_text
        CHECK (context69.git_blob_text_is_safe(content))
);

-- The per-generation byte budget, mirrored by MAX_GIT_GENERATION_BYTES in
-- src/db/git_repositories/file_types.rs. A CHECK cannot see the other rows of a
-- generation, so the aggregate is enforced here: the generation row is locked
-- first, so two concurrent writers cannot each stay under the ceiling, and the
-- sum runs in an AFTER trigger, where bytes inserted earlier by the same
-- statement are already visible and a multi-blob manifest is charged in full.
-- Raising aborts the writing statement, so a rejected manifest stores nothing.
-- The sum is an index-only scan over one generation's rows, and that generation
-- is bounded by the ceiling itself, so the cost stays proportional to one
-- snapshot build.
CREATE OR REPLACE FUNCTION context69.git_enforce_generation_byte_budget()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
DECLARE
    stored_bytes BIGINT;
BEGIN
    PERFORM 1
    FROM context69.git_repository_generations g
    WHERE g.generation_key = NEW.generation_key
    FOR UPDATE;

    SELECT COALESCE(SUM(b.byte_count), 0)
    INTO stored_bytes
    FROM context69.git_generation_blobs b
    WHERE b.generation_key = NEW.generation_key;

    IF stored_bytes > 64 * 1024 * 1024 THEN
        RAISE EXCEPTION
            'git generation content exceeds the % byte budget: %',
            64 * 1024 * 1024,
            stored_bytes
            USING ERRCODE = 'check_violation';
    END IF;
    RETURN NULL;
END
$$;

-- Content belongs to a building generation only. The generation row is locked
-- for the same reason the byte budget locks it: without the lock a concurrent
-- completion could activate the generation between this check and the write.
CREATE OR REPLACE FUNCTION context69.git_require_building_generation()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
DECLARE
    generation_status TEXT;
BEGIN
    SELECT g.status
    INTO generation_status
    FROM context69.git_repository_generations g
    WHERE g.generation_key = NEW.generation_key
    FOR UPDATE;

    IF generation_status IS DISTINCT FROM 'building' THEN
        RAISE EXCEPTION
            'git generation content is only writable while the generation is building'
            USING ERRCODE = 'check_violation';
    END IF;
    RETURN NEW;
END
$$;

-- Path manifest: one safe repository path per generation, mapped to that
-- generation's bytes.
CREATE TABLE IF NOT EXISTS context69.git_generation_files (
    file_key UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    generation_key UUID NOT NULL,
    repository_key UUID NOT NULL,
    provider_blob_sha TEXT NOT NULL,
    path TEXT NOT NULL,
    language TEXT NOT NULL,
    byte_count BIGINT NOT NULL,
    line_count BIGINT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    -- Reference targets for the child row, so a chunk can address a file of its
    -- own generation only.
    CONSTRAINT uq_git_generation_files_key
        UNIQUE (generation_key, file_key),
    -- One manifest row per path per snapshot: two generations may hold the same
    -- path with different content, one generation never holds it twice.
    CONSTRAINT uq_git_generation_files_path
        UNIQUE (generation_key, path),
    CONSTRAINT fk_git_generation_files_generation
        FOREIGN KEY (repository_key, generation_key)
        REFERENCES context69.git_repository_generations (repository_key, generation_key)
        ON DELETE CASCADE,
    -- The bytes must belong to this generation, so a file can never point at
    -- another snapshot's blob. No delete action is declared: replacing a
    -- manifest retires the files and their unreferenced bytes in one statement,
    -- and NO ACTION lets that statement resolve both orderings, where a
    -- restrictive action would fail the retire-then-insert sequence.
    CONSTRAINT fk_git_generation_files_blob
        FOREIGN KEY (generation_key, provider_blob_sha)
        REFERENCES context69.git_generation_blobs (generation_key, provider_blob_sha),
    -- Stored paths stay repository-relative and free of traversal, separator,
    -- and control-character hazards, matching the acquisition path policy.
    CONSTRAINT chk_git_generation_files_path
        CHECK (path <> ''
               AND octet_length(path) <= 512
               AND path !~ '(^/|\.\.|//|\\|/$)'
               AND path !~ '[[:cntrl:]]'),
    -- A bounded language token rather than a closed enumeration, so a new
    -- language is classified without a migration while the stored value stays a
    -- short, shape-checked identifier.
    CONSTRAINT chk_git_generation_files_language
        CHECK (language ~ '^[a-z0-9][a-z0-9_+#-]{0,31}$'),
    CONSTRAINT chk_git_generation_files_counts
        CHECK (byte_count >= 0
               AND byte_count <= 8 * 1024 * 1024
               AND line_count >= 0)
);

-- Line-anchored chunks of one file: verbatim text plus the inclusive line range
-- it was cut from.
CREATE TABLE IF NOT EXISTS context69.git_generation_chunks (
    chunk_key UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    generation_key UUID NOT NULL,
    file_key UUID NOT NULL,
    chunk_index INTEGER NOT NULL,
    start_line INTEGER NOT NULL,
    end_line INTEGER NOT NULL,
    chunk_text TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT uq_git_generation_chunks_key
        UNIQUE (generation_key, chunk_key),
    -- One row per position inside a file: a replacement cannot leave two texts
    -- claiming the same index.
    CONSTRAINT uq_git_generation_chunks_index
        UNIQUE (file_key, chunk_index),
    -- The file must be one of this chunk's own generation, so chunk text can
    -- never be attributed to a different snapshot than its file.
    CONSTRAINT fk_git_generation_chunks_file
        FOREIGN KEY (generation_key, file_key)
        REFERENCES context69.git_generation_files (generation_key, file_key)
        ON DELETE CASCADE,
    CONSTRAINT chk_git_generation_chunks_index
        CHECK (chunk_index >= 0),
    -- Inclusive 1-based range: a citation can name the exact lines, and a chunk
    -- can never be empty or run backwards.
    CONSTRAINT chk_git_generation_chunks_lines
        CHECK (start_line >= 1 AND end_line >= start_line),
    -- Chunk text is bounded, non-empty, and NUL-free, matching the encoding
    -- guarantee of the blob it was cut from. The ceiling is above the chunker's
    -- own bound so the bound can move without a migration.
    CONSTRAINT chk_git_generation_chunks_text
        CHECK (chunk_text <> ''
               AND octet_length(chunk_text) <= 16 * 1024
               AND strpos(chunk_text, convert_from(decode('00', 'hex'), 'UTF8')) = 0)
);

CREATE TRIGGER trg_git_generation_blobs_byte_budget
AFTER INSERT OR UPDATE ON context69.git_generation_blobs
FOR EACH ROW EXECUTE FUNCTION context69.git_enforce_generation_byte_budget();

CREATE TRIGGER trg_git_generation_blobs_building_generation
BEFORE INSERT OR UPDATE ON context69.git_generation_blobs
FOR EACH ROW EXECUTE FUNCTION context69.git_require_building_generation();

CREATE TRIGGER trg_git_generation_files_building_generation
BEFORE INSERT OR UPDATE ON context69.git_generation_files
FOR EACH ROW EXECUTE FUNCTION context69.git_require_building_generation();

CREATE TRIGGER trg_git_generation_chunks_building_generation
BEFORE INSERT OR UPDATE ON context69.git_generation_chunks
FOR EACH ROW EXECUTE FUNCTION context69.git_require_building_generation();

-- Chunk reads of one generation: the lexical query walks a snapshot, and the
-- composite foreign key check on file deletion walks the same rows.
CREATE INDEX IF NOT EXISTS idx_git_generation_chunks_generation
    ON context69.git_generation_chunks (generation_key, file_key);

-- The lexical query matches with ILIKE and is correct on a sequential scan, so
-- the GIN index is only an accelerator and is created exactly where the
-- trigram extension exists (the 0006 search-settings pattern). Without the
-- extension the query still returns the same rows, just without the index.
DO $$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_extension WHERE extname = 'pg_trgm') THEN
        EXECUTE 'CREATE INDEX IF NOT EXISTS git_generation_chunks_text_trgm_idx ON context69.git_generation_chunks USING gin (lower(chunk_text) gin_trgm_ops)';
    END IF;
END
$$;
