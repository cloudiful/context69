-- Whether the owned schema currently holds the base relation every migrated
-- Context69 schema has, so a reset cycle can be observed end to end.
SELECT to_regclass('context69.groups') IS NOT NULL AS "present!"
