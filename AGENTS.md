# Context69 Notes

- Frontend PrimeVue work: before introducing or changing PrimeVue components, props, slots, pass-through config, or theme behavior, consult `docs/primevue-llms-full.txt`.
- Prefer PrimeVue's documented composition surfaces and theme tokens before adding custom overrides.
- Route-aware navigation UI should follow the named routes in `frontend/src/router/index.ts`; keep breadcrumb and sidebar logic aligned with those route names.
- Frontend API contracts, path parameters, request bodies, and response types must come from `frontend/src/generated/openapi.ts`. Do not use `any`, handwritten schema fallbacks, duplicated request/response interfaces, or manual response annotations to replace generated OpenAPI types.
- When the backend API contract changes, run `cd frontend && bun run generate:api:from-backend`. This exports backend OpenAPI, then regenerates frontend types in order. Include both `frontend/openapi/context69.openapi.json` and `frontend/src/generated/openapi.ts` in the same change.
- Before handing off frontend/API contract changes, run `cd frontend && bun run build:with-api` plus the affected Vitest files; run the full frontend suite when shared behavior changes. Do not rely on stale or uncommitted generated OpenAPI or component declaration files to satisfy type checking.
