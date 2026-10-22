-- Live execution leases of the two authorities (issue 1020 P4A).
--
-- The handover refuses to touch rows while a worker still holds a live lease on
-- an item this database owns: the domain mirror and the library attempt are
-- both read here, read-only, so a --workers-stopped flag is never taken as
-- proof of quiescence by itself. The library side is joined back to the items
-- through their captured segment, so only an attempt that still executes a live
-- item counts; attempts whose item is gone belong to no authority over this
-- task center. The library tables are read schema-qualified and are never
-- mutated by this tool.
SELECT (EXISTS (
           SELECT 1
           FROM context69.task_items
           WHERE status = 'running'
             AND lease_until > clock_timestamp()
       )) AS "domain_live!",
       (EXISTS (
           SELECT 1
           FROM cloudiful_tasks.attempts attempt
           JOIN context69.task_items item
             ON item.execution_task_id = attempt.task_id
           WHERE attempt.status = 'running'
             AND attempt.lease_expires_at > now()
       )) AS "library_live!"
