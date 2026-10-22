# OpenAPI generator tooling

Locked tool package that produces `frontend/src/generated/openapi.ts`.

`openapi-typescript` reads the TypeScript compiler API through a JavaScript
dependency, which the native TypeScript 7 package no longer exposes. The
generator therefore runs in this isolated package on TypeScript 5.9.3 while the
application itself is type-checked with the root TypeScript 7.0.2 (`tsc`). The
TypeScript 5 runtime is a generator-only tool and never checks, transpiles or
resolves application sources.

Run it from `frontend/` through the root script, which installs this package
from its lockfile and then executes the generator:

```sh
bun run generate:api
bun run generate:api:from-backend
```

Both installs use the same release-age floor as CI. Dependency versions are
exact pins; update them with the lockfile in the same change.
