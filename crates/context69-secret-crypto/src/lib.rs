//! Authenticated encryption for persisted reversible secrets.
//!
//! This crate is the leaf primitive behind the application-owned encrypted
//! secret store. It owns exactly three things and nothing else:
//!
//! 1. a **deployment-supplied master key** ([`MasterKey`]) that is parsed from
//!    configuration and is never written to storage, logs, or errors;
//! 2. a **purpose** ([`SecretPurpose`]) that names one owner of one secret, so a
//!    ciphertext sealed for one owner cannot be opened by another; and
//! 3. **versioned authenticated ciphertext** ([`SecretCipher`]) built from
//!    ChaCha20-Poly1305 with a fresh random nonce per seal, the purpose and key
//!    name bound as associated data, and the master-key version carried in the
//!    frame so a rotated key can be detected instead of silently mis-decrypted.
//!
//! It is a leaf on purpose. It depends on no database driver, no HTTP client,
//! and no other workspace crate, so a service crate can depend on the primitive
//! without depending on the application.
//!
//! ```
//! use context69_secret_crypto::{MasterKey, SecretCipher, SecretPurpose};
//!
//! let master = MasterKey::from_base64("AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=").unwrap();
//! let cipher = SecretCipher::new(master, 1);
//! let purpose = SecretPurpose::new("browser_session.signing_key").unwrap();
//!
//! let sealed = cipher.seal(&purpose, "browser_session_signing_key_v2", &[7_u8; 64]).unwrap();
//! assert!(!sealed.windows(7).any(|window| window == [7_u8; 7]));
//!
//! let opened = cipher.open(&purpose, "browser_session_signing_key_v2", &sealed).unwrap();
//! assert_eq!(opened.expose(), &[7_u8; 64]);
//! ```

use core::fmt;

use base64::{Engine as _, engine::general_purpose::STANDARD_PAD_INDIFFERENT};
use chacha20poly1305::{
    ChaCha20Poly1305,
    aead::{Aead, Generate, Key, KeyInit, Payload},
};
use zeroize::Zeroizing;

pub mod frame;

pub use frame::{
    FrameError, FrameMetadata, SecretFrame, frame_header, is_frame, is_valid_key_name,
    is_valid_purpose,
};

/// Master-key length in bytes.
pub const MASTER_KEY_LEN: usize = 32;

/// A deployment-provided master key.
///
/// The key is held in a zeroizing buffer and is never `Display`, `Debug`, or
/// otherwise printable: an audit note, a log line, or an error message can
/// therefore mention the key without revealing it. Nothing in this crate
/// persists it, and the value reaches the process only through configuration.
#[derive(Clone)]
pub struct MasterKey(Zeroizing<[u8; MASTER_KEY_LEN]>);

impl MasterKey {
    /// Wraps raw key material.
    ///
    /// # Errors
    ///
    /// Returns [`SecretCipherError::InvalidMasterKeyLength`] unless exactly
    /// [`MASTER_KEY_LEN`] bytes are supplied.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, SecretCipherError> {
        let key: [u8; MASTER_KEY_LEN] = bytes
            .try_into()
            .map_err(|_| SecretCipherError::InvalidMasterKeyLength(bytes.len()))?;
        Ok(Self(Zeroizing::new(key)))
    }

    /// Parses a base64 master key as it arrives from deployment configuration.
    ///
    /// Padding is optional. The decoded length must be exactly
    /// [`MASTER_KEY_LEN`].
    ///
    /// # Errors
    ///
    /// Returns [`SecretCipherError::InvalidMasterKeyEncoding`] when the value is
    /// not base64 and [`SecretCipherError::InvalidMasterKeyLength`] when it does
    /// not decode to [`MASTER_KEY_LEN`] bytes. Neither error carries any part of
    /// the rejected value.
    pub fn from_base64(encoded: &str) -> Result<Self, SecretCipherError> {
        let decoded = STANDARD_PAD_INDIFFERENT
            .decode(encoded.trim().as_bytes())
            .map_err(|_| SecretCipherError::InvalidMasterKeyEncoding)?;
        Self::from_bytes(&decoded)
    }
}

impl fmt::Debug for MasterKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("MasterKey(<redacted>)")
    }
}

/// The owner of one persisted secret.
///
/// A purpose is a domain separator: the same bytes sealed under one purpose do
/// not open under another, because the purpose is part of the authenticated
/// header. A caller therefore cannot read a credential that belongs to a
/// different feature, even when both are stored in the same table.
///
/// [`SecretPurpose::new`] accepts the same bounded identifier character set as
/// a stored key name, so a purpose is safe in a log line and in a database key.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SecretPurpose(String);

