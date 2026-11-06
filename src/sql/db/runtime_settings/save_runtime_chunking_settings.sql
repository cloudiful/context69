INSERT INTO context69.runtime_chunking_settings (
    singleton,
    max_chars,
    overlap_chars,
    unit,
    tokenizer,
    max_tokens,
    overlap_tokens,
    updated_at
)
VALUES (TRUE, $1, $2, $3, $4, $5, $6, now())
ON CONFLICT (singleton) DO UPDATE
SET max_chars = EXCLUDED.max_chars,
    overlap_chars = EXCLUDED.overlap_chars,
    unit = EXCLUDED.unit,
    tokenizer = EXCLUDED.tokenizer,
    max_tokens = EXCLUDED.max_tokens,
    overlap_tokens = EXCLUDED.overlap_tokens,
    updated_at = now()
