# MCP

Context69 supports MCP over Streamable HTTP and stdio. The tool contract is
bounded so an agent can inspect a result first and request detail only when it
is needed.

## Tool selection

Use this sequence for most retrieval workflows:

1. `search_documents` for semantic or hybrid retrieval. It returns compact
   hits with title, snippet, source, time, document id, external id, and score.
2. `get_document` for one document's metadata and chunks. Pass the returned
   `next_chunk_cursor` to fetch the next chunk page.
3. `get_documents` for several known document keys after search or structured
   query. It keeps one response item per requested key.
4. `query_documents` when the caller already knows the group and needs
   metadata filters, time filters, sorting, and cursor pagination. It returns
   summaries only.
5. `get_document_by_external_id` for an exact group/source/external-id lookup.

`search` is not an alias. Use `search_documents` as the single search tool.
Metadata and full body text are intentionally absent from search results.

## Bounded output

- Search and structured query accept at most 20 results per call.
- `get_document` returns at most 50 chunks and truncates each chunk to 4,000
  characters.
- `get_documents` accepts at most 20 keys and returns at most five chunks per
  detail item.
- Source listing is capped at 100 entries.

Responses include `truncated`, `has_more`, `next_cursor`, or
`next_chunk_cursor` where another request can continue the result. A bounded
inline response is used in v0.8; `file_first` is deliberately not part of the
contract yet.

## Examples

Search:

```json
{
  "query": "央行降准对银行股的影响",
  "limit": 8,
  "group_path": "research/news"
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

Parameter and filter errors return MCP `invalid_params` data with a `fix`
message. Service failures include `retryable` and a suggested action when the
failure looks transient.

## Endpoints

When MCP is enabled, Streamable HTTP listens at:

```text
http://127.0.0.1:8097/mcp
```

Run stdio mode with:

```bash
cargo run -- mcp-stdio
```
