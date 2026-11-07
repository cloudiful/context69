//! The explicit tokenizer and input-token envelope of the embedding provider.
//!
//! The policy is a plain persisted value shared by the settings, chunking and
//! translation contracts. Counting never changes the text that is sent: the
//! two server prefixes exist only because the provider itself prepends them
//! before tokenizing an input, so the guard has to count them too.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Tokenizer that measures the embedding input envelope.
///
/// It is separate from the chunking tokenizer: an operator may chunk with one
/// vocabulary and count the provider input with another.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EmbeddingTokenizer {
    /// The `cl100k_base` vocabulary.
    #[default]
    Cl100kBase,
    /// The `o200k_base` vocabulary.
    O200kBase,
    /// One operator-provisioned local Hugging Face `tokenizer.json` artifact.
    HfLocal,
}

impl EmbeddingTokenizer {
    /// The persisted tokenizer name.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Cl100kBase => "cl100k_base",
            Self::O200kBase => "o200k_base",
            Self::HfLocal => "hf_local",
        }
    }

    /// The tokenizer a persisted name names.
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "cl100k_base" => Some(Self::Cl100kBase),
            "o200k_base" => Some(Self::O200kBase),
            "hf_local" => Some(Self::HfLocal),
            _ => None,
        }
    }
}

/// The exact input-token envelope and server-added prefixes of one provider.
///
/// `max_input_tokens = None` disables the guard while keeping the policy
/// serialized and versioned. The prefixes are counted only; they are never
/// sent to the provider, trimmed, normalized or separated from the input.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(default)]
pub struct EmbeddingInputPolicy {
    /// Tokenizer that measures the envelope.
    pub tokenizer: EmbeddingTokenizer,
    /// The pinned local artifact hash. Present exactly for `hf_local`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tokenizer_sha256: Option<String>,
    /// The full input-token cap. `None` disables the guard.
    pub max_input_tokens: Option<usize>,
    /// Text the provider prepends to every document input before tokenizing.
    pub document_server_prefix: String,
    /// Text the provider prepends to every query input before tokenizing.
    pub query_server_prefix: String,
    /// Whether the tokenizer adds its special tokens when it counts.
    pub add_special_tokens: bool,
}

impl Default for EmbeddingInputPolicy {
    fn default() -> Self {
        Self {
            tokenizer: EmbeddingTokenizer::Cl100kBase,
            tokenizer_sha256: None,
            max_input_tokens: None,
            document_server_prefix: String::new(),
            query_server_prefix: String::new(),
            add_special_tokens: false,
        }
    }
}

impl EmbeddingInputPolicy {
    /// Whether every field equals the complete default policy.
    pub fn is_default(&self) -> bool {
        self == &Self::default()
    }

    /// Validates the tokenizer/hash/special-token combination and the cap.
    pub fn validate(&self) -> Result<(), String> {
        match self.tokenizer {
            EmbeddingTokenizer::HfLocal => {
                let sha = self
                    .tokenizer_sha256
                    .as_deref()
                    .ok_or_else(|| "hf_local requires a 64-hex tokenizer_sha256".to_string())?;
                if !is_lower_sha256(sha) {
                    return Err(
                        "hf_local tokenizer_sha256 must be a lowercase 64-hex artifact hash"
                            .to_string(),
                    );
                }
            }
            _ => {
                if self.tokenizer_sha256.is_some() {
                    return Err(
                        "tokenizer_sha256 is only valid for the hf_local tokenizer".to_string()
                    );
                }
                if self.add_special_tokens {
                    return Err(
                        "add_special_tokens is only supported by an hf_local tokenizer".to_string(),
                    );
                }
            }
        }
        if self.max_input_tokens == Some(0) {
            return Err("max_input_tokens must be greater than zero when present".to_string());
        }
        Ok(())
    }
}

/// Whether a value is exactly 64 lowercase hexadecimal characters.
pub fn is_lower_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}
