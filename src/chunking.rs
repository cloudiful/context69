use std::mem;
use std::sync::{Arc, RwLock};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use self::chunking_token::{ResolvedTokenizer, TokenizerKind, TokenizerRef};
use self::chunking_token_artifact::TokenizerCatalog;
use crate::domain::{DocumentChunk, NormalizedDocument};

const DEFAULT_MAX_CHARS: usize = 1200;
const DEFAULT_OVERLAP_CHARS: usize = 200;
const PARAGRAPH_SEPARATOR: &str = "\n\n";

/// Payload key that pins the chunking budget of one task item.
///
/// A submission stamps the snapshot once, before any item runs. Every retry,
/// resume, or later stage of that item reads the pinned value instead of the
/// live runtime budget, so a settings save only governs tasks submitted after
/// it committed.
pub const CHUNK_BUDGET_PAYLOAD_KEY: &str = "chunk_budget";

#[path = "chunking_token.rs"]
pub mod chunking_token;

#[path = "chunking_token_artifact.rs"]
pub mod chunking_token_artifact;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChunkingConfig {
    pub max_chars: usize,
    pub overlap_chars: usize,
}

impl Default for ChunkingConfig {
    fn default() -> Self {
        Self {
            max_chars: DEFAULT_MAX_CHARS,
            overlap_chars: DEFAULT_OVERLAP_CHARS,
        }
    }
}

/// Unit that drives the chunking budget.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChunkBudgetMode {
    /// Legacy character budget.
    #[default]
    Chars,
    /// Token budget measured with an explicit tokenizer.
    Tokens,
}

/// Token budget used when [`ChunkBudgetMode::Tokens`] is selected.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenBudget {
    pub tokenizer: TokenizerRef,
    pub max_tokens: usize,
    pub overlap_tokens: usize,
}

/// Chunking budget that carries the legacy character configuration plus an
/// explicit mode and an optional token configuration.
///
/// The character values keep their original meaning; they are never relabeled
/// as token counts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChunkBudget {
    pub mode: ChunkBudgetMode,
    pub chars: ChunkingConfig,
    pub tokens: Option<TokenBudget>,
    /// The explicit embedding input envelope pinned with this budget.
    ///
    /// Absent keeps the legacy snapshot byte-identical. It is present only
    /// when the policy is nondefault.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_policy: Option<context69_contracts::settings::EmbeddingInputPolicy>,
}

/// Failure raised by the token-aware chunking path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChunkingError {
    InvalidTokenBudget {
        reason: String,
    },
    /// A character budget no chunker could honor, refused before a fork is
    /// created or a snapshot is accepted.
    InvalidCharacterBudget {
        reason: String,
    },
    CharacterExceedsBudget {
        character: char,
        tokens: usize,
        max_tokens: usize,
    },
    TokenBudgetExceeded {
        max_tokens: usize,
    },
    /// The pinned tokenizer is an artifact this process did not verify.
    /// The chunk path fails closed instead of substituting another engine.
    TokenizerUnavailable {
        sha256: String,
        reason: String,
    },
    /// One input cannot be emitted without exceeding the explicit provider
    /// input-token envelope, even after the overlap gave up its room.
    InputEnvelopeExceeded {
        role: &'static str,
        count: usize,
        max_input_tokens: usize,
        prefix_chars: usize,
    },
    /// The pinned embedding input policy is invalid or unresolvable.
    InvalidInputPolicy {
        reason: String,
    },
    /// A pinned budget snapshot is present but unreadable. It is never
    /// silently replaced by the live budget: a task that cannot read its own
    /// snapshot must fail instead of chunking under a budget it never chose.
    InvalidSnapshot {
        reason: String,
    },
}

impl std::fmt::Display for ChunkingError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidTokenBudget { reason } => {
                write!(formatter, "invalid token chunking budget: {reason}")
            }
            Self::InvalidCharacterBudget { reason } => {
                write!(formatter, "invalid character chunking budget: {reason}")
            }
            Self::CharacterExceedsBudget {
                character,
                tokens,
                max_tokens,
            } => write!(
                formatter,
                "character {character:?} needs {tokens} tokens but the budget allows {max_tokens}"
            ),
            Self::TokenBudgetExceeded { max_tokens } => write!(
                formatter,
                "the chunking content does not fit into a budget of {max_tokens} tokens"
            ),
            Self::TokenizerUnavailable { sha256, reason } => write!(
                formatter,
                "the pinned tokenizer artifact {sha256} is unavailable: {reason}"
            ),
            Self::InputEnvelopeExceeded {
                role,
                count,
                max_input_tokens,
                prefix_chars,
            } => write!(
                formatter,
                "the {role} input needs {count} tokens but the configured input envelope allows \
                 {max_input_tokens} tokens ({prefix_chars} characters of server-added prefix are \
                 counted); no text was truncated or rechunked"
            ),
            Self::InvalidInputPolicy { reason } => {
                write!(formatter, "invalid embedding input policy: {reason}")
            }
            Self::InvalidSnapshot { reason } => {
                write!(formatter, "invalid chunk budget snapshot: {reason}")
            }
        }
    }
}

impl std::error::Error for ChunkingError {}

/// The process-wide chunking budget.
///
/// Every consumer reads the current budget through one shared handle, so a
/// successful settings save takes effect for the next task without a restart,
/// and a task that already pinned its snapshot keeps that snapshot.
///
/// The lock is a short, synchronous read or replace of a small value; it is
/// never held across an await, a tokenization pass, or a provider call.
#[derive(Clone)]
pub struct ChunkingRuntime {
    budget: Arc<RwLock<ChunkBudget>>,
    catalog: Arc<TokenizerCatalog>,
}

