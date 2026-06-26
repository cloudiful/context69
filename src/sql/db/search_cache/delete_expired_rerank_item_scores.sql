DELETE FROM context69.rerank_item_scores
WHERE last_used_at < now() - make_interval(days => $1::int)
