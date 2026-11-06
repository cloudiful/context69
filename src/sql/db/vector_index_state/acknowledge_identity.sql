-- Acknowledge a confirmed same-space identity for the active vector index.
--
-- Only the active row may be rekeyed: an acknowledgement describes the
-- collection ordinary point writers use, so a row that a concurrent activation
-- already deactivated can never be rekeyed behind the live pointer's back.
UPDATE context69.vector_index_state
SET fingerprint = $1,
    embedding_base_url = $2,
    embedding_model = $3,
    dimensions = $4
WHERE collection_name = $5
  AND fingerprint = $6
  AND dimensions = $7
  AND active
