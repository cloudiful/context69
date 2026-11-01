-- Files whose released source cannot supply the admission that claims them.
--
-- A stored file or text task reads its own stored bytes, so a released source
-- is unavailable and must be refused. A URL task downloads the current source
-- URI again, so released cached bytes are still rerunnable as long as the file
-- carries a downloadable URL. Any other state stays refused.
SELECT count(*)::bigint AS "count!"
FROM context69.library_files
WHERE group_id = $1
  AND id = ANY($2)
  AND source_released_at IS NOT NULL
  AND (
      $3::text <> 'url'
      OR source_uri IS NULL
      OR source_uri !~* '^https?://'
  )
