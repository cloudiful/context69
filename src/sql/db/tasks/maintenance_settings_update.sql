INSERT INTO context69.task_maintenance_settings (
    singleton,
    cleanup_enabled,
    retention_days,
    updated_at
)
VALUES (
    TRUE,
    $1,
    $2,
    now()
)
ON CONFLICT (singleton) DO UPDATE
SET cleanup_enabled = EXCLUDED.cleanup_enabled,
    retention_days = EXCLUDED.retention_days,
    updated_at = now()
RETURNING cleanup_enabled, retention_days, updated_at
