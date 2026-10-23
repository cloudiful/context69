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

## Queue performance profile

```sh
cd frontend
QUEUE_PERF_LABEL=baseline \
QUEUE_PERF_BASE_SHA=<commit> \
QUEUE_PERF_OUTPUT=$HOME/.cache/opencode/context69-queue-perf/1014/baseline \
bun run test:e2e:queue-perf
```

It runs the dedicated `e2e/processing-queue-perf.spec.ts` scenarios against
synthetic 100-row queue fixtures on port `5188`: table baseline, 100 updates at
5 ms plus a 1000-update burst, one and ten expanded rows after two item pages,
and a 100-target current-page recovery with bounded concurrency. Measured
probes are written to `metrics.json` under the configured output directory; no
real backend or database is involved.

## Configuration

| Variable | Default | Purpose |
| --- | --- | --- |
| `QUEUE_PERF_LABEL` | required | Short run label such as `baseline` |
| `QUEUE_PERF_BASE_SHA` | required | Source revision the run belongs to |
| `QUEUE_PERF_OUTPUT` | required | Absolute output directory under `~/.cache/opencode/context69-queue-perf/1014/` |

The default suite takes no environment configuration and never leaves the
preview origin.
