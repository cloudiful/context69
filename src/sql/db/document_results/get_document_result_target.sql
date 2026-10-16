SELECT
    d.record_hash
FROM context69.documents d
INNER JOIN context69.groups g ON g.id = d.group_id
WHERE d.id = $1
  AND d.group_id = $2
  AND (g.visibility = 'public' OR d.group_id = ANY($3))
  AND ($4::text IS NULL OR g.full_path = $4)
