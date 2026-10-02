//! Versioned ciphertext framing for persisted reversible secrets.
//!
//! A frame is the single stored representation of an encrypted secret. It
//! carries the metadata a reader needs to decide *whether* it may decrypt a
//! value at all — the container magic, the container version, the master-key
//! version, the owning purpose, and the secret's key name — followed by the
//! nonce and the authenticated ciphertext. Every one of those header bytes is
//! also the AEAD associated data, so a header edit is a decryption failure
//! rather than a silently reinterpreted secret.
//!
//! Framing is deliberately free of storage, transport, and configuration
//! concerns: it neither knows that secrets live in PostgreSQL nor how a master
//! key reaches the process. [`crate::SecretCipher`] is the layer that combines
//! this framing with an AEAD.
//!
//! ```text
//! magic            4 bytes  b"C69S"
//! format_version   1 byte   container version, currently 1
//! key_version      4 bytes  big-endian master-key version
//! purpose_len      2 bytes  big-endian
//! purpose          n bytes  UTF-8 purpose, 1..=MAX_PURPOSE_LEN
//! key_name_len     2 bytes  big-endian
//! key_name         m bytes  UTF-8 key name, 1..=MAX_KEY_NAME_LEN
//! nonce            12 bytes
//! ciphertext       rest     at least TAG_LEN bytes, tag appended
//! ```

use core::fmt;

/// Marks a byte string as a secret frame rather than a legacy plaintext value.
pub const MAGIC: [u8; 4] = *b"C69S";
/// Container version this build reads and writes.
pub const FORMAT_VERSION: u8 = 1;
/// Longest accepted purpose.
pub const MAX_PURPOSE_LEN: usize = 64;
/// Longest accepted secret key name.
pub const MAX_KEY_NAME_LEN: usize = 128;
/// Nonce length of the AEAD this frame is built for.
pub const NONCE_LEN: usize = 12;
/// Authentication tag length of the AEAD this frame is built for.
pub const TAG_LEN: usize = 16;

/// Bytes before the purpose length field.
const PREFIX_LEN: usize = MAGIC.len() + 1 + 4;
/// Length fields and nonce that surround the two names in the header.
const HEADER_FIELDS_LEN: usize = 2 + 2 + NONCE_LEN;

/// Why a stored byte string is not a frame this build can read.
///
/// Every variant describes the shape of the input, never its content: a frame
/// that failed to decode carries no secret, so a `Debug` rendering of this
/// error is safe to log.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameError {
    /// The bytes do not start with [`MAGIC`], so they are not a frame.
    NotAFrame,
    /// The container version is not one this build understands.
    UnsupportedFormatVersion(u8),
    /// The frame ended inside a fixed-size field.
    Truncated,
    /// A length field is zero or past its documented bound.
    InvalidNameLength,
    /// A name field is not valid UTF-8.
    InvalidNameEncoding,
    /// A name is outside the accepted character set.
    InvalidNameCharacters,
    /// The ciphertext is shorter than an authentication tag.
    CiphertextTooShort,
}

impl fmt::Display for FrameError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotAFrame => formatter.write_str("stored value is not a secret frame"),
            Self::UnsupportedFormatVersion(version) => {
                write!(formatter, "unsupported secret frame version {version}")
            }
            Self::Truncated => formatter.write_str("secret frame is truncated"),
            Self::InvalidNameLength => {
                formatter.write_str("secret frame name length is out of range")
            }
            Self::InvalidNameEncoding => {
                formatter.write_str("secret frame name is not valid utf-8")
            }
            Self::InvalidNameCharacters => {
                formatter.write_str("secret frame name has unsupported characters")
            }
            Self::CiphertextTooShort => {
                formatter.write_str("secret frame ciphertext is shorter than its tag")
            }
        }
    }
}

impl std::error::Error for FrameError {}

/// Whether a stored byte string claims to be a secret frame.
///
/// Callers use this to tell a versioned ciphertext from a legacy plaintext
/// value without decoding either.
#[must_use]
pub fn is_frame(bytes: &[u8]) -> bool {
    bytes.starts_with(&MAGIC)
}

/// Whether a name is inside its documented bound and character set.
///
/// Names reach the associated data, a log line, and a database key, so they are
/// restricted to ASCII alphanumerics plus `.`, `_`, and `-`. That excludes
/// control characters, whitespace, and non-ASCII without excluding any
/// identifier this store actually owns.
fn is_valid_name(name: &str, max_len: usize) -> bool {
    !name.is_empty()
        && name.len() <= max_len
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
}

