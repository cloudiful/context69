//! Provider-neutral Git acquisition foundation (issue #681 phase 3A).
//!
//! This module exposes the validated inputs, hard bounds, injectable
//! transport, and the [`RepositoryAcquirer`] surface later source, task, and
//! index phases build their scheduling and persistence on. The only concrete
//! provider here is a strictly bounded public GitHub client: it resolves a
//! ref, lists one recursive tree, and fetches individual blobs. It never
//! executes repository content, invokes Git, follows redirects, accepts
//! credentials, or exposes provider response bodies through errors.
//!
//! It also carries the pure content preparation the lexical index is built on
//! (issue #681 work unit 3B2): path-based language classification, and code text
//! that is validated and cut into verbatim, line-anchored chunks. Both are
//! lexical and deterministic, so a stored manifest and its chunks are
//! reproducible from a tree listing and its blobs alone.

mod bounds;
mod classify;
mod code_text;
mod github_client;
mod github_schemas;
mod github_transport;
mod github_url;
mod model;
mod ref_path_safety;
mod resolve;

// The content preparation below is reached through its own modules until the
// snapshot orchestration of work unit 3B3 schedules it; re-exporting it here
// would only add an unused import in the meantime.
pub(crate) use github_client::{GitHubAcquirer, RepositoryAcquirer};
pub(crate) use github_transport::{GitHttpTransport, GitHubApiTransport, TransportResponse};
pub(crate) use github_url::GitHubRepoCoordinates;
pub(crate) use model::{BlobContent, GitAcquisitionLimits, TreeFileEntry, TreeListing};
pub(crate) use ref_path_safety::{SafeRef, SafeSha, SafeTreePath};

#[cfg(test)]
#[path = "module_tests.rs"]
mod module_tests;
