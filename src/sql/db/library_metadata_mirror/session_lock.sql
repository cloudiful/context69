SELECT pg_try_advisory_lock(
    hashtextextended('context69.library_file_metadata_mirror:' || $1::uuid::text, 0)
) AS "locked!"
