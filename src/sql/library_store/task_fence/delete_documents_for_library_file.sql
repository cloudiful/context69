-- The documents of one library file, deleted inside the guarded write of a
-- running task item (issue 1020 P3B).
--
-- The delete runs on the connection of the guarded transaction, so the
-- documents and the grant that authorized the cleanup commit together and a
-- worker that lost its segment removes nothing.
DELETE FROM context69.documents
WHERE metadata_json->>'library_file_id' = $1
