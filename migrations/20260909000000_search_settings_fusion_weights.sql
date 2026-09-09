ALTER TABLE context69.search_settings
    ADD COLUMN vector_weight REAL NOT NULL DEFAULT 0.55,
    ADD COLUMN keyword_weight REAL NOT NULL DEFAULT 0.35;
