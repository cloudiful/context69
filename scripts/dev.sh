#!/usr/bin/env bash

set -Eeuo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

usage() {
  cat <<'EOF'
Usage: ./scripts/dev.sh <backend|full|help>

Commands:
  backend   Build and start the Rust backend service only
  full      Build backend, refresh frontend OpenAPI types, then start backend + Vite
  help      Show this message

Config:
  Provide runtime config through the standard context69 config file or CONTEXT69_* env vars.
  RUST_LOG defaults to info if not set.
  Frontend listens on 0.0.0.0:5173 and proxies /v1, /healthz, and /openapi.json to the backend.
  Backend listens on 0.0.0.0:8096 and MCP on 0.0.0.0:8097.
  Browser sessions fall back to memory when Valkey is unavailable.
  CONTEXT69_AUTH__SESSION_VALKEY_URL is available as a break-glass override.
EOF
}

ensure_backend_binary() {
  printf 'Building backend binary once before startup...\n'
  cargo build --bin context69
}

ensure_frontend_sdk() {
  printf 'Exporting OpenAPI and generating frontend client before startup...\n'
  cargo run --bin context69 -- export-openapi
  (
    cd frontend
    bun run generate:api
  )
}

run_stack() {
  local mode="$1"

  ensure_backend_binary
  if [[ "$mode" == "full" ]]; then
    ensure_frontend_sdk
  fi

  printf 'Starting local dev stack (%s)\n' "$mode"
  printf '  backend             http://127.0.0.1:8096\n'
  printf '  backend OpenAPI     http://127.0.0.1:8096/openapi.json\n'
  printf '  backend health      http://127.0.0.1:8096/healthz\n'
  printf '  MCP HTTP            http://127.0.0.1:8097/mcp\n'
  printf '  auth sessions       runtime Settings Valkey (fallback redis://127.0.0.1:6379)\n'
  if [[ "$mode" == "full" ]]; then
    printf '  frontend            http://0.0.0.0:5173 (Vite will print LAN URLs)\n'
  fi
  printf 'Stop with Ctrl+C\n'

  exec bash scripts/dev-supervisor.sh "$mode"
}

case "${1:-help}" in
  backend|full)
    if (($# > 1)); then
      printf 'Unexpected argument: %s\n' "$2" >&2
      usage >&2
      exit 2
    fi
    run_stack "$1"
    ;;
  help)
    if (($# > 1)); then
      printf 'Unexpected argument: %s\n' "$2" >&2
      usage >&2
      exit 2
    fi
    usage
    ;;
  *)
    printf 'Unknown command: %s\n' "$1" >&2
    usage >&2
    exit 2
    ;;
esac
