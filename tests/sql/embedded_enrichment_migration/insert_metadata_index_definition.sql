INSERT INTO context69.metadata_index_definitions
    (index_id, group_id, source_key, field_path, data_type, value_kind, status)
VALUES ($1, $2, 'retire-migration-src', $3, 'keyword', 'scalar', $4)
RETURNING index_id