impl ChunkingRuntime {
    pub fn new(budget: ChunkBudget) -> Self {
        Self::with_catalog(budget, TokenizerCatalog::installed_or_empty())
    }

    pub fn with_catalog(budget: ChunkBudget, catalog: Arc<TokenizerCatalog>) -> Self {
        Self {
            budget: Arc::new(RwLock::new(budget)),
            catalog,
        }
    }

    /// A handle with the same immutable catalog and a different live budget.
    pub fn with_budget(&self, budget: ChunkBudget) -> Self {
        Self {
            budget: Arc::new(RwLock::new(budget)),
            catalog: Arc::clone(&self.catalog),
        }
    }

    pub fn catalog(&self) -> &Arc<TokenizerCatalog> {
        &self.catalog
    }

    /// The budget as it is right now. The returned value is an owned snapshot.
    ///
    /// A poisoned lock is recovered: the guarded value is a plain configuration
    /// value whose invariants are re-validated on use, so a panic elsewhere
    /// must not take chunking down with it.
    pub fn snapshot(&self) -> ChunkBudget {
        match self.budget.read() {
            Ok(budget) => budget.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }

    /// Replaces the active budget. The next snapshot observes the new value.
    pub fn replace(&self, budget: ChunkBudget) {
        match self.budget.write() {
            Ok(mut active) => *active = budget,
            Err(poisoned) => *poisoned.into_inner() = budget,
        }
    }

    /// Resolves one budget against the immutable catalog.
    ///
    /// A static tokenizer needs no artifact. A local Hugging Face tokenizer is
    /// looked up by its pinned hash and fails closed when the process catalog
    /// does not carry exactly that artifact.
    pub fn resolve(&self, budget: &ChunkBudget) -> Result<ResolvedChunkBudget, ChunkingError> {
        budget.validate()?;
        let tokenizer =
            match budget.mode {
                ChunkBudgetMode::Chars => None,
                ChunkBudgetMode::Tokens => {
                    let tokens = budget.tokens.as_ref().ok_or_else(|| {
                        ChunkingError::InvalidTokenBudget {
                            reason: "token mode requires a token budget".to_string(),
                        }
                    })?;
                    Some(match &tokens.tokenizer {
                        TokenizerRef::Cl100kBase => {
                            ResolvedTokenizer::from_static(TokenizerKind::Cl100kBase)
                        }
                        TokenizerRef::O200kBase => {
                            ResolvedTokenizer::from_static(TokenizerKind::O200kBase)
                        }
                        TokenizerRef::HfLocal { sha256 } => {
                            let tokenizer = self.catalog.require(sha256).map_err(|error| {
                                ChunkingError::TokenizerUnavailable {
                                    sha256: sha256.clone(),
                                    reason: error.to_string(),
                                }
                            })?;
                            ResolvedTokenizer::from_hf(sha256.clone(), tokenizer)
                        }
                    })
                }
            };
        let input_policy = match &budget.input_policy {
            Some(policy) if !policy.is_default() => {
                Some(self.catalog.resolve_input_policy(policy)?)
            }
            _ => None,
        };
        Ok(ResolvedChunkBudget {
            budget: budget.clone(),
            tokenizer,
            input_policy,
        })
    }

    /// Resolves the current live budget.
    pub fn resolve_snapshot(&self) -> Result<ResolvedChunkBudget, ChunkingError> {
        self.resolve(&self.snapshot())
    }

    /// The character budget of this deployment.
    ///
    /// A task item that predates the pinned snapshot has no recoverable
    /// original configuration, so it is pinned to the legacy character budget
    /// exactly as the deployment still stores it, never to a token budget.
    pub fn legacy_chars_budget(&self) -> ChunkBudget {
        ChunkBudget::chars(self.snapshot().chars)
    }
}

impl Default for ChunkingRuntime {
    fn default() -> Self {
        Self::new(ChunkBudget::default())
    }
}

impl From<ChunkingConfig> for ChunkingRuntime {
    fn from(config: ChunkingConfig) -> Self {
        Self::new(ChunkBudget::chars(config))
    }
}

impl From<&ChunkingConfig> for ChunkingRuntime {
    fn from(config: &ChunkingConfig) -> Self {
        Self::new(ChunkBudget::chars(config.clone()))
    }
}

impl From<ChunkBudget> for ChunkingRuntime {
    fn from(budget: ChunkBudget) -> Self {
        Self::new(budget)
    }
}

impl From<&ChunkBudget> for ChunkingRuntime {
    fn from(budget: &ChunkBudget) -> Self {
        Self::new(budget.clone())
    }
}

/// The lowercase SHA256 identity of one nondefault embedding input policy.
///
/// Every field is written as a version tag and an unambiguous length-prefixed
/// value, so two different policies can never serialize to the same bytes.
pub fn embedding_input_policy_identity(
    policy: &context69_contracts::settings::EmbeddingInputPolicy,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"context69.embedding_input_policy.v1");
    let mut field = |bytes: &[u8]| {
        hasher.update((bytes.len() as u64).to_be_bytes());
        hasher.update(bytes);
    };
    field(policy.tokenizer.name().as_bytes());
    match policy.tokenizer_sha256.as_deref() {
        Some(sha) => {
            field(b"sha256");
            field(sha.as_bytes());
        }
        None => field(b"no-sha256"),
    }
    match policy.max_input_tokens {
        Some(max) => {
            field(b"max-input-tokens");
            field(&max.to_be_bytes());
        }
        None => field(b"no-max-input-tokens"),
    }
    field(policy.document_server_prefix.as_bytes());
    field(policy.query_server_prefix.as_bytes());
    field(if policy.add_special_tokens {
        b"add-special-tokens"
    } else {
        b"no-add-special-tokens"
    });
    let digest = hasher.finalize();
    let mut hex = String::with_capacity(64);
    for byte in digest {
        hex.push_str(&format!("{byte:02x}"));
    }
    hex
}

