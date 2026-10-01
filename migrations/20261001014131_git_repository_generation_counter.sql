-- Per-repository generation counter (issue #681 work unit 3B1).
--
-- The next generation number used to be derived from
-- MAX(generation_number) inside the same statement that locked the repository
-- row. Every part of one statement reads one snapshot, so a start that waited
-- for the lock still computed its number from a snapshot taken before the lock
-- holder committed, and two concurrent starts produced the same number instead
-- of consecutive ones.
--
-- The counter lives on the repository row and is incremented by the same
-- statement that inserts the generation, so a start that waits for the row
-- re-reads the committed counter and takes the next number. Allocation and
-- insertion share one transaction, so a start that fails consumes no number.
--
-- Additive only: no existing column is reinterpreted and no applied migration
-- changes. The backfill seeds the counter from the generations already stored,
-- so numbering continues where the stored history left off and the first start
-- after this migration can never collide with a number already taken.

ALTER TABLE context69.git_repository_sources
    ADD COLUMN generation_counter BIGINT NOT NULL DEFAULT 0;

-- Existing repositories resume from their highest stored generation.
UPDATE context69.git_repository_sources s
SET generation_counter = COALESCE(
    (
        SELECT MAX(g.generation_number)
        FROM context69.git_repository_generations g
        WHERE g.repository_key = s.repository_key
    ),
    0
);

ALTER TABLE context69.git_repository_sources
    ADD CONSTRAINT chk_git_repository_sources_generation_counter
        CHECK (generation_counter >= 0);
