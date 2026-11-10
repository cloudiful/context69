//! Provider construction stays provider-neutral and origin-bound.

use std::sync::Arc;

use super::super::super::{ForgejoOrigin, GitAcquisitionLimits, support::NeverTransport};
use super::super::{ForgejoSnapshotProvider, GitHubSnapshotProvider, GitSnapshotProvider};

#[test]
fn the_public_provider_builds_a_credential_free_acquirer() {
    let provider = GitHubSnapshotProvider::new(Arc::new(NeverTransport));
    assert!(
        provider
            .acquirer(
                "https://github.com/octo/repo",
                GitAcquisitionLimits::default()
            )
            .is_ok(),
        "public acquisition must not need a credential"
    );
    assert!(
        provider
            .acquirer(
                "https://evil.invalid/octo/repo",
                GitAcquisitionLimits::default()
            )
            .is_err(),
        "the factory must reject a non-canonical GitHub URL"
    );
}

#[test]
fn the_forgejo_provider_binds_the_connection_origin() {
    let provider = ForgejoSnapshotProvider::new(
        Arc::new(NeverTransport),
        ForgejoOrigin::from_base_url("https://forgejo.example/api/v1").expect("origin"),
    );
    assert!(
        provider
            .acquirer(
                "https://forgejo.example/octo/repo",
                GitAcquisitionLimits::default()
            )
            .is_ok(),
        "public Forgejo acquisition needs no credential"
    );
    assert!(
        provider
            .acquirer(
                "https://evil.example/octo/repo",
                GitAcquisitionLimits::default()
            )
            .is_err(),
        "a repository outside the connection origin must be refused"
    );
}