/// The serialized form of a pinned budget.
pub fn chunk_budget_to_value(budget: &ChunkBudget) -> Value {
    serde_json::to_value(budget).expect("a chunk budget is serializable")
}

/// Adds or replaces the pinned budget of one payload, preserving every other
/// key exactly as it was.
pub fn payload_with_chunk_budget(payload: &Value, budget: &ChunkBudget) -> Value {
    let mut next = payload.clone();
    match &mut next {
        Value::Object(map) => {
            map.insert(
                CHUNK_BUDGET_PAYLOAD_KEY.to_string(),
                chunk_budget_to_value(budget),
            );
        }
        _ => {
            let mut map = serde_json::Map::new();
            map.insert(
                CHUNK_BUDGET_PAYLOAD_KEY.to_string(),
                chunk_budget_to_value(budget),
            );
            next = Value::Object(map);
        }
    }
    next
}

/// Reads the pinned budget of a payload.
///
/// `Ok(None)` means the payload carries no snapshot (a legacy item).
/// A snapshot that is present but unreadable or invalid fails closed with
/// [`ChunkingError::InvalidSnapshot`]; it is never treated as absent.
pub fn parse_chunk_budget(payload: &Value) -> Result<Option<ChunkBudget>, ChunkingError> {
    let Some(value) = payload.get(CHUNK_BUDGET_PAYLOAD_KEY) else {
        return Ok(None);
    };
    if value.is_null() {
        return Err(ChunkingError::InvalidSnapshot {
            reason: "the pinned budget is null".to_string(),
        });
    }
    let budget: ChunkBudget =
        serde_json::from_value(value.clone()).map_err(|error| ChunkingError::InvalidSnapshot {
            reason: error.to_string(),
        })?;
    if budget.mode == ChunkBudgetMode::Chars {
        if budget.chars.max_chars == 0 {
            return Err(ChunkingError::InvalidSnapshot {
                reason: "max_chars must be greater than zero".to_string(),
            });
        }
        if budget.chars.overlap_chars >= budget.chars.max_chars {
            return Err(ChunkingError::InvalidSnapshot {
                reason: "overlap_chars must be smaller than max_chars".to_string(),
            });
        }
    }
    budget.validate()?;
    Ok(Some(budget))
}

/// The budget a task item must chunk under.
///
/// A pinned snapshot always wins. A legacy payload without one resolves to
/// `legacy_fallback`; a malformed snapshot is refused instead of falling back.
pub fn resolve_chunk_budget(
    payload: &Value,
    legacy_fallback: &ChunkBudget,
) -> Result<ChunkBudget, ChunkingError> {
    Ok(parse_chunk_budget(payload)?.unwrap_or_else(|| legacy_fallback.clone()))
}

impl ChunkBudget {
    /// Budget that keeps the legacy character behavior.
    pub fn chars(config: ChunkingConfig) -> Self {
        Self {
            mode: ChunkBudgetMode::Chars,
            chars: config,
            tokens: None,
            input_policy: None,
        }
    }

    /// Budget that measures chunks with an explicit legacy tokenizer.
    pub fn tokens(tokenizer: TokenizerKind, max_tokens: usize, overlap_tokens: usize) -> Self {
        Self::tokens_ref(tokenizer.into(), max_tokens, overlap_tokens)
    }

    /// Budget that measures chunks with an explicit tokenizer reference.
    pub fn tokens_ref(tokenizer: TokenizerRef, max_tokens: usize, overlap_tokens: usize) -> Self {
        Self {
            mode: ChunkBudgetMode::Tokens,
            chars: ChunkingConfig::default(),
            tokens: Some(TokenBudget {
                tokenizer,
                max_tokens,
                overlap_tokens,
            }),
            input_policy: None,
        }
    }

    pub fn validate(&self) -> Result<(), ChunkingError> {
        if self.mode != ChunkBudgetMode::Tokens {
            if self.chars.max_chars == 0 {
                return Err(ChunkingError::InvalidCharacterBudget {
                    reason: "max_chars must be greater than zero".to_string(),
                });
            }
            if self.chars.overlap_chars >= self.chars.max_chars {
                return Err(ChunkingError::InvalidCharacterBudget {
                    reason: "overlap_chars must be smaller than max_chars".to_string(),
                });
            }
            return Ok(());
        }
        let Some(tokens) = self.tokens.as_ref() else {
            return Err(ChunkingError::InvalidTokenBudget {
                reason: "token mode requires a token budget".to_string(),
            });
        };
        if tokens.max_tokens == 0 {
            return Err(ChunkingError::InvalidTokenBudget {
                reason: "max_tokens must be greater than zero".to_string(),
            });
        }
        if tokens.overlap_tokens >= tokens.max_tokens {
            return Err(ChunkingError::InvalidTokenBudget {
                reason: "overlap_tokens must be smaller than max_tokens".to_string(),
            });
        }
        tokens.tokenizer.validate()?;
        if let Some(policy) = &self.input_policy {
            policy
                .validate()
                .map_err(|reason| ChunkingError::InvalidInputPolicy { reason })?;
        }
        Ok(())
    }