impl SecretPurpose {
    /// Validates and wraps a purpose name.
    ///
    /// # Errors
    ///
    /// Returns [`SecretCipherError::InvalidPurpose`] when the name is empty,
    /// longer than [`frame::MAX_PURPOSE_LEN`] bytes, or outside the accepted
    /// character set.
    pub fn new(purpose: &str) -> Result<Self, SecretCipherError> {
        if !is_valid_identifier(purpose, frame::MAX_PURPOSE_LEN) {
            return Err(SecretCipherError::InvalidPurpose);
        }
        Ok(Self(purpose.to_string()))
    }

    /// The purpose name as stored in the frame header.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for SecretPurpose {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// Whether a value is a bounded identifier usable as a purpose or key name.
///
/// The same bound the frame header enforces, so a caller can reject a bad name
/// before it reaches a query, a log line, or the cipher.
pub fn is_valid_identifier(value: &str, max_len: usize) -> bool {
    !value.is_empty()
        && value.len() <= max_len
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
}

/// A decrypted secret value.
///
/// The plaintext is held in a zeroizing buffer, is not `Display` or `Debug`,
/// and is only reachable through the explicitly named [`SecretValue::expose`],
/// so a caller has to state at the call site that it is about to use the value.
pub struct SecretValue(Zeroizing<Vec<u8>>);

impl SecretValue {
    /// Wraps bytes that are already plaintext.
    ///
    /// Public so a store can hand back a value it already holds; a caller with
    /// ciphertext still goes through [`crate::SecretCipher::open`].
    #[must_use]
    pub fn from_plaintext(bytes: Vec<u8>) -> Self {
        Self(Zeroizing::new(bytes))
    }

    /// Borrows the plaintext.
    #[must_use]
    pub fn expose(&self) -> &[u8] {
        &self.0
    }

    /// Takes ownership of the plaintext for a caller that needs to keep it.
    #[must_use]
    pub fn into_inner(self) -> Vec<u8> {
        self.0.to_vec()
    }
}

impl fmt::Debug for SecretValue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SecretValue")
            .field("len", &self.0.len())
            .finish_non_exhaustive()
    }
}

/// Why a seal or open operation failed.
///
/// No variant carries plaintext, ciphertext, or key material, so every one of
/// them is safe to include in an error chain that reaches a log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SecretCipherError {
    /// The configured master key did not decode to [`MASTER_KEY_LEN`] bytes.
    InvalidMasterKeyLength(usize),
    /// The configured master key was not valid base64.
    InvalidMasterKeyEncoding,
    /// The purpose name is empty, oversized, or outside its character set.
    InvalidPurpose,
    /// The stored bytes are not a frame this build can read.
    Frame(FrameError),
    /// The stored frame was sealed with a different master-key version.
    KeyVersionMismatch {
        /// Version the configured master key is registered under.
        configured: u32,
        /// Version the stored ciphertext was sealed with.
        stored: u32,
    },
    /// The stored frame belongs to a different purpose.
    PurposeMismatch,
    /// The stored frame belongs to a different key name.
    KeyNameMismatch,
    /// The ciphertext did not authenticate under the header it was read with.
    AuthenticationFailed,
    /// The system random source could not produce a nonce.
    RandomnessUnavailable,
}

impl fmt::Display for SecretCipherError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidMasterKeyLength(len) => write!(
                formatter,
                "secret master key must decode to {MASTER_KEY_LEN} bytes, got {len}"
            ),
            Self::InvalidMasterKeyEncoding => {
                formatter.write_str("secret master key must be base64")
            }
            Self::InvalidPurpose => formatter.write_str("secret purpose is not a valid identifier"),
            Self::Frame(error) => write!(formatter, "{error}"),
            Self::KeyVersionMismatch { configured, stored } => write!(
                formatter,
                "secret was sealed with master key version {stored}, but version {configured} is configured"
            ),
            Self::PurposeMismatch => formatter.write_str("secret belongs to a different purpose"),
            Self::KeyNameMismatch => formatter.write_str("secret belongs to a different key name"),
            Self::AuthenticationFailed => {
                formatter.write_str("secret ciphertext failed authentication")
            }
            Self::RandomnessUnavailable => {
                formatter.write_str("system randomness is unavailable for a secret nonce")
            }
        }
    }
}

impl std::error::Error for SecretCipherError {}

