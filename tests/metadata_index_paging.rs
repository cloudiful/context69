//! Regression coverage for the metadata-index rebuild paging path (issue #530
//! Task 2): `list_documents.sql` walks documents by `id > $3` with `LIMIT $4`,
//! and `DocumentStoreService::build_index` must consume every page before it
//! finishes an index.
//!
//! The cursor test needs only the database; the build test also needs a
//! `LibraryService`. Both run only when `CONTEXT69_TEST_DATABASE_URL` points at
//! a scratch database (migrations are applied automatically) and skip
//! otherwise.

#[path = "metadata_index_paging/support.rs"]
mod support;

#[path = "metadata_index_paging/cursor.rs"]
mod cursor;

#[path = "metadata_index_paging/build.rs"]
mod build;