    /// Deterministic identity of the budget, used to detect a budget change.
    pub fn identity(&self) -> String {
        let base = match self.mode {
            ChunkBudgetMode::Chars => format!(
                "chars:{}:{}",
                self.chars.max_chars, self.chars.overlap_chars
            ),
            ChunkBudgetMode::Tokens => match self.tokens.as_ref() {
                Some(tokens) => format!(
                    "tokens:{}:{}:{}",
                    tokens.tokenizer.identity_name(),
                    tokens.max_tokens,
                    tokens.overlap_tokens
                ),
                None => "tokens:unconfigured".to_string(),
            },
        };
        match &self.input_policy {
            Some(policy) if !policy.is_default() => {
                format!("{base}:input:{}", embedding_input_policy_identity(policy))
            }
            _ => base,
        }
    }
}

impl From<ChunkingConfig> for ChunkBudget {
    fn from(config: ChunkingConfig) -> Self {
        Self::chars(config)
    }
}

impl From<&ChunkingConfig> for ChunkBudget {
    fn from(config: &ChunkingConfig) -> Self {
        Self::chars(config.clone())
    }
}

impl Default for ChunkBudget {
    fn default() -> Self {
        Self::chars(ChunkingConfig::default())
    }
}

/// A budget that the chunking entry points accept as a borrowed input.
///
/// The legacy character configuration and the explicit [`ChunkBudget`] are both
/// valid sources, so a caller can pass either one.
pub trait ChunkBudgetSource {
    /// Result of collecting the chunks of one document.
    type Output;
    /// Item produced by the chunk iterator.
    type Item;
    /// Chunk iterator for one document.
    type Iter<'a>: Iterator<Item = Self::Item>;

    fn chunk_document(
        &self,
        document_id: i64,
        source_key: &str,
        document: &NormalizedDocument,
    ) -> Self::Output;

    fn chunk_document_iter<'a>(
        &self,
        document_id: i64,
        source_key: &'a str,
        document: &'a NormalizedDocument,
        generation: Option<i64>,
    ) -> Self::Iter<'a>;
}

impl ChunkBudgetSource for ChunkingConfig {
    type Output = Vec<DocumentChunk>;
    type Item = DocumentChunk;
    type Iter<'a> = ChunkDocumentIter<'a>;

    fn chunk_document(
        &self,
        document_id: i64,
        source_key: &str,
        document: &NormalizedDocument,
    ) -> Self::Output {
        chunk_document_iter(document_id, source_key, document, self, None).collect()
    }

    fn chunk_document_iter<'a>(
        &self,
        document_id: i64,
        source_key: &'a str,
        document: &'a NormalizedDocument,
        generation: Option<i64>,
    ) -> Self::Iter<'a> {
        ChunkDocumentIter::new(document_id, source_key, document, self, generation)
    }
}

impl ChunkBudgetSource for ChunkBudget {
    type Output = Result<Vec<DocumentChunk>, ChunkingError>;
    type Item = Result<DocumentChunk, ChunkingError>;
    type Iter<'a> = ChunkBudgetDocumentIter<'a>;

    fn chunk_document(
        &self,
        document_id: i64,
        source_key: &str,
        document: &NormalizedDocument,
    ) -> Self::Output {
        match self.mode {
            ChunkBudgetMode::Chars => {
                let resolved = static_resolved_budget(self)?;
                resolved
                    .chunk_document_iter(document_id, source_key, document, None)?
                    .collect()
            }
            ChunkBudgetMode::Tokens => {
                let resolved = static_resolved_budget(self)?;
                let iter = token_chunk_iter(document_id, source_key, document, &resolved, None)?;
                iter.collect()
            }
        }
    }

    fn chunk_document_iter<'a>(
        &self,
        document_id: i64,
        source_key: &'a str,
        document: &'a NormalizedDocument,
        generation: Option<i64>,
    ) -> Self::Iter<'a> {
        match self.mode {
            ChunkBudgetMode::Chars => match static_resolved_budget(self) {
                Ok(resolved) => match resolved.chunk_document_iter(
                    document_id,
                    source_key,
                    document,
                    generation,
                ) {
                    Ok(iter) => iter,
                    Err(error) => ChunkBudgetDocumentIter::Tokens(
                        chunking_token::TokenChunkDocumentIter::with_failure(
                            document_id,
                            source_key,
                            document,
                            error,
                        ),
                    ),
                },
                Err(error) => ChunkBudgetDocumentIter::Tokens(
                    chunking_token::TokenChunkDocumentIter::with_failure(
                        document_id,
                        source_key,
                        document,
                        error,
                    ),
                ),
            },
            ChunkBudgetMode::Tokens => {
                match static_resolved_budget(self).and_then(|resolved| {
                    token_chunk_iter(document_id, source_key, document, &resolved, generation)
                }) {
                    Ok(iter) => ChunkBudgetDocumentIter::Tokens(iter),
                    Err(error) => ChunkBudgetDocumentIter::Tokens(
                        chunking_token::TokenChunkDocumentIter::with_failure(
                            document_id,
                            source_key,
                            document,
                            error,
                        ),
                    ),
                }
            }
        }
    }
}

/// Chunk iterator that serves both budget modes.
pub enum ChunkBudgetDocumentIter<'a> {
    Chars(ChunkDocumentIter<'a>),
    CharsGuarded(GuardedCharChunkIter<'a>),
    Tokens(chunking_token::TokenChunkDocumentIter<'a>),
}

impl Iterator for ChunkBudgetDocumentIter<'_> {
    type Item = Result<DocumentChunk, ChunkingError>;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::Chars(iter) => iter.next().map(Ok),
            Self::CharsGuarded(iter) => iter.next(),
            Self::Tokens(iter) => iter.next(),
        }
    }
}

