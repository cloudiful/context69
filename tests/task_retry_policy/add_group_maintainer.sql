-- The maintainer role a manual retry needs in the item's group.
INSERT INTO context69.group_memberships (group_id, user_id, role)
VALUES ($1, $2, 'maintainer')
ON CONFLICT DO NOTHING
