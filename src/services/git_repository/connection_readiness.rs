//! Offline Git provider connection readiness (issue #681 phase 4B2).
//!
//! Readiness is a pure projection of already-persisted, non-secret metadata:
//! the connection mode, whether a read-credential reference is present, and
//! whether the connection is disabled. It never opens ciphertext, reads a
//! secret value, calls a provider, or derives installation identity or
//! transport of its own.

use crate::contracts::sources::{
    GitConnectionMode, GitConnectionReadiness, GitConnectionReadinessResponse,
};
use crate::db::StoredGitProviderConnection;

/// Derives readiness from the facts the persisted row already carries.
///
/// Disabled always wins. Otherwise the mode decides: public reads need
/// nothing, token reads need a stored read-credential reference, and
/// installation reads stay incomplete until a later phase persists an
/// installation identity.
pub(crate) fn readiness(
    mode: GitConnectionMode,
    has_read_credential: bool,
    disabled: bool,
) -> GitConnectionReadiness {
    if disabled {
        return GitConnectionReadiness::Disabled;
    }
    match mode {
        GitConnectionMode::Public => GitConnectionReadiness::Public,
        GitConnectionMode::Token if has_read_credential => GitConnectionReadiness::Token,
        GitConnectionMode::Token | GitConnectionMode::Installation => {
            GitConnectionReadiness::Incomplete
        }
    }
}

/// Projects one stored connection into the outward readiness response.
///
/// Only the connection's own non-secret fields are read; the credential,
/// webhook, and App-private-key references are deliberately ignored, so no
/// secret-store key or secret value can cross the response.
pub(crate) fn response(connection: &StoredGitProviderConnection) -> GitConnectionReadinessResponse {
    let has_read_credential = connection.credential_secret_key.is_some();
    let disabled = connection.disabled_at.is_some();
    GitConnectionReadinessResponse {
        connection_key: connection.connection_key.clone(),
        mode: connection.mode,
        readiness: readiness(connection.mode, has_read_credential, disabled),
        has_read_credential,
        disabled,
    }
}