/// A budget that was resolved against one immutable catalog.
///
/// The stored payload keeps only the tokenizer reference; this value owns the
/// already-loaded engine for the duration of one cutting pass. Every real
/// cutting path resolves through [`ChunkingRuntime`] before it reaches here.
#[derive(Clone)]
pub struct ResolvedChunkBudget {
    budget: ChunkBudget,
    tokenizer: Option<ResolvedTokenizer>,
    input_policy: Option<crate::chunking::chunking_token_artifact::ResolvedEmbeddingInputPolicy>,
}

impl ResolvedChunkBudget {
    pub fn budget(&self) -> &ChunkBudget {
        &self.budget
    }

    pub fn identity(&self) -> String {
        self.budget.identity()
    }

    /// The resolved embedding input envelope pinned with this budget.
    pub fn input_policy(
        &self,
    ) -> Option<&crate::chunking::chunking_token_artifact::ResolvedEmbeddingInputPolicy> {
        self.input_policy.as_ref()
    }

    pub fn chunk_document(
        &self,
        document_id: i64,
        source_key: &str,
        document: &NormalizedDocument,
    ) -> Result<Vec<DocumentChunk>, ChunkingError> {
        self.chunk_document_iter(document_id, source_key, document, None)?
            .collect()
    }

    pub fn chunk_document_iter<'a>(
        &self,
        document_id: i64,
        source_key: &'a str,
        document: &'a NormalizedDocument,
        generation: Option<i64>,
    ) -> Result<ChunkBudgetDocumentIter<'a>, ChunkingError> {
        self.budget.validate()?;
        Ok(match self.budget.mode {
            ChunkBudgetMode::Chars => match &self.input_policy {
                Some(policy) => ChunkBudgetDocumentIter::CharsGuarded(GuardedCharChunkIter::new(
                    document_id,
                    source_key,
                    document,
                    &self.budget.chars,
                    generation,
                    policy.clone(),
                )),
                None => ChunkBudgetDocumentIter::Chars(chunk_document_iter(
                    document_id,
                    source_key,
                    document,
                    &self.budget.chars,
                    generation,
                )),
            },
            ChunkBudgetMode::Tokens => ChunkBudgetDocumentIter::Tokens(token_chunk_iter(
                document_id,
                source_key,
                document,
                self,
                generation,
            )?),
        })
    }
}

/// Chunk a document with a resolved budget.
pub fn chunk_document_resolved(
    document_id: i64,
    source_key: &str,
    document: &NormalizedDocument,
    budget: &ResolvedChunkBudget,
) -> Result<Vec<DocumentChunk>, ChunkingError> {
    budget.chunk_document(document_id, source_key, document)
}

/// Stream the chunks of a document with a resolved budget.
pub fn chunk_document_iter_resolved<'a>(
    document_id: i64,
    source_key: &'a str,
    document: &'a NormalizedDocument,
    budget: &ResolvedChunkBudget,
    generation: Option<i64>,
) -> Result<ChunkBudgetDocumentIter<'a>, ChunkingError> {
    budget.chunk_document_iter(document_id, source_key, document, generation)
}

/// Resolves a budget that names only a legacy static tokenizer.
fn static_resolved_budget(budget: &ChunkBudget) -> Result<ResolvedChunkBudget, ChunkingError> {
    budget.validate()?;
    let tokenizer = match budget.mode {
        ChunkBudgetMode::Chars => None,
        ChunkBudgetMode::Tokens => {
            let tokens =
                budget
                    .tokens
                    .as_ref()
                    .ok_or_else(|| ChunkingError::InvalidTokenBudget {
                        reason: "token mode requires a token budget".to_string(),
                    })?;
            match &tokens.tokenizer {
                TokenizerRef::Cl100kBase => {
                    Some(ResolvedTokenizer::from_static(TokenizerKind::Cl100kBase))
                }
                TokenizerRef::O200kBase => {
                    Some(ResolvedTokenizer::from_static(TokenizerKind::O200kBase))
                }
                TokenizerRef::HfLocal { sha256 } => {
                    return Err(ChunkingError::TokenizerUnavailable {
                        sha256: sha256.clone(),
                        reason:
                            "a local Hugging Face tokenizer must be resolved through the verified tokenizer catalog"
                                .to_string(),
                    });
                }
            }
        }
    };
    let input_policy = match &budget.input_policy {
        Some(policy) if !policy.is_default() => {
            Some(TokenizerCatalog::installed_or_empty().resolve_input_policy(policy)?)
        }
        _ => None,
    };
    Ok(ResolvedChunkBudget {
        budget: budget.clone(),
        tokenizer,
        input_policy,
    })
}

/// Chunk a document with the given budget.
///
/// The legacy character configuration and the explicit [`ChunkBudget`] are both
/// accepted; the character configuration keeps its previous infallible result
/// and the explicit budget reports token failures.
pub fn chunk_document<B>(
    document_id: i64,
    source_key: &str,
    document: &NormalizedDocument,
    budget: &B,
) -> B::Output
where
    B: ChunkBudgetSource + ?Sized,
{
    budget.chunk_document(document_id, source_key, document)
}

/// Stream the chunks of a document with the given budget.
pub fn chunk_document_iter<'a, B>(
    document_id: i64,
    source_key: &'a str,
    document: &'a NormalizedDocument,
    budget: &B,
    generation: Option<i64>,
) -> B::Iter<'a>
where
    B: ChunkBudgetSource + ?Sized,
{
    budget.chunk_document_iter(document_id, source_key, document, generation)
}

/// Chunk a document with an explicit budget, reporting token failures.
pub fn chunk_document_with_budget(
    document_id: i64,
    source_key: &str,
    document: &NormalizedDocument,
    budget: &ChunkBudget,
) -> Result<Vec<DocumentChunk>, ChunkingError> {
    budget.chunk_document(document_id, source_key, document)
}

