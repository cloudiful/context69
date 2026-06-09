# API Reference

## Main Endpoints

- `GET /healthz`
- `GET /openapi.json`
- `POST /v1/search`
- `GET /v1/documents/{document_id}`
- source and settings management endpoints under `/v1/*`

## OpenAPI

Export the OpenAPI document:

```bash
cargo run -- export-openapi
```

Generated output path:

```text
frontend/openapi/context69.openapi.json
```
