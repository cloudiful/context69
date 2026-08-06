SELECT cleanup_enabled,
       retention_days,
       updated_at
FROM context69.task_maintenance_settings
WHERE singleton