/// Stream the chunks of a document with an explicit budget, reporting token
/// failures before the first chunk is produced.
pub fn chunk_document_iter_with_budget<'a>(
    document_id: i64,
    source_key: &'a str,
    document: &'a NormalizedDocument,
    budget: &ChunkBudget,
    generation: Option<i64>,
) -> Result<ChunkBudgetDocumentIter<'a>, ChunkingError> {
    budget.validate()?;
    Ok(match budget.mode {
        ChunkBudgetMode::Chars => match static_resolved_budget(budget)? {
            resolved => {
                resolved.chunk_document_iter(document_id, source_key, document, generation)?
            }
        },
        ChunkBudgetMode::Tokens => ChunkBudgetDocumentIter::Tokens(token_chunk_iter(
            document_id,
            source_key,
            document,
            &static_resolved_budget(budget)?,
            generation,
        )?),
    })
}

fn token_chunk_iter<'a>(
    document_id: i64,
    source_key: &'a str,
    document: &'a NormalizedDocument,
    budget: &ResolvedChunkBudget,
    generation: Option<i64>,
) -> Result<chunking_token::TokenChunkDocumentIter<'a>, ChunkingError> {
    budget.budget.validate()?;
    let tokens =
        budget
            .budget
            .tokens
            .as_ref()
            .ok_or_else(|| ChunkingError::InvalidTokenBudget {
                reason: "token mode requires a token budget".to_string(),
            })?;
    let tokenizer = budget
        .tokenizer
        .clone()
        .ok_or_else(|| ChunkingError::InvalidTokenBudget {
            reason: "token mode requires a resolved tokenizer".to_string(),
        })?;
    chunking_token::TokenChunkDocumentIter::new_resolved(
        document_id,
        source_key,
        document,
        tokenizer,
        tokens.max_tokens,
        tokens.overlap_tokens,
        generation,
    )
    .map(|iter| iter.with_input_policy(budget.input_policy.clone()))
}

pub struct ChunkDocumentIter<'a> {
    document_id: i64,
    source_key: &'a str,
    document: &'a NormalizedDocument,
    generation: Option<i64>,
    paragraphs: std::str::Split<'a, &'static str>,
    current: String,
    current_chars: usize,
    pending: Option<PendingParagraph<'a>>,
    max_chars: usize,
    overlap_chars: usize,
    next_index: i32,
    finished: bool,
    /// Source bytes appended since the last emitted chunk, never the repeated
    /// overlap.
    raw: String,
    /// The raw source bytes of the most recently emitted chunk.
    last_raw: String,
}

struct PendingParagraph<'a> {
    text: &'a str,
    offset: usize,
    separator_offset: usize,
}

impl<'a> ChunkDocumentIter<'a> {
    fn new(
        document_id: i64,
        source_key: &'a str,
        document: &'a NormalizedDocument,
        config: &ChunkingConfig,
        generation: Option<i64>,
    ) -> Self {
        let max_chars = config.max_chars.max(1);
        let overlap_chars = config.overlap_chars.min(max_chars - 1);
        let paragraphs = document.body_text.split(PARAGRAPH_SEPARATOR);

        Self {
            document_id,
            source_key,
            document,
            generation,
            paragraphs,
            current: String::new(),
            current_chars: 0,
            pending: None,
            max_chars,
            overlap_chars,
            next_index: 0,
            finished: false,
            raw: String::new(),
            last_raw: String::new(),
        }
    }

    fn next_paragraph(&mut self) -> Option<&'a str> {
        self.paragraphs.find_map(|value| {
            let value = value.trim();
            (!value.is_empty()).then_some(value)
        })
    }

    fn take_chunk(&mut self, keep_overlap: bool) -> String {
        let chunk = mem::take(&mut self.current);
        self.last_raw = mem::take(&mut self.raw);
        if keep_overlap {
            self.current = build_overlap_from_text(&chunk, self.overlap_chars);
            self.current_chars = self.current.chars().count();
        } else {
            self.current_chars = 0;
        }
        chunk
    }

    fn document_chunk(&mut self, text: String) -> DocumentChunk {
        let index = self.next_index;
        self.next_index += 1;
        DocumentChunk {
            id: chunk_uuid(
                self.source_key,
                &self.document.external_id,
                &self.document.record_hash,
                index,
                self.generation,
            ),
            document_id: self.document_id,
            chunk_index: index,
            text,
            record_hash: self.document.record_hash.clone(),
        }
    }
}

