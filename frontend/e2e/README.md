# End-to-end browser tests

[Playwright Test](https://playwright.dev/) suites for the React frontend. Vitest
stays the component/unit runner and Rust stays the API/integration runner.

## Default mock suite

```sh
cd frontend
bun run test:e2e
```

`playwright.config.ts` builds nothing itself: it serves the existing `dist/`
through `vite preview` on the owned port `5201`, with a fixed UTC browser. Every
`/v1` request is answered by the generated-contract mocks in
`e2e/react/transport.ts`, so the suite needs no backend, database or external
service. Run `bun run build` first when the preview would otherwise be stale.

`e2e/react/**` covers the product routes: shell/auth, search/streaming, the
document reader, groups/library/upload, settings (including manual Save,
storage failure and partial peer failure) and the processing queue.

The default suite takes no environment configuration and never leaves the
preview origin.
