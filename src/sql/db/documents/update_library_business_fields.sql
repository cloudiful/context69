-- update_library_business_fields.sql (issue 50, phase 2; issue 1138, P19)
--
-- Rewrite the business fields of one document under a compare-and-set on its
-- current record hash. $8 is the expected current hash the caller locked and
-- read in the same transaction; a mismatch updates nothing.
--
-- Returning the updated id lets the caller detect a CAS miss (zero rows) even
-- when the record hash does not change.
UPDATE context69.documents
SET external_id = $2,
    source_uri = $3,
    published_at = $4,
    updated_at_source = $5,
    metadata_json = $6,
    record_hash = $7,
    updated_at = now()
WHERE id = $1
  AND record_hash IS NOT DISTINCT FROM $8
RETURNING id
