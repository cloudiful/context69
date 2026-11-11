//! The acquisition credential seam.
//!
//! A credential is a decrypted read token whose only permitted use is one
//! provider request header. The type deliberately has no `Display`, no
//! serialization, and a redacted `Debug`, so it cannot be logged, projected, or
//! stored by accident. Callers must name the exposure at the call site.
//!
//! Loading a sealed credential is the caller's responsibility: this module
//! carries the value across the provider seam but never opens the secret store.
//! A provider call without a credential is public acquisition; a connection
//! that requires one must fail closed rather than silently downgrade to public
//! access.

use std::fmt;

use context69_secret_crypto::SecretValue;
use reqwest::header::HeaderValue;

use crate::domain_errors::DomainError;

/// One decrypted provider read credential.
pub(crate) struct GitAccessCredential(SecretValue);

impl GitAccessCredential {
    /// Wrap one decrypted provider read credential.
    pub(crate) fn new(value: SecretValue) -> Self {
        Self(value)
    }

    /// The plaintext token bytes, exposed only at the request-header call site.
    pub(crate) fn expose(&self) -> &[u8] {
        self.0.expose()
    }

    /// The only header form this credential may take.
    ///
    /// An invalid header value maps to a stable error and never echoes the
    /// token. The returned value is used once at the request call site and is
    /// never logged.
    pub(crate) fn bearer_header(&self) -> anyhow::Result<HeaderValue> {
        let mut value = Vec::with_capacity(7 + self.expose().len());
        value.extend_from_slice(b"Bearer ");
        value.extend_from_slice(self.expose());
        HeaderValue::from_bytes(&value)
            .map_err(|_| DomainError::invalid_argument("git_credential_invalid").into())
    }
}

impl fmt::Debug for GitAccessCredential {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GitAccessCredential")
            .field("value", &"<redacted>")
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use context69_secret_crypto::SecretValue;

    use super::GitAccessCredential;

    #[test]
    fn a_credential_never_appears_in_debug_output() {
        let credential =
            GitAccessCredential::new(SecretValue::from_plaintext(b"super-secret-pat".to_vec()));
        let rendered = format!("{credential:?}");
        assert!(rendered.contains("redacted"), "{rendered}");
        assert!(!rendered.contains("super-secret-pat"), "{rendered}");
        assert_eq!(credential.expose(), b"super-secret-pat");
    }

    #[test]
    fn a_credential_yields_only_a_bearer_header() {
        let credential =
            GitAccessCredential::new(SecretValue::from_plaintext(b"super-secret-pat".to_vec()));
        let header = credential.bearer_header().expect("valid header");
        assert_eq!(header.to_str().expect("ascii"), "Bearer super-secret-pat");
        let invalid =
            GitAccessCredential::new(SecretValue::from_plaintext(b"bad\nheader".to_vec()));
        let error = invalid.bearer_header().expect_err("invalid header");
        assert!(error.to_string().contains("git_credential_invalid"));
        assert!(!error.to_string().contains("bad"));
    }
}
