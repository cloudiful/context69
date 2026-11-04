-- Delete the fixture documents of one owned group (issue 1172 P5A test cleanup).
--
-- The group id is the fixture's own unique group, so this never reaches an
-- unrelated document. It runs only after the fixture's execution grants were
-- quiesced and its tasks were deleted, and it replaces the file-metadata-based
-- cleanup that missed a seeded canonical document whose metadata carries no
-- `library_file_id`.
DELETE FROM context69.documents
WHERE group_id = $1
