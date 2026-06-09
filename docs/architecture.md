# Architecture Notes

## High-Level Flow

1. Read source records from external systems
2. Normalize and chunk documents
3. Generate embeddings
4. Store metadata in PostgreSQL
5. Store vectors in Qdrant
6. Serve retrieval results over HTTP API and MCP

## Current Boundaries

- retrieval only
- built-in connector focus is PostgreSQL
- no answer-generation pipeline
- scheduler supports optional distributed coordination through Valkey

## Storage

- PostgreSQL stores application metadata, documents, and sync state
- Qdrant stores chunk vectors for retrieval
