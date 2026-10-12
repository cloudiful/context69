#!/usr/bin/env bash
# Thin wrapper around the Rust backend test runner. The database target is
# resolved privately from CONTEXT69_TEST_DATABASE_URL or the repository .env
# file; it is never taken as an argument.
#
# Every argument is forwarded verbatim, so the destructive schema reset is
# reached by asking for it here explicitly:
#
#   ./scripts/test-backend.sh --preflight           # read-only, report the scope
#   ./scripts/test-backend.sh --authorize-reset      # reset, migrate, then run
#
# The wrapper never adds --authorize-reset on your behalf; the consent is yours
# to type, and the two lines above are the only supported entry points.
set -euo pipefail
repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
exec cargo run \
  --quiet \
  --manifest-path "$repo_root/Cargo.toml" \
  -p context69-test-support \
  --bin context69-backend-test-runner \
  -- "$@"