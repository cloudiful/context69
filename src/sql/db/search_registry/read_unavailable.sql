-- The durable derived-index readiness flag of the registry epoch row.
--
-- It is exactly the readiness predicate the search snapshot carries: a
-- non-idle rebuild barrier or a durably invalid document index. Maintenance
-- writers use it to defer without reading the whole registry.
SELECT (rebuild_state <> 'idle' OR document_index_invalid) AS "unavailable!"
FROM context69.search_registry_epoch
WHERE id = 1
