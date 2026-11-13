-- Live execution lease of the domain mirror (issue 1020 P4A).
--
-- The handover refuses to touch rows while a worker still holds a live lease on
-- an item this database owns. This statement reads only the domain mirror; the
-- library half is evaluated by the queue capability in the caller's transaction,
-- so no task-table SQL remains here.
SELECT (EXISTS (
           SELECT 1
           FROM context69.task_items
           WHERE status = 'running'
             AND lease_until > clock_timestamp()
       )) AS "domain_live!"
