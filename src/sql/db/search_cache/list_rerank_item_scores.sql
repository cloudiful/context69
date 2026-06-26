UPDATE context69.rerank_item_scores
SET last_used_at = now()
WHERE rerank_model = $1
  AND query_hash = $2
  AND chunk_id = ANY($3)
RETURNING rerank_model, query_hash, query_text_trimmed, chunk_id, score
