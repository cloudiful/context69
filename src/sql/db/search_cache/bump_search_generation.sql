INSERT INTO context69.search_generations (scope, generation, updated_at)
VALUES ('global', 1, now())
ON CONFLICT (scope) DO UPDATE
SET generation = context69.search_generations.generation + 1,
    updated_at = now()
RETURNING generation
