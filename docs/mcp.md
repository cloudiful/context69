# MCP

Context69 supports MCP in two modes:

- HTTP Streamable MCP
- stdio MCP

## HTTP MCP

When MCP is enabled in config, `cargo run` starts the HTTP MCP server alongside the REST API.

Default endpoint:

```text
http://127.0.0.1:8097/mcp
```

## stdio MCP

Run:

```bash
cargo run -- mcp-stdio
```

Use this mode when integrating with clients that expect a stdio-based MCP server process.
