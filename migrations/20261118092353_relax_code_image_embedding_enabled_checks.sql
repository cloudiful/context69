-- The optional code and image channels store sparse overrides.
--
-- A NULL or blank column is an override that inherits the document embedding
-- configuration, so an enabled channel no longer has to repeat the endpoint,
-- the model pair, the dimensions, the timeout, or the image collection in its
-- own row. The two "enabled" checks demanded exactly that repetition and are
-- the only constraints that stand in the way, so they are dropped here.
--
-- Every other constraint stays: a stored override that is present is still a
-- real value, and a stored numeric override is still positive. The checks are
-- dropped, never rewritten, and no applied migration is edited.
ALTER TABLE context69.runtime_code_embedding_settings
    DROP CONSTRAINT IF EXISTS runtime_code_embedding_settings_enabled_check;

ALTER TABLE context69.runtime_image_embedding_settings
    DROP CONSTRAINT IF EXISTS runtime_image_embedding_settings_enabled_check;
