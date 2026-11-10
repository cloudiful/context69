SELECT pg_advisory_unlock(
    hashtextextended('context69.library_file_metadata_mirror:' || $1::uuid::text, 0)
) AS "unlocked!"