/// Whether a purpose name is one a frame header can carry.
#[must_use]
pub fn is_valid_purpose(purpose: &str) -> bool {
    is_valid_name(purpose, MAX_PURPOSE_LEN)
}

/// Whether a secret key name is one a frame header can carry.
#[must_use]
pub fn is_valid_key_name(key_name: &str) -> bool {
    is_valid_name(key_name, MAX_KEY_NAME_LEN)
}

/// Writes the header that a frame's associated data is made of, after checking
/// that both names are identifiers this build accepts.
fn header_bytes(
    format_version: u8,
    key_version: u32,
    purpose: &str,
    key_name: &str,
    nonce: [u8; NONCE_LEN],
) -> Result<Vec<u8>, FrameError> {
    if !is_valid_name(purpose, MAX_PURPOSE_LEN) {
        return Err(classify_name(purpose, MAX_PURPOSE_LEN));
    }
    if !is_valid_name(key_name, MAX_KEY_NAME_LEN) {
        return Err(classify_name(key_name, MAX_KEY_NAME_LEN));
    }
    let mut bytes =
        Vec::with_capacity(PREFIX_LEN + HEADER_FIELDS_LEN + purpose.len() + key_name.len());
    bytes.extend_from_slice(&MAGIC);
    bytes.push(format_version);
    bytes.extend_from_slice(&key_version.to_be_bytes());
    bytes.extend_from_slice(&(purpose.len() as u16).to_be_bytes());
    bytes.extend_from_slice(purpose.as_bytes());
    bytes.extend_from_slice(&(key_name.len() as u16).to_be_bytes());
    bytes.extend_from_slice(key_name.as_bytes());
    bytes.extend_from_slice(&nonce);
    Ok(bytes)
}

/// The header a frame with these parameters would carry.
///
/// A sealer needs this before it has ciphertext, because the header is the
/// associated data the plaintext is encrypted under.
pub fn frame_header(
    key_version: u32,
    purpose: &str,
    key_name: &str,
    nonce: [u8; NONCE_LEN],
) -> Result<Vec<u8>, FrameError> {
    header_bytes(FORMAT_VERSION, key_version, purpose, key_name, nonce)
}

/// A frame's metadata without its ciphertext.
///
/// `Debug` is derived on the metadata only, so a frame can be described in an
/// audit note without ever touching the sealed value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameMetadata {
    /// Container version.
    pub format_version: u8,
    /// Master-key version the ciphertext was produced with.
    pub key_version: u32,
    /// Owning purpose.
    pub purpose: String,
    /// Secret key name.
    pub key_name: String,
}

/// A decoded secret frame.
#[derive(Clone, PartialEq, Eq)]
pub struct SecretFrame {
    /// Container version read from the stored bytes.
    pub format_version: u8,
    /// Master-key version the ciphertext was produced with.
    pub key_version: u32,
    /// Owning purpose, bound into the associated data.
    pub purpose: String,
    /// Secret key name, bound into the associated data.
    pub key_name: String,
    /// Per-message nonce.
    pub nonce: [u8; NONCE_LEN],
    /// Ciphertext with the authentication tag appended.
    pub ciphertext: Vec<u8>,
}

impl fmt::Debug for SecretFrame {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SecretFrame")
            .field("metadata", &self.metadata())
            .field("ciphertext_len", &self.ciphertext.len())
            .finish_non_exhaustive()
    }
}

impl SecretFrame {
    /// Builds a frame from already sealed ciphertext.
    ///
    /// # Errors
    ///
    /// Returns [`FrameError::InvalidNameLength`] or
    /// [`FrameError::InvalidNameCharacters`] when a name is outside its bound
    /// or character set, and [`FrameError::CiphertextTooShort`] when the
    /// ciphertext cannot carry an authentication tag.
    pub fn new(
        key_version: u32,
        purpose: &str,
        key_name: &str,
        nonce: [u8; NONCE_LEN],
        ciphertext: Vec<u8>,
    ) -> Result<Self, FrameError> {
        if !is_valid_name(purpose, MAX_PURPOSE_LEN) {
            return Err(classify_name(purpose, MAX_PURPOSE_LEN));
        }
        if !is_valid_name(key_name, MAX_KEY_NAME_LEN) {
            return Err(classify_name(key_name, MAX_KEY_NAME_LEN));
        }
        if ciphertext.len() < TAG_LEN {
            return Err(FrameError::CiphertextTooShort);
        }
        Ok(Self {
            format_version: FORMAT_VERSION,
            key_version,
            purpose: purpose.to_string(),
            key_name: key_name.to_string(),
            nonce,
            ciphertext,
        })
    }

