-- Serialize Docling poll reservations across processes (issue #118
-- poll-limits). Uses a dedicated advisory key so poll claims never contend
-- with submit admission (727335732). Held only for the short check-and-
-- reserve transaction, never across the Docling HTTP request.
SELECT pg_advisory_xact_lock(727335733)
