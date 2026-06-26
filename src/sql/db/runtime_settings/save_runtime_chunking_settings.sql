INSERT INTO context69.runtime_chunking_settings (
    singleton,
    max_chars,
    overlap_chars,
    updated_at
)
VALUES (TRUE, $1, $2, now())
ON CONFLICT (singleton) DO UPDATE
SET max_chars = EXCLUDED.max_chars,
    overlap_chars = EXCLUDED.overlap_chars,
    updated_at = now()