impl Iterator for ChunkDocumentIter<'_> {
    type Item = DocumentChunk;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if self.finished {
                return None;
            }

            if let Some(mut pending) = self.pending.take() {
                if pending.separator_offset < PARAGRAPH_SEPARATOR.len() {
                    if self.current_chars == self.max_chars {
                        self.pending = Some(pending);
                        let chunk = self.take_chunk(true);
                        return Some(self.document_chunk(chunk));
                    }
                    let remaining_separator = &PARAGRAPH_SEPARATOR[pending.separator_offset..];
                    let capacity = self.max_chars - self.current_chars;
                    let take_len = prefix_len_by_chars(remaining_separator, capacity);
                    self.current.push_str(&remaining_separator[..take_len]);
                    self.raw.push_str(&remaining_separator[..take_len]);
                    pending.separator_offset += take_len;
                    self.current_chars += take_len;
                    if pending.separator_offset < PARAGRAPH_SEPARATOR.len() {
                        self.pending = Some(pending);
                        let chunk = self.take_chunk(true);
                        return Some(self.document_chunk(chunk));
                    }
                }

                if pending.offset < pending.text.len() {
                    if self.current_chars == self.max_chars {
                        self.pending = Some(pending);
                        let chunk = self.take_chunk(true);
                        return Some(self.document_chunk(chunk));
                    }

                    let remaining = &pending.text[pending.offset..];
                    let capacity = self.max_chars - self.current_chars;
                    let take_len = prefix_len_by_chars(remaining, capacity);
                    self.current.push_str(&remaining[..take_len]);
                    self.raw.push_str(&remaining[..take_len]);
                    pending.offset += take_len;
                    self.current_chars += remaining[..take_len].chars().count();

                    if pending.offset < pending.text.len() {
                        self.pending = Some(pending);
                        let chunk = self.take_chunk(true);
                        return Some(self.document_chunk(chunk));
                    }
                }

                continue;
            }

            let Some(paragraph) = self.next_paragraph() else {
                if self.current.is_empty() {
                    self.finished = true;
                    return None;
                }
                let chunk = self.take_chunk(false);
                self.finished = true;
                return Some(self.document_chunk(chunk));
            };

            let paragraph_chars = paragraph.chars().count();
            let separator_chars = if self.current.is_empty() { 0 } else { 2 };
            if self.current_chars + separator_chars + paragraph_chars <= self.max_chars {
                if separator_chars != 0 {
                    self.current.push_str(PARAGRAPH_SEPARATOR);
                    self.raw.push_str(PARAGRAPH_SEPARATOR);
                }
                self.current.push_str(paragraph);
                self.raw.push_str(paragraph);
                self.current_chars += separator_chars + paragraph_chars;
                continue;
            }

            if !self.current.is_empty() {
                let chunk = self.take_chunk(true);
                self.pending = Some(PendingParagraph {
                    text: paragraph,
                    offset: 0,
                    separator_offset: 0,
                });
                return Some(self.document_chunk(chunk));
            }

            self.pending = Some(PendingParagraph {
                text: paragraph,
                offset: 0,
                separator_offset: PARAGRAPH_SEPARATOR.len(),
            });
        }
    }
}

/// The selected overlap and source cut of one guarded character chunk.
struct SelectedCharCut {
    overlap: String,
    content_len: usize,
}

/// The largest positive source prefix of `buffer` that fits the character
/// budget and the exact embedding input envelope beside the chosen overlap.
///
/// Token counts are not monotonic in prefix length, so every UTF-8 content
/// boundary inside the bounded window is tried from largest to smallest; the
/// first fit is the largest for that overlap. The overlap gives up one leading
/// character only when no positive content can fit beside it.
fn guarded_char_cut(
    policy: &crate::chunking::chunking_token_artifact::ResolvedEmbeddingInputPolicy,
    overlap: &str,
    buffer: &str,
    max_chars: usize,
) -> Result<SelectedCharCut, ChunkingError> {
    let mut current_overlap = overlap.to_string();
    loop {
        let max_content_chars = max_chars.saturating_sub(current_overlap.chars().count());
        if max_content_chars > 0 {
            let byte_limit = prefix_len_by_chars(buffer, max_content_chars);
            let text = &buffer[..byte_limit];
            let mut boundaries = Vec::with_capacity(text.chars().count() + 1);
            boundaries.push(0);
            boundaries.extend(
                text.char_indices()
                    .map(|(index, character)| index + character.len_utf8()),
            );
            for characters in (1..boundaries.len()).rev() {
                let end = boundaries[characters];
                let candidate = format!("{current_overlap}{}", &text[..end]);
                if policy.count_input(policy.document_prefix(), &candidate)
                    <= policy.max_input_tokens().unwrap_or(usize::MAX)
                {
                    return Ok(SelectedCharCut {
                        overlap: current_overlap,
                        content_len: end,
                    });
                }
            }
        }
        if current_overlap.is_empty() {
            let first = buffer.chars().next().unwrap_or('\0');
            let count = policy.count_input(policy.document_prefix(), &first.to_string());
            return Err(ChunkingError::InputEnvelopeExceeded {
                role: "document",
                count,
                max_input_tokens: policy.max_input_tokens().unwrap_or(usize::MAX),
                prefix_chars: policy.document_prefix().chars().count(),
            });
        }
        let mut chars = current_overlap.chars();
        chars.next();
        current_overlap = chars.as_str().to_string();
    }
}

/// A character-budget iterator that also enforces the embedding input envelope.
///
/// It reuses the legacy splitter as the source of raw pieces, then re-flows
/// those pieces through the exact envelope. The source order and every byte are
/// preserved, and the returned chunk carries the same repeat suffix the legacy
/// budget used.
pub struct GuardedCharChunkIter<'a> {
    inner: ChunkDocumentIter<'a>,
    policy: crate::chunking::chunking_token_artifact::ResolvedEmbeddingInputPolicy,
    max_chars: usize,
    overlap_chars: usize,
    buffer: String,
    overlap: String,
    next_index: i32,
    done: bool,
}

impl<'a> GuardedCharChunkIter<'a> {
    fn new(
        document_id: i64,
        source_key: &'a str,
        document: &'a NormalizedDocument,
        config: &ChunkingConfig,
        generation: Option<i64>,
        policy: crate::chunking::chunking_token_artifact::ResolvedEmbeddingInputPolicy,
    ) -> Self {
        let inner = ChunkDocumentIter::new(document_id, source_key, document, config, generation);
        let max_chars = inner.max_chars;
        let overlap_chars = inner.overlap_chars;
        Self {
            inner,
            policy,
            max_chars,
            overlap_chars,
            buffer: String::new(),
            overlap: String::new(),
            next_index: 0,
            done: false,
        }
    }
}

