-- Drop the derived original chunks before a bounded installation refills them.
--
-- Only `document_chunks` is reset: canonical `document_versions`, `documents`,
-- files, business metadata, translations and task history are untouched.
-- `RESTRICT` is the explicit non-cascading form: if any table ever gained an
-- inbound foreign key, this statement fails and rolls the reset transaction
-- back instead of silently widening the deletion scope.
TRUNCATE context69.document_chunks RESTRICT
