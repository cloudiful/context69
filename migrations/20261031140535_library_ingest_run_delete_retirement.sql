-- Before-delete retirement of an unresolved library ingest run (issue 1138 P22).
--
-- The terminal-status trigger of P18 retires an unresolved run when its item
-- reaches `failed` or `cancelled`. A deletion never passes through such a
-- status: the cascade that removes the run history would leave a staging
-- registry entry behind, so its staged point ids stay reserved forever and a
-- late write of a staged point would never be excluded from the search.
--
-- This trigger runs before the row is removed, so the item row is still
-- locked and the canonical lock order (item, epoch, registry, run) holds. The
-- helper moves every staged id into the permanent retired set and bumps the
-- epoch in the same transaction, and it never deletes a tombstone. A resolved
-- or switched run is a success and stays untouched.

CREATE OR REPLACE FUNCTION context69.retire_library_ingest_run_on_item_delete()
RETURNS trigger
LANGUAGE plpgsql
AS $$
BEGIN
    PERFORM context69.retire_unresolved_library_ingest_run(OLD.id);
    RETURN OLD;
END;
$$;

CREATE TRIGGER task_items_retire_library_ingest_run_on_delete
BEFORE DELETE ON context69.task_items
FOR EACH ROW
EXECUTE FUNCTION context69.retire_library_ingest_run_on_item_delete();