impl Iterator for GuardedCharChunkIter<'_> {
    type Item = Result<DocumentChunk, ChunkingError>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if !self.done
                && self.buffer.chars().count() + self.overlap.chars().count() < self.max_chars
            {
                match self.inner.next() {
                    Some(_) => {
                        self.buffer
                            .push_str(&std::mem::take(&mut self.inner.last_raw));
                        continue;
                    }
                    None => {
                        self.done = true;
                        continue;
                    }
                }
            }
            if self.buffer.is_empty() {
                return None;
            }
            let cut =
                match guarded_char_cut(&self.policy, &self.overlap, &self.buffer, self.max_chars) {
                    Ok(cut) => cut,
                    Err(error) => return Some(Err(error)),
                };
            let content = self.buffer[..cut.content_len].to_string();
            self.buffer.drain(..cut.content_len);
            let mut text = String::with_capacity(cut.overlap.len() + content.len());
            text.push_str(&cut.overlap);
            text.push_str(&content);
            self.overlap = build_overlap_from_text(&text, self.overlap_chars);
            let index = self.next_index;
            self.next_index += 1;
            return Some(Ok(DocumentChunk {
                id: chunk_uuid(
                    self.inner.source_key,
                    &self.inner.document.external_id,
                    &self.inner.document.record_hash,
                    index,
                    self.inner.generation,
                ),
                document_id: self.inner.document_id,
                chunk_index: index,
                text,
                record_hash: self.inner.document.record_hash.clone(),
            }));
        }
    }
}

fn build_overlap_from_text(text: &str, overlap_chars: usize) -> String {
    if overlap_chars == 0 {
        return String::new();
    }
    let chars = text.chars().collect::<Vec<_>>();
    if chars.len() <= overlap_chars {
        return text.to_string();
    }
    chars[chars.len() - overlap_chars..].iter().collect()
}

fn prefix_len_by_chars(text: &str, count: usize) -> usize {
    text.char_indices()
        .nth(count)
        .map(|(index, _)| index)
        .unwrap_or(text.len())
}

fn chunk_uuid(
    source_key: &str,
    external_id: &str,
    record_hash: &str,
    chunk_index: i32,
    generation: Option<i64>,
) -> Uuid {
    let name = match generation {
        Some(generation) => {
            format!("{source_key}:{external_id}:{record_hash}:{chunk_index}:{generation}")
        }
        None => format!("{source_key}:{external_id}:{record_hash}:{chunk_index}"),
    };
    Uuid::new_v5(&Uuid::NAMESPACE_URL, name.as_bytes())
}

#[cfg(test)]
#[path = "chunking_token_tests.rs"]
mod chunking_token_tests;
#[cfg(test)]
#[path = "chunking_tests.rs"]
mod tests;

#[cfg(test)]
mod runtime_tests {
    use super::*;
    use serde_json::json;

    fn token_budget() -> ChunkBudget {
        ChunkBudget {
            mode: ChunkBudgetMode::Tokens,
            chars: ChunkingConfig {
                max_chars: 1000,
                overlap_chars: 100,
            },
            tokens: Some(TokenBudget {
                tokenizer: TokenizerRef::O200kBase,
                max_tokens: 512,
                overlap_tokens: 64,
            }),
            input_policy: None,
        }
    }

    #[test]
    fn a_pinned_budget_round_trips_and_preserves_other_keys() {
        let payload = json!({"external_id": "a", "content_base64": "abc"});
        let budget = token_budget();
        let pinned = payload_with_chunk_budget(&payload, &budget);

        assert_eq!(
            pinned.get("external_id").and_then(Value::as_str),
            Some("a"),
            "non-chunking keys survive the stamp"
        );
        assert_eq!(
            pinned.get("content_base64").and_then(Value::as_str),
            Some("abc")
        );
        assert_eq!(parse_chunk_budget(&pinned).expect("parse"), Some(budget));
    }

    #[test]
    fn a_malformed_snapshot_fails_closed_instead_of_falling_back() {
        let payload = json!({CHUNK_BUDGET_PAYLOAD_KEY: {"mode": "tokens"}});
        let error = parse_chunk_budget(&payload).expect_err("malformed must fail");
        assert!(matches!(error, ChunkingError::InvalidSnapshot { .. }));

        let legacy = ChunkBudget::chars(ChunkingConfig::default());
        assert!(
            resolve_chunk_budget(&payload, &legacy).is_err(),
            "a malformed snapshot is never replaced by the fallback"
        );
    }

    #[test]
    fn a_missing_snapshot_resolves_to_the_legacy_fallback() {
        let payload = json!({"external_id": "legacy"});
        let legacy = ChunkBudget::chars(ChunkingConfig {
            max_chars: 900,
            overlap_chars: 90,
        });
        let resolved = resolve_chunk_budget(&payload, &legacy).expect("legacy resolves");
        assert_eq!(resolved.identity(), "chars:900:90");
    }

    #[test]
    fn the_runtime_replaces_the_budget_and_keeps_the_legacy_characters() {
        let runtime = ChunkingRuntime::from(ChunkBudget::chars(ChunkingConfig {
            max_chars: 1000,
            overlap_chars: 100,
        }));
        assert_eq!(runtime.snapshot().identity(), "chars:1000:100");
        assert_eq!(runtime.legacy_chars_budget().identity(), "chars:1000:100");

        runtime.replace(token_budget());
        assert_eq!(runtime.snapshot().identity(), "tokens:o200k_base:512:64");
        assert_eq!(
            runtime.legacy_chars_budget().identity(),
            "chars:1000:100",
            "a legacy item keeps the character budget even while the live unit is tokens"
        );
    }

    #[test]
    fn a_clone_observes_the_same_replacement() {
        let runtime = ChunkingRuntime::default();
        let clone = runtime.clone();
        runtime.replace(token_budget());
        assert_eq!(clone.snapshot().identity(), "tokens:o200k_base:512:64");
    }
}
