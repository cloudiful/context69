CREATE TABLE IF NOT EXISTS context69.search_generations (
    scope TEXT PRIMARY KEY,
    generation BIGINT NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

INSERT INTO context69.search_generations (scope, generation)
VALUES ('global', 0)
ON CONFLICT (scope) DO NOTHING;

CREATE TABLE IF NOT EXISTS context69.rerank_item_scores (
    rerank_model TEXT NOT NULL,
    query_hash TEXT NOT NULL,
    query_text_trimmed TEXT NOT NULL,
    chunk_id UUID NOT NULL,
    score REAL NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_used_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (rerank_model, query_hash, chunk_id)
);

CREATE INDEX IF NOT EXISTS rerank_item_scores_last_used_at_idx
    ON context69.rerank_item_scores (last_used_at);
