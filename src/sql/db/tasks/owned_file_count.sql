-- Count how many of the requested file ids exist in the owning group.
-- Callers compare this with the distinct requested count so a batch is
-- rejected atomically before any advisory lock or active-task lookup.
SELECT count(*) AS "count!"
FROM context69.library_files
WHERE group_id = $1
  AND id = ANY($2)
