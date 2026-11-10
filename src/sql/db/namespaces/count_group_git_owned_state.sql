-- Counts the Git repository ownership a group delete would drop (issue #28
-- phase P13).
--
-- Deleting a group cascades to its descendant groups and repository sources.
-- Those source rows carry the only durable record of remote points, webhook
-- registrations, and index generations, so the whole subtree is counted before
-- any delete. A provider connection without a repository source owns no remote
-- points and does not block removal; it cascades with the group like any other
-- group-owned configuration.
--
-- $1 group_id.
WITH RECURSIVE subtree AS (
    SELECT id
    FROM context69.groups
    WHERE id = $1

    UNION ALL

    SELECT child.id
    FROM context69.groups child
    JOIN subtree parent ON child.parent_group_id = parent.id
)
SELECT count(*) AS owned
FROM context69.git_repository_sources
WHERE group_id IN (SELECT id FROM subtree)
