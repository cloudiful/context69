DELETE FROM context69.document_metadata_values values_row
USING unnest($1::uuid[], $2::bigint[])
    AS requested(index_id, document_id)
WHERE values_row.index_id = requested.index_id
  AND values_row.document_id = requested.document_id
