-- Durable ownership of the rebuild target collection (issue 1212 R3).
--
-- A rebuild writes into a physical collection that it creates itself. The
-- barrier already records the name of that collection, but the name alone
-- cannot tell an owned collection from a foreign one: a crash between the
-- create and the next durable write leaves a collection that the barrier
-- records while no durable record proves that this attempt created it.
--
-- The marker closes that window. It is set immediately after the target was
-- created and it is cleared whenever the recorded target is released, so a
-- recorded target with the marker set is one this attempt created, and a
-- recorded target without it is either absent or ambiguous. An ambiguous
-- target is never adopted and never written: it is refused and the barrier is
-- cleared, so the next explicit attempt derives a fresh target name.
--
-- The column is additive and defaults to false, so an existing epoch row
-- describes a target that no attempt has created yet.
ALTER TABLE context69.search_registry_epoch
    ADD COLUMN rebuild_target_created BOOLEAN NOT NULL DEFAULT FALSE;
