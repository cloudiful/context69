#!/usr/bin/env nu

def print-startup-summary [mode: string] {
  print $"Starting local dev stack \(($mode)\)"
  print "  backend             http://127.0.0.1:8096"
  print "  backend OpenAPI     http://127.0.0.1:8096/openapi.json"
  print "  backend health      http://127.0.0.1:8096/healthz"
  print "  MCP HTTP            http://127.0.0.1:8097/mcp"
  if $mode == "full" {
    print "  frontend            http://127.0.0.1:5173"
  }
  print "Stop with Ctrl+C"
}

def ensure-backend-binary [] {
  print "Building backend binary once before startup..."
  ^cargo build --bin context69
}

def ensure-frontend-sdk [] {
  print "Exporting OpenAPI and generating frontend client before startup..."
  ^cargo run --bin context69 -- export-openapi
  do {
    cd frontend
    ^bun run generate:api
  }
}

def run-stack [mode: string] {
  ensure-backend-binary
  if $mode == "full" {
    ensure-frontend-sdk
  }
  print-startup-summary $mode
  run-external bash scripts/dev-supervisor.sh $mode
}

def main [command?: string] {
  let action = ($command | default "help")

  match $action {
    "backend" => {
      run-stack "backend"
    }
    "full" => {
      run-stack "full"
    }
    "help" => {
      print "Usage: nu scripts/dev.nu <backend|full|help>"
      print "Commands:"
      print "  backend   Build and start the Rust backend service only"
      print "  full      Build backend, refresh frontend OpenAPI types, then start backend + Vite"
      print "  help      Show this message"
      print "Config:"
      print "  Provide runtime config through the standard context69 config file or CONTEXT69_* env vars"
      print "  RUST_LOG defaults to info if not set"
      print "  Frontend proxies /v1, /healthz, and /openapi.json to http://127.0.0.1:8096"
    }
    _ => {
      error make {
        msg: $"unknown command: ($action)"
        help: "Run `nu scripts/dev.nu help` for usage."
      }
    }
  }
}
