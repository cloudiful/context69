SELECT record_hash
FROM context69.documents
WHERE id = $1
FOR UPDATE
