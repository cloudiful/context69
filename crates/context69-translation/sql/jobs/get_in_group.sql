SELECT j.*
FROM context69.document_translation_jobs j
JOIN context69.documents d ON d.id = j.document_id
WHERE j.id = $1 AND d.group_id = $2
