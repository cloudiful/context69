# MCP

Context69 supports MCP over Streamable HTTP and stdio. The tool contract is
bounded so an agent can inspect a result first and request detail only when it
is needed.

## Tool selection

Use this sequence for most retrieval workflows:

1. `search_documents` for semantic or hybrid retrieval. It accepts only
   `query`, `group_path`, `source_key`, `locale`, `limit` (1-20, default 8),
   and `cursor`. It returns compact hits with title, snippet, source, time,
   document id, external id, and score, plus `next_cursor`/`has_more` for
   continuation. It deliberately accepts no `page`, no published window, no
   metadata filters, and no sort mode — structured filtering stays behind
   `query_documents`.
2. `get_document` for one document's bounded metadata and chunk window. Pass
   the returned `next_chunk_cursor` to fetch the next chunk page.
3. `get_documents` for several known document keys after search or structured
   query. It accepts 1-20 keys and keeps one response item per requested key
   with at most five chunks per detail item.
4. `query_documents` when the caller already knows the group and needs
   metadata filters, time filters, sorting, and cursor pagination
   (`limit` 1-20, omitted `limit` defaults to 20). It returns bounded
   summaries only, plus `next_cursor`/`has_more`.
5. `get_document_by_external_id` for an exact group/source/external-id lookup.
   It returns the first bounded chunk window (20 chunks).
6. `list_sources` for configured sources as safe summaries with cursor
   pagination (`limit` 1-100, default 50, plus `cursor`). It returns
   `group_path`, `source_key`, `display_name`, `description`, `visibility`,
   and `origin_status` only — connection names, base queries, origin messages,
   and database state are never disclosed over MCP.

`search` is not an alias. Use `search_documents` as the single search tool.
Metadata and full body text are intentionally absent from search results.

## Bounded output

- Search and structured query accept at most 20 results per call; an omitted
  structured-query `limit` defaults to the MCP-local 20 (the HTTP DTO default
  of 50 never applies to tool calls). Out-of-range limits are rejected with an
  `invalid_params` fix hint instead of being silently clamped.
- `get_documents` accepts a `keys` array of at most 20 entries (`maxItems: 20`
  in the tool schema); oversized key lists are rejected at deserialization and
  by validation with the same 1..=20 bound.
- Search snippets are capped at 600 characters; chunk texts at 4,000
  characters. These caps are declared as `maxLength` in the tool schemas.
- `get_document` returns at most 50 chunks per call (default 20).
- `get_documents` accepts at most 20 keys and returns at most five chunks per
  detail item.
- Source listing and resource enumeration return at most 100 entries per
  `list_sources` call and 50 entries per `list_resources` page.

Every `has_more: true` response carries a continuation token (`next_cursor`
or `next_chunk_cursor`). Resource listing honors the incoming
`PaginatedRequestParams.cursor` offset instead of restarting the enumeration.
A bounded inline response is used in v0.8; `file_first` is deliberately not
part of the contract yet.

## Examples

Search (all `search_documents` fields):

```json
{
  "query": "央行降准对银行股的影响",
  "limit": 8,
  "group_path": "research/news",
  "source_key": "news-pg",
  "locale": "zh-CN",
  "cursor": null
}
```

Continue a search with the returned cursor:

```json
{
  "query": "央行降准对银行股的影响",
  "limit": 8,
  "cursor": "eyJvZmZzZXQiOjgsInJlcmluayI6dHJ1ZX0"
}
```

Fetch the first chunk page:

```json
{
  "document_id": 421,
  "chunk_limit": 20
}
```

Continue with the `next_chunk_cursor` returned by the previous call:

```json
{
  "document_id": 421,
  "chunk_cursor": "20",
  "chunk_limit": 20
}
```

Structured query without `limit` (defaults to 20 results):

```json
{
  "group_path": "research/news",
  "query": {
    "source_key": "news-pg"
  }
}
```

Batch detail for known keys (at most 20 keys):

```json
{
  "group_path": "research/news",
  "request": {
    "keys": [
      {"source_key": "news-pg", "external_id": "art-1"},
      {"source_key": "news-pg", "external_id": "art-2"}
    ]
  }
}
```

List sources with cursor pagination:

```json
{
  "limit": 50
}
```

```json
{
  "limit": 50,
  "cursor": "50"
}
```

Parameter and filter errors return MCP `invalid_params` data with a `fix`
message. Invalid cursors are rejected the same way instead of silently
restarting the listing. Service failures include `retryable` and a suggested
action when the failure looks transient.

## Endpoints

When MCP is enabled, Streamable HTTP listens at:

```text
http://127.0.0.1:8097/mcp
```

Run stdio mode with:

```bash
cargo run -- mcp-stdio
```
