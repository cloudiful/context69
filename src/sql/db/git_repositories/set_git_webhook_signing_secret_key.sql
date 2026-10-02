-- Narrows the signing-secret reference of one group-owned webhook registration.
--
-- A registration hangs off a repository source, so group ownership is resolved
-- through that join: a registration of another group's repository matches no
-- row, and the caller learns the update did not land rather than having
-- repointed a hook the group does not own. The join is an existence check
-- rather than an `UPDATE ... FROM` so the parameter order stays group, then
-- repository, then the new reference.
--
-- `active` is left alone: sealing or rotating a signing secret is not a
-- lifecycle change, and a rotation must not silently re-enable a hook an
-- operator deactivated.
UPDATE context69.git_webhook_registrations r
SET signing_secret_key = $3,
    updated_at = now()
WHERE r.repository_key = $2
  AND EXISTS (
      SELECT 1
      FROM context69.git_repository_sources s
      WHERE s.repository_key = r.repository_key
        AND s.group_id = $1
  )
