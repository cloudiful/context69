-- Storable code bytes and chunk text: the content validation PostgreSQL can
-- actually evaluate (issue #681 work unit 3B2 correction).
--
-- Additive only. Two objects are replaced in place and nothing else is
-- touched: no table, column, index, or trigger is altered, and no applied
-- migration is rewritten. The blob check `chk_git_generation_blobs_text` keeps
-- calling `context69.git_blob_text_is_safe`, so correcting the function is what
-- makes storable bytes acceptable, and the chunk check keeps its name so the
-- rule a writer is held to is the same rule the rows were validated under.
--
-- The rule a stored byte string has to satisfy is one thing: it decodes as
-- UTF-8, and therefore holds no NUL byte. PostgreSQL cannot also express the
-- NUL half as a `text` comparison, because a `text` value cannot contain a NUL
-- at all: building one with `convert_from(decode('00', 'hex'), 'UTF8')` raises
-- `invalid byte sequence for encoding "UTF8": 0x00`, which inside a function
-- reports every input as unsafe and inside a CHECK aborts the writing
-- statement. The decode is thus the whole test. A byte string holding 0x00
-- cannot survive `convert_from(..., 'UTF8')` either, so a successful conversion
-- proves both halves at once, and the handler still reports unstorable bytes
-- instead of failing the write.
--
-- `IMMUTABLE` and `STRICT` are kept, so the predicate stays usable from the
-- CHECK constraint and from an index expression.
CREATE OR REPLACE FUNCTION context69.git_blob_text_is_safe(content BYTEA)
RETURNS BOOLEAN
LANGUAGE plpgsql
IMMUTABLE
STRICT
AS $$
BEGIN
    -- convert_from raises for malformed bytes and for an embedded NUL, so the
    -- handler below is what turns a rejected row into a constraint violation
    -- rather than into an aborted statement.
    PERFORM convert_from(content, 'UTF8');
    RETURN TRUE;
EXCEPTION
    WHEN others THEN
        RETURN FALSE;
END
$$;

-- A stored chunk is non-empty UTF-8 text within the 16 KiB ceiling. The
-- validity of its bytes already comes from the blob it was cut from, and `text`
-- cannot hold a NUL, so these two predicates are the whole enforceable content
-- rule for a chunk. The swap validates the rows already stored and takes one
-- brief exclusive lock on the chunk table.
ALTER TABLE context69.git_generation_chunks
    DROP CONSTRAINT chk_git_generation_chunks_text,
    ADD CONSTRAINT chk_git_generation_chunks_text
        CHECK (chunk_text <> ''
               AND octet_length(chunk_text) <= 16 * 1024);
