SELECT max_chars, overlap_chars, unit, tokenizer, tokenizer_sha256, max_tokens, overlap_tokens
FROM context69.runtime_chunking_settings
WHERE singleton = TRUE
