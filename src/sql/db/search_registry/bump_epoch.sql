UPDATE context69.search_registry_epoch
SET epoch = context69.search_registry_epoch.epoch + 1,
    updated_at = now()
WHERE id = 1
RETURNING epoch
