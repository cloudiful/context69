-- Compares the path manifests of two index generations of one repository, one
-- bounded page of changed paths in path order.
--
-- $1 group_id, $2 repository_key, $3 compared-from generation_key,
-- $4 compared-to generation_key, $5 limit, $6 offset.
--
-- Both generation keys are the caller's claims, not the caller's facts, so this
-- statement checks them itself before it compares anything. `eligible_pair`
-- yields exactly one row when *both* keys name a generation of this repository
-- owned by this group whose status is comparable, and no row otherwise; the
-- comparison cross joins it, so a missing, foreign, or non-comparable key
-- produces no rows at all instead of a full outer join against an empty side
-- that would report every path of the other side as `added` or `deleted`. The
-- pair is also pinned to one repository, so two keys of different repositories
-- never meet.
--
-- A comparable generation is a *completed* one: the active `ready` generation
-- and a `superseded` generation that a newer snapshot replaced. Superseded is
-- exactly the state the default starting point lives in — once a repository
-- indexes a newer commit, the generation its checkpoint still names is
-- superseded, and refusing it would make "what changed since the last index"
-- permanently unanswerable. A `building` or `failed` snapshot is not comparable
-- at any point: its manifest is partial or abandoned, so a comparison over it
-- would report a truncated or invented diff.
--
-- The two manifests are then read separately and joined on the repository-relative
-- path with a full outer comparison, so a path only one side holds is still
-- compared instead of disappearing from the result. Each side keeps its own
-- group and repository filter, so neither side can widen the other.
--
-- `provider_blob_sha` is compared here and nowhere else. It decides the change
-- kind and the unchanged filter, and it is never selected: a path whose stored
-- content address is identical on both sides is omitted rather than reported, and
-- a modified path reports only that the two differ. No provider blob id can
-- therefore reach the row, the response, or a test.
--
-- `UNIQUE (generation_key, path)` makes each side path-unique, so the join emits
-- one row per path and the path order is a total order: the same comparison with
-- the same page window always returns the same rows. The order names the coalesced
-- expression rather than the output alias, because both join inputs expose a
-- `path` column and a bare name would be ambiguous.
WITH eligible_pair AS (
    SELECT TRUE AS pair_ready
    FROM context69.git_repository_generations g_from
    JOIN context69.git_repository_generations g_to
        ON g_to.repository_key = g_from.repository_key
    JOIN context69.git_repository_sources s ON s.repository_key = g_from.repository_key
    WHERE s.group_id = $1
      AND g_from.repository_key = $2
      AND g_from.generation_key = $3
      AND g_to.generation_key = $4
      AND g_from.status IN ('ready', 'superseded')
      AND g_to.status IN ('ready', 'superseded')
    LIMIT 1
),
from_side AS (
    SELECT
        f.path,
        f.file_key,
        f.language,
        f.byte_count,
        f.line_count,
        f.provider_blob_sha
    FROM context69.git_generation_files f
    JOIN context69.git_repository_generations g ON g.generation_key = f.generation_key
    JOIN context69.git_repository_sources s ON s.repository_key = g.repository_key
    WHERE s.group_id = $1
      AND g.repository_key = $2
      AND f.generation_key = $3
      AND g.status IN ('ready', 'superseded')
),
to_side AS (
    SELECT
        f.path,
        f.file_key,
        f.language,
        f.byte_count,
        f.line_count,
        f.provider_blob_sha
    FROM context69.git_generation_files f
    JOIN context69.git_repository_generations g ON g.generation_key = f.generation_key
    JOIN context69.git_repository_sources s ON s.repository_key = g.repository_key
    WHERE s.group_id = $1
      AND g.repository_key = $2
      AND f.generation_key = $4
      AND g.status IN ('ready', 'superseded')
)
SELECT
    COALESCE(b.path, a.path) AS "path!",
    CASE
        WHEN b.path IS NULL THEN 'added'
        WHEN a.path IS NULL THEN 'deleted'
        ELSE 'modified'
    END AS "change_kind!",
    b.file_key AS "before_file_key",
    b.language AS "before_language",
    b.byte_count AS "before_byte_count",
    b.line_count AS "before_line_count",
    a.file_key AS "after_file_key",
    a.language AS "after_language",
    a.byte_count AS "after_byte_count",
    a.line_count AS "after_line_count"
FROM (from_side b FULL JOIN to_side a ON a.path = b.path) CROSS JOIN eligible_pair p
-- `IS DISTINCT FROM` is the comparison itself: a missing side (an added or a
-- deleted path) is never equal to a stored content address, and two equal
-- addresses are never distinct, so exactly the unchanged paths drop out.
WHERE b.provider_blob_sha IS DISTINCT FROM a.provider_blob_sha
ORDER BY COALESCE(b.path, a.path) ASC
LIMIT $5 OFFSET $6
