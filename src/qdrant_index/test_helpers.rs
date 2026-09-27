//! Test-only `QdrantIndex` constructors enabled exclusively through the
//! `integration-test-helpers` Cargo feature. The feature is declared in
//! `Cargo.toml` and is not part of any default feature set, so production
//! builds (`cargo build`, `cargo build --release`, and the deployed binary)
//! never see these methods (no feature, no symbol). Other integration tests
//! that do not enable the feature behave the same way.

use anyhow::Result;
use qdrant_client::Qdrant;

use super::QdrantIndex;

impl QdrantIndex {
    /// Builds a `QdrantIndex` against an arbitrary gRPC endpoint without
    /// performing the `ensure_collection` round trip used by
    /// `QdrantIndex::connect`. Production code MUST keep calling
    /// `QdrantIndex::connect` so the collection boot path stays intact;
    /// this constructor exists solely for the issue 43 phase 0
    /// reproduction fixture and any future test that needs a deterministic
    /// cleanup failure against an unreachable gRPC target.
    pub fn for_test_unreachable(
        url: &str,
        collection_name: &str,
        dimensions: usize,
    ) -> Result<Self> {
        let client = Qdrant::from_url(url).build()?;
        Ok(Self {
            client,
            collection_name: collection_name.to_string(),
            dimensions,
        })
    }

    /// Noop Qdrant for checkpoint tests: all vector operations succeed without
    /// network. Collection name must be exactly `test-noop` to trigger the
    /// bypass; production code never uses this name.
    pub fn for_test_noop(collection_name: &str, dimensions: usize) -> Result<Self> {
        // Still build a client so the struct is valid, but methods will
        // short-circuit before using it when the collection is `test-noop`.
        let client = Qdrant::from_url("http://127.0.0.1:1").build()?;
        Ok(Self {
            client,
            collection_name: collection_name.to_string(),
            dimensions,
        })
    }
}
