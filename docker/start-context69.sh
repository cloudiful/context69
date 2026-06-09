#!/usr/bin/env bash
set -euo pipefail

/usr/local/bin/context69 serve &
app_pid=$!

nginx -g 'daemon off;' &
nginx_pid=$!

cleanup() {
    kill "$app_pid" "$nginx_pid" 2>/dev/null || true
    wait "$app_pid" 2>/dev/null || true
    wait "$nginx_pid" 2>/dev/null || true
}

trap cleanup EXIT INT TERM

wait -n "$app_pid" "$nginx_pid"
exit $?
