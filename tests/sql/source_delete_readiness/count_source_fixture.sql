SELECT
    (SELECT count(*) FROM context69.source_configs WHERE source_key = $1) AS "sources!",
    (SELECT count(*) FROM context69.documents WHERE source_key = $1) AS "documents!",
    (
        SELECT count(*)
        FROM context69.document_chunks chunk
        JOIN context69.documents document ON document.id = chunk.document_id
        WHERE document.source_key = $1
    ) AS "chunks!"