impl From<FrameError> for SecretCipherError {
    fn from(error: FrameError) -> Self {
        Self::Frame(error)
    }
}

/// Authenticated encryption bound to one master key and one key version.
///
/// The key version travels in every frame, so a deployment that rotates its
/// master key can tell "this secret was sealed with a retired key" apart from
/// "this secret was tampered with" and re-encrypt deliberately instead of
/// guessing.
#[derive(Clone)]
pub struct SecretCipher {
    key: Zeroizing<[u8; MASTER_KEY_LEN]>,
    key_version: u32,
}

impl fmt::Debug for SecretCipher {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SecretCipher")
            .field("key_version", &self.key_version)
            .finish_non_exhaustive()
    }
}

impl SecretCipher {
    /// Binds a master key to the version its ciphertext is written with.
    pub fn new(master_key: MasterKey, key_version: u32) -> Self {
        Self {
            key: master_key.0,
            key_version,
        }
    }

    /// The master-key version this cipher writes and expects to read.
    #[must_use]
    pub fn key_version(&self) -> u32 {
        self.key_version
    }

    /// Seals a plaintext into a versioned frame for one purpose and key name.
    ///
    /// A fresh nonce is drawn from the system random source for every call, so
    /// sealing the same plaintext twice produces two different frames.
    ///
    /// # Errors
    ///
    /// Returns [`SecretCipherError::RandomnessUnavailable`] when no nonce can be
    /// generated and [`SecretCipherError::Frame`] when the key name is not a
    /// valid identifier.
    pub fn seal(
        &self,
        purpose: &SecretPurpose,
        key_name: &str,
        plaintext: &[u8],
    ) -> Result<Vec<u8>, SecretCipherError> {
        let nonce = chacha20poly1305::Nonce::try_generate()
            .map_err(|_| SecretCipherError::RandomnessUnavailable)?;
        let raw_nonce: [u8; frame::NONCE_LEN] = nonce.into();
        let aad = frame_header(self.key_version, purpose.as_str(), key_name, raw_nonce)?;
        let ciphertext = self
            .cipher()
            .encrypt(
                &nonce,
                Payload {
                    msg: plaintext,
                    aad: &aad,
                },
            )
            .map_err(|_| SecretCipherError::AuthenticationFailed)?;
        let sealed = SecretFrame::new(
            self.key_version,
            purpose.as_str(),
            key_name,
            raw_nonce,
            ciphertext,
        )?;
        Ok(sealed.encode())
    }

    /// Opens a stored frame for one purpose and key name.
    ///
    /// The stored purpose and key name are compared with the requested ones
    /// first, so a cross-purpose or cross-key read fails with a specific reason
    /// rather than a generic authentication failure, and are then re-checked by
    /// the AEAD because the header is the associated data.
    ///
    /// # Errors
    ///
    /// Returns [`SecretCipherError::PurposeMismatch`] or
    /// [`SecretCipherError::KeyNameMismatch`] when the frame belongs elsewhere,
    /// [`SecretCipherError::KeyVersionMismatch`] when it was sealed with another
    /// master-key version, [`SecretCipherError::Frame`] when the bytes are not a
    /// readable frame, and [`SecretCipherError::AuthenticationFailed`] when the
    /// ciphertext does not verify.
    pub fn open(
        &self,
        purpose: &SecretPurpose,
        key_name: &str,
        stored: &[u8],
    ) -> Result<SecretValue, SecretCipherError> {
        let frame = SecretFrame::decode(stored)?;
        if frame.key_version != self.key_version {
            return Err(SecretCipherError::KeyVersionMismatch {
                configured: self.key_version,
                stored: frame.key_version,
            });
        }
        if frame.purpose != purpose.as_str() {
            return Err(SecretCipherError::PurposeMismatch);
        }
        if frame.key_name != key_name {
            return Err(SecretCipherError::KeyNameMismatch);
        }
        let nonce: chacha20poly1305::Nonce = frame.nonce.into();
        let aad = frame.associated_data();
        let plaintext = self
            .cipher()
            .decrypt(
                &nonce,
                Payload {
                    msg: &frame.ciphertext,
                    aad: &aad,
                },
            )
            .map_err(|_| SecretCipherError::AuthenticationFailed)?;
        Ok(SecretValue::from_plaintext(plaintext))
    }

    fn cipher(&self) -> ChaCha20Poly1305 {
        let key = Key::<ChaCha20Poly1305>::from(*self.key);
        ChaCha20Poly1305::new(&key)
    }
}
