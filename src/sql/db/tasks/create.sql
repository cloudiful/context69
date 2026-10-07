-- Parent task row for a new submission. `queued_count` starts equal to
-- `total_count`, and `stage` starts at the kind's entry stage so a queued task
-- reports the work it is about to do rather than a later pipeline stage.
-- `git_index` maps to `indexing` like `vector_rebuild`: without it the CASE
-- fell through to `finalize` and a freshly submitted git-indexing task was
-- labelled as already finished.
INSERT INTO context69.tasks (
    id, user_id, group_id, kind, group_path, source_key, origin, total_count, queued_count, stage
)
VALUES (
    $1,
    $2,
    $3,
    $4,
    $5,
    $6,
    $7,
    $8,
    $8,
    CASE $4
        WHEN 'url_batch' THEN 'download'
        WHEN 'file_batch' THEN 'storage'
        WHEN 'text_batch' THEN 'storage'
        WHEN 'source_sync' THEN 'sync'
        WHEN 'delete_batch' THEN 'delete'
        WHEN 'translation' THEN 'translation'
        WHEN 'vector_rebuild' THEN 'indexing'
        WHEN 'git_index' THEN 'indexing'
        ELSE 'finalize'
    END
)
RETURNING id
