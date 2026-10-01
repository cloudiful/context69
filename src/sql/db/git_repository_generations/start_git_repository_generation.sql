-- Starts the next generation of a repository owned by $1.
--
-- The number is allocated by incrementing the repository's own counter and
-- inserting with the value that increment returned, in one statement. The
-- increment re-reads the row version the lock holder committed, so two
-- concurrent starts take consecutive numbers instead of the same one, and
-- there is no aggregate over the generations table whose snapshot could
-- predate the lock. Allocation and insertion share a transaction, so a start
-- that fails consumes no number.
WITH allocated AS (
    UPDATE context69.git_repository_sources
    SET generation_counter = generation_counter + 1,
        updated_at = now()
    WHERE group_id = $1
      AND repository_key = $2
    RETURNING repository_key, generation_counter
)
INSERT INTO context69.git_repository_generations (
    repository_key,
    generation_number,
    ref_name,
    commit_sha,
    index_profile
)
SELECT
    allocated.repository_key,
    allocated.generation_counter,
    $3,
    $4,
    $5
FROM allocated
RETURNING
    repository_key,
    generation_key,
    generation_number,
    ref_name,
    commit_sha,
    index_profile,
    status,
    file_count,
    excluded_file_count,
    total_bytes,
    error_code,
    started_at,
    completed_at,
    created_at,
    updated_at
