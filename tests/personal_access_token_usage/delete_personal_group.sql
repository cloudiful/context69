-- The personal group the authorization read created for this test's user.
DELETE FROM context69.groups
WHERE group_key = $1
