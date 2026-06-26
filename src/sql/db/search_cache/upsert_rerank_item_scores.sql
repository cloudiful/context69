INSERT INTO context69.rerank_item_scores (
    rerank_model,
    query_hash,
    query_text_trimmed,
    chunk_id,
    score,
    created_at,
    last_used_at
)
SELECT
    item.rerank_model,
    item.query_hash,
    item.query_text_trimmed,
    item.chunk_id,
    item.score,
    now(),
    now()
FROM unnest(
    $1::text[],
    $2::text[],
    $3::text[],
    $4::uuid[],
    $5::real[]
) AS item(rerank_model, query_hash, query_text_trimmed, chunk_id, score)
ON CONFLICT (rerank_model, query_hash, chunk_id) DO UPDATE
SET score = EXCLUDED.score,
    query_text_trimmed = EXCLUDED.query_text_trimmed,
    last_used_at = now()
