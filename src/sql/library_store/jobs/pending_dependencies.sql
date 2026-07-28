WITH pending AS (
    SELECT job.requires_docling
    FROM context69.library_ingest_jobs job
    JOIN context69.library_files file ON file.id = job.file_id
    WHERE job.status = 'pending'
)
SELECT
    EXISTS (SELECT 1 FROM pending) AS "has_pending!: bool",
    EXISTS (
        SELECT 1
        FROM pending
        WHERE requires_docling
    ) AS "requires_docling!: bool",
    EXISTS (
        SELECT 1
        FROM pending
    ) AND $1::TEXT = 's3' AS "requires_s3!: bool";
