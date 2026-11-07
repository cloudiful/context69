-- A library PDF that has not been ingested yet, so the actual driver reaches
-- the blocking Docling arm (issue 21 C3).
--
-- The shared legacy fixture hardcodes an ingest status the current contract no
-- longer parses, so this fixture owns the exact resumable shape: a failed
-- conversion with no sections and a group-scoped library file.
INSERT INTO context69.library_files (
    id,
    group_id,
    filename,
    media_type,
    size_bytes,
    sha256,
    storage_rel_path,
    ingest_status,
    error_message,
    ingested_at,
    visibility
)
VALUES ($1, $2, $3, 'application/pdf', 10, 'abc', '/objects/abc', 'failed', NULL, NULL, 'public')