    /// The frame metadata, without the ciphertext.
    #[must_use]
    pub fn metadata(&self) -> FrameMetadata {
        FrameMetadata {
            format_version: self.format_version,
            key_version: self.key_version,
            purpose: self.purpose.clone(),
            key_name: self.key_name.clone(),
        }
    }

    /// Writes the header and associated data this frame authenticates.
    fn write_header(&self, bytes: &mut Vec<u8>) {
        bytes.extend_from_slice(&MAGIC);
        bytes.push(self.format_version);
        bytes.extend_from_slice(&self.key_version.to_be_bytes());
        bytes.extend_from_slice(&(self.purpose.len() as u16).to_be_bytes());
        bytes.extend_from_slice(self.purpose.as_bytes());
        bytes.extend_from_slice(&(self.key_name.len() as u16).to_be_bytes());
        bytes.extend_from_slice(self.key_name.as_bytes());
        bytes.extend_from_slice(&self.nonce);
    }

    /// Serializes the frame to its stored byte form.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut bytes = self.associated_data();
        bytes.extend_from_slice(&self.ciphertext);
        bytes
    }

    /// The associated data authenticating the header.
    ///
    /// This is the frame's own header — magic, container version, master-key
    /// version, purpose, key name, and nonce. A reader authenticates the header
    /// it parsed, so a header rewritten after sealing cannot verify.
    #[must_use]
    pub fn associated_data(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(
            PREFIX_LEN + HEADER_FIELDS_LEN + self.purpose.len() + self.key_name.len(),
        );
        self.write_header(&mut bytes);
        bytes
    }

    /// Decodes a stored byte string into a frame.
    ///
    /// # Errors
    ///
    /// Returns the [`FrameError`] describing why the bytes are not a frame this
    /// build can read. Decoding never inspects or reports ciphertext content.
    pub fn decode(bytes: &[u8]) -> Result<Self, FrameError> {
        if !is_frame(bytes) {
            return Err(FrameError::NotAFrame);
        }
        let mut cursor = MAGIC.len();
        let format_version = read_u8(bytes, &mut cursor)?;
        if format_version != FORMAT_VERSION {
            return Err(FrameError::UnsupportedFormatVersion(format_version));
        }
        let key_version = u32::from_be_bytes(read_array::<4>(bytes, &mut cursor)?);
        let purpose = read_name(bytes, &mut cursor, MAX_PURPOSE_LEN)?;
        let key_name = read_name(bytes, &mut cursor, MAX_KEY_NAME_LEN)?;
        let nonce = read_array::<NONCE_LEN>(bytes, &mut cursor)?;
        let ciphertext = bytes.get(cursor..).ok_or(FrameError::Truncated)?.to_vec();
        if ciphertext.len() < TAG_LEN {
            return Err(FrameError::CiphertextTooShort);
        }
        Ok(Self {
            format_version,
            key_version,
            purpose,
            key_name,
            nonce,
            ciphertext,
        })
    }
}

fn classify_name(name: &str, max_len: usize) -> FrameError {
    if name.is_empty() || name.len() > max_len {
        FrameError::InvalidNameLength
    } else {
        FrameError::InvalidNameCharacters
    }
}

fn read_u8(bytes: &[u8], cursor: &mut usize) -> Result<u8, FrameError> {
    let value = *bytes.get(*cursor).ok_or(FrameError::Truncated)?;
    *cursor += 1;
    Ok(value)
}

fn read_array<const N: usize>(bytes: &[u8], cursor: &mut usize) -> Result<[u8; N], FrameError> {
    let value: [u8; N] = read_slice(bytes, cursor, N)?
        .try_into()
        .map_err(|_| FrameError::Truncated)?;
    Ok(value)
}

fn read_slice<'a>(bytes: &'a [u8], cursor: &mut usize, len: usize) -> Result<&'a [u8], FrameError> {
    let end = cursor.checked_add(len).ok_or(FrameError::Truncated)?;
    let slice = bytes.get(*cursor..end).ok_or(FrameError::Truncated)?;
    *cursor = end;
    Ok(slice)
}

fn read_name(bytes: &[u8], cursor: &mut usize, max_len: usize) -> Result<String, FrameError> {
    let len = u16::from_be_bytes(read_array::<2>(bytes, cursor)?) as usize;
    if len == 0 {
        return Err(FrameError::InvalidNameLength);
    }
    let raw = read_slice(bytes, cursor, len)?;
    let name = core::str::from_utf8(raw).map_err(|_| FrameError::InvalidNameEncoding)?;
    if !is_valid_name(name, max_len) {
        return Err(classify_name(name, max_len));
    }
    Ok(name.to_string())
}
