SELECT
    groups.default_source_policy
FROM context69.groups
WHERE groups.id = $1
