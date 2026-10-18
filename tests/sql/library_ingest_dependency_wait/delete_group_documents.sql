DELETE FROM context69.documents
WHERE metadata_json->>'library_file_id' IN (
    SELECT id::text
    FROM context69.library_files
    WHERE group_id = $1
)
