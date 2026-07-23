INSERT INTO context69.document_metadata_values
    (index_id, document_id, ordinal, keyword_value, integer_value, float_value, boolean_value, datetime_value)
SELECT index_id, document_id, ordinal, NULL, NULL, NULL, NULL, value
FROM unnest($1::uuid[], $2::bigint[], $3::integer[], $4::timestamptz[])
    AS values_row(index_id, document_id, ordinal, value)
