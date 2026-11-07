use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tiktoken_rs::tokenizer::Tokenizer;
use tiktoken_rs::{CoreBPE, bpe_for_tokenizer};

use super::{ChunkingError, PARAGRAPH_SEPARATOR, chunk_uuid};
use crate::chunking::chunking_token_artifact::is_lower_sha256;
use crate::domain::{DocumentChunk, NormalizedDocument};

/// How many times the content of one chunk may be shortened or the overlap of
/// one chunk may give up room before the chunk is refused.
const CONTENT_SHORTEN_STEPS: usize = 4;

/// How many candidates the suffix search inspects.
const SUFFIX_SEARCH_STEPS: usize = 16;

/// Bytes the pending window reserves per token of the content budget.
///
/// The window only has to hold one candidate that can fill the content
/// budget, so its bound follows the configured budget and never the length of
/// the document.
const WINDOW_BYTES_PER_TOKEN: usize = 8;

/// The smallest pending window, in bytes.
const WINDOW_MIN_BYTES: usize = 32;

/// Tokenizer selection for the token-aware chunking path.
///
/// The encoders are the local BPE tables that ship with the `tiktoken-rs`
/// dependency. No model name is guessed and no vocabulary is downloaded, so a
/// token count only matches the tokenizer that produced it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TokenizerKind {
    Cl100kBase,
    O200kBase,
}

impl TokenizerKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Cl100kBase => "cl100k_base",
            Self::O200kBase => "o200k_base",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "cl100k_base" => Some(Self::Cl100kBase),
            "o200k_base" => Some(Self::O200kBase),
            _ => None,
        }
    }

    fn encoder(&self) -> &'static CoreBPE {
        let tokenizer = match self {
            Self::Cl100kBase => Tokenizer::Cl100kBase,
            Self::O200kBase => Tokenizer::O200kBase,
        };
        bpe_for_tokenizer(tokenizer).expect("tiktoken embeds both supported encoders")
    }
}

/// The durable tokenizer selection of one task or settings row.
///
/// The two legacy variants keep their exact names and identity strings. The
/// `hf_local` variant names one operator-provisioned artifact by hash, so a
/// task pins the exact bytes it counted with instead of a mutable file path or
/// a model alias.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TokenizerRef {
    Cl100kBase,
    O200kBase,
    HfLocal { sha256: String },
}

impl TokenizerRef {
    /// The persisted tokenizer name.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Cl100kBase => "cl100k_base",
            Self::O200kBase => "o200k_base",
            Self::HfLocal { .. } => "hf_local",
        }
    }

    /// The pinned artifact hash of a local Hugging Face tokenizer.
    pub fn sha256(&self) -> Option<&str> {
        match self {
            Self::HfLocal { sha256 } => Some(sha256),
            _ => None,
        }
    }

    /// The identity component of the budget.
    pub fn identity_name(&self) -> String {
        match self {
            Self::HfLocal { sha256 } => format!("hf_local:{sha256}"),
            other => other.name().to_string(),
        }
    }

    /// The legacy static engine, when this selection is one.
    pub fn static_kind(&self) -> Option<TokenizerKind> {
        match self {
            Self::Cl100kBase => Some(TokenizerKind::Cl100kBase),
            Self::O200kBase => Some(TokenizerKind::O200kBase),
            Self::HfLocal { .. } => None,
        }
    }

    /// Validates the variant payload.
    pub fn validate(&self) -> Result<(), ChunkingError> {
        if let Self::HfLocal { sha256 } = self
            && !is_lower_sha256(sha256)
        {
            return Err(ChunkingError::InvalidTokenBudget {
                reason: "hf_local requires a lowercase 64-hex artifact sha256".to_string(),
            });
        }
        Ok(())
    }
}

impl From<TokenizerKind> for TokenizerRef {
    fn from(kind: TokenizerKind) -> Self {
        match kind {
            TokenizerKind::Cl100kBase => Self::Cl100kBase,
            TokenizerKind::O200kBase => Self::O200kBase,
        }
    }
}

/// One loaded tokenizer engine.
#[derive(Clone)]
pub enum TokenizerEngine {
    Tiktoken(TokenizerKind),
    Hf(Arc<tokenizers::Tokenizer>),
}

impl TokenizerEngine {
    pub fn static_kind(kind: TokenizerKind) -> Self {
        Self::Tiktoken(kind)
    }

    pub fn hf(tokenizer: Arc<tokenizers::Tokenizer>) -> Self {
        Self::Hf(tokenizer)
    }

    fn encode_ids(&self, text: &str) -> Vec<u32> {
        match self {
            Self::Tiktoken(kind) => kind.encoder().encode_ordinary(text),
            // No special tokens and no truncation or padding: the count is the
            // candidate's own tokens, exactly like encode_ordinary.
            Self::Hf(tokenizer) => tokenizer
                .encode(text, false)
                .map(|encoding| encoding.get_ids().to_vec())
                .unwrap_or_default(),
        }
    }

    fn decode_bytes(&self, tokens: &[u32]) -> Option<Vec<u8>> {
        match self {
            Self::Tiktoken(kind) => kind.encoder().decode_bytes(tokens).ok(),
            Self::Hf(tokenizer) => tokenizer
                .decode(tokens, true)
                .ok()
                .map(|text| text.into_bytes()),
        }
    }

    /// Number of tokens this engine produces for `text`.
    pub fn count(&self, text: &str) -> usize {
        self.encode_ids(text).len()
    }

    /// Number of tokens this engine produces for `text` with its special
    /// tokens added when requested.
    ///
    /// A built-in tiktoken encoder has no special-token template, so the
    /// caller must reject that combination before it reaches this method.
    pub fn count_with_special(&self, text: &str, add_special_tokens: bool) -> usize {
        match self {
            Self::Tiktoken(kind) => kind.encoder().encode_ordinary(text).len(),
            Self::Hf(tokenizer) => tokenizer
                .encode(text, add_special_tokens)
                .map(|encoding| encoding.get_ids().len())
                .unwrap_or_default(),
        }
    }

    fn suffix<'a>(&self, text: &'a str, max_tokens: usize) -> &'a str {
        if text.is_empty() || max_tokens == 0 {
            return "";
        }
        let tokens = self.encode_ids(text);
        if tokens.len() <= max_tokens {
            return text;
        }
        let mut take = max_tokens.min(tokens.len());
        for _ in 0..SUFFIX_SEARCH_STEPS {
            match suffix_candidate(self, text, &tokens, take, max_tokens) {
                SuffixCandidate::Fits(candidate) => return candidate,
                SuffixCandidate::InsideCharacter => {
                    if take == 0 {
                        break;
                    }
                    take -= 1;
                }
                SuffixCandidate::OverBudget => take /= 2,
            }
        }
        ""
    }

    fn prefix_within<'a>(&self, window: &'a str, limit: usize) -> Option<&'a str> {
        let tokens = self.encode_ids(window);
        if tokens.len() <= limit {
            return Some(window);
        }
        for take in (1..=tokens.len()).rev() {
            if let Some(candidate) = prefix_candidate(self, window, &tokens, take, limit) {
                return Some(candidate);
            }
        }
        None
    }
}

/// One resolved tokenizer selection and its loaded engine.
#[derive(Clone)]
pub struct ResolvedTokenizer {
    reference: TokenizerRef,
    engine: Arc<TokenizerEngine>,
}

impl ResolvedTokenizer {
    pub fn from_static(kind: TokenizerKind) -> Self {
        Self {
            reference: kind.into(),
            engine: Arc::new(TokenizerEngine::static_kind(kind)),
        }
    }

    pub fn from_hf(sha256: String, tokenizer: Arc<tokenizers::Tokenizer>) -> Self {
        Self {
            reference: TokenizerRef::HfLocal { sha256 },
            engine: Arc::new(TokenizerEngine::hf(tokenizer)),
        }
    }

    pub fn reference(&self) -> &TokenizerRef {
        &self.reference
    }

    pub fn engine(&self) -> &TokenizerEngine {
        &self.engine
    }

    pub fn count(&self, text: &str) -> usize {
        self.engine.count(text)
    }

    /// Number of tokens this selection produces with its special-token policy.
    pub fn count_with_special(&self, text: &str, add_special_tokens: bool) -> usize {
        self.engine.count_with_special(text, add_special_tokens)
    }

    pub fn suffix<'a>(&self, text: &'a str, max_tokens: usize) -> &'a str {
        self.engine.suffix(text, max_tokens)
    }
}

/// Number of tokens the selected tokenizer produces for `text`.
pub fn token_count(text: &str, tokenizer: TokenizerKind) -> usize {
    tokenizer.encoder().encode_ordinary(text).len()
}

/// A suffix of `text` that encodes to at most `max_tokens` tokens.
///
/// The suffix starts on a character boundary, so it never splits a Unicode
/// scalar value, and it is verified with an exact encoding: a token boundary
/// can encode to more tokens on its own, so a suffix that does not fit the
/// budget is shortened instead of being returned. The result is bounded, not
/// necessarily the longest possible suffix.
pub fn token_suffix(text: &str, tokenizer: TokenizerKind, max_tokens: usize) -> &str {
    TokenizerEngine::static_kind(tokenizer).suffix(text, max_tokens)
}

/// The outcome of one suffix candidate.
enum SuffixCandidate<'a> {
    /// The candidate fits the budget.
    Fits(&'a str),
    /// The candidate starts inside a character.
    InsideCharacter,
    /// The candidate decodes but does not fit the budget.
    OverBudget,
}

/// The suffix of `text` that the last `take` tokens cover.
fn suffix_candidate<'a>(
    engine: &TokenizerEngine,
    text: &'a str,
    tokens: &[u32],
    take: usize,
    max_tokens: usize,
) -> SuffixCandidate<'a> {
    let Some(bytes) = engine.decode_bytes(&tokens[tokens.len() - take..]) else {
        return SuffixCandidate::InsideCharacter;
    };
    let mut start = text.len().saturating_sub(bytes.len());
    while start < text.len() && !text.is_char_boundary(start) {
        start += 1;
    }
    let candidate = &text[start..];
    if engine.count(candidate) <= max_tokens {
        SuffixCandidate::Fits(candidate)
    } else {
        SuffixCandidate::OverBudget
    }
}

/// Streaming chunker for the token-budget path.
///
/// The iterator emits one document chunk at a time. It keeps a borrowed
/// paragraph cursor and one bounded window of normalized text, so a long
/// document never grows the pending state and no paragraph is materialized as
/// a list of pieces. Every chunk is cut from that one bounded candidate and
/// verified with an exact encoding: no accumulated prefix is re-encoded per
/// appended piece.
pub struct TokenChunkDocumentIter<'a> {
    document_id: i64,
    source_key: &'a str,
    document: &'a NormalizedDocument,
    generation: Option<i64>,
    tokenizer: ResolvedTokenizer,
    max_tokens: usize,
    overlap_tokens: usize,
    /// The pinned embedding input envelope, absent for a legacy budget.
    input_policy: Option<super::chunking_token_artifact::ResolvedEmbeddingInputPolicy>,
    /// The paragraphs of the normalized source, in order.
    paragraphs: std::str::Split<'a, &'static str>,
    /// The text of the current paragraph that is not in the window yet.
    paragraph: &'a str,
    /// Whether a paragraph separator is still owed before the text of the
    /// current paragraph is appended.
    ///
    /// The separator is appended only when the window has room for it, so it
    /// waits here instead of being dropped or pushed past the window bound.
    separator_due: bool,
    /// Whether a paragraph was already taken, so the next one is preceded by
    /// a separator.
    seen_paragraph: bool,
    /// The bounded window of normalized text that is not emitted yet.
    window: String,
    /// The overlap the next chunk repeats.
    next_overlap: Option<String>,
    next_index: i32,
    finished: bool,
    failure: Option<ChunkingError>,
}

impl<'a> TokenChunkDocumentIter<'a> {
    pub fn new(
        document_id: i64,
        source_key: &'a str,
        document: &'a NormalizedDocument,
        tokenizer: TokenizerKind,
        max_tokens: usize,
        overlap_tokens: usize,
        generation: Option<i64>,
    ) -> Result<Self, ChunkingError> {
        Self::new_resolved(
            document_id,
            source_key,
            document,
            ResolvedTokenizer::from_static(tokenizer),
            max_tokens,
            overlap_tokens,
            generation,
        )
    }

    pub fn new_resolved(
        document_id: i64,
        source_key: &'a str,
        document: &'a NormalizedDocument,
        tokenizer: ResolvedTokenizer,
        max_tokens: usize,
        overlap_tokens: usize,
        generation: Option<i64>,
    ) -> Result<Self, ChunkingError> {
        if max_tokens == 0 {
            return Err(ChunkingError::InvalidTokenBudget {
                reason: "max_tokens must be greater than zero".to_string(),
            });
        }
        if overlap_tokens >= max_tokens {
            return Err(ChunkingError::InvalidTokenBudget {
                reason: "overlap_tokens must be smaller than max_tokens".to_string(),
            });
        }
        Ok(Self {
            document_id,
            source_key,
            document,
            generation,
            tokenizer,
            max_tokens,
            overlap_tokens,
            input_policy: None,
            paragraphs: document.body_text.split(PARAGRAPH_SEPARATOR),
            paragraph: "",
            separator_due: false,
            seen_paragraph: false,
            window: String::new(),
            next_overlap: None,
            next_index: 0,
            finished: false,
            failure: None,
        })
    }

    /// Attaches the pinned embedding input envelope.
    pub fn with_input_policy(
        mut self,
        policy: Option<super::chunking_token_artifact::ResolvedEmbeddingInputPolicy>,
    ) -> Self {
        self.input_policy = policy;
        self
    }

    /// Iterator that reports a configuration failure on its first item.
    pub fn with_failure(
        document_id: i64,
        source_key: &'a str,
        document: &'a NormalizedDocument,
        error: ChunkingError,
    ) -> Self {
        Self {
            document_id,
            source_key,
            document,
            generation: None,
            tokenizer: ResolvedTokenizer::from_static(TokenizerKind::Cl100kBase),
            max_tokens: 1,
            overlap_tokens: 0,
            input_policy: None,
            paragraphs: document.body_text.split(PARAGRAPH_SEPARATOR),
            paragraph: "",
            separator_due: false,
            seen_paragraph: false,
            window: String::new(),
            next_overlap: None,
            next_index: 0,
            finished: false,
            failure: Some(error),
        }
    }

    /// The token budget the content of the next chunk may use.
    ///
    /// The repeated overlap of the previous chunk is optional, so the content
    /// budget shrinks with it and the overlap gives up room when the content
    /// cannot be represented.
    fn content_limit(&self, overlap_tokens: usize) -> usize {
        self.max_tokens.saturating_sub(overlap_tokens).max(1)
    }

    /// The size the pending window may reach, in bytes.
    ///
    /// The bound follows the configured budget, not the document length.
    fn window_target_bytes(&self, overlap_tokens: usize) -> usize {
        self.content_limit(overlap_tokens)
            .saturating_mul(WINDOW_BYTES_PER_TOKEN)
            .saturating_add(WINDOW_MIN_BYTES)
    }

    /// Fills the pending window with normalized source text.
    ///
    /// The text is appended from the paragraph cursor without tokenizing what
    /// is appended, and every separator of the normalized source is preserved.
    ///
    /// The window never grows past its bound. A separator that does not fit
    /// the remaining capacity waits for the next output instead of being
    /// appended anyway, and the paragraph it belongs to stays in the cursor,
    /// so no separator is dropped and no text is lost.
    fn fill_window(&mut self, overlap_tokens: usize) {
        let target = self.window_target_bytes(overlap_tokens);
        while self.window.len() < target {
            if self.paragraph.is_empty() {
                let Some(next) = self.paragraphs.find_map(|value| {
                    let value = value.trim();
                    (!value.is_empty()).then_some(value)
                }) else {
                    break;
                };
                self.paragraph = next;
                if self.seen_paragraph {
                    // Every paragraph after the first one is preceded by a
                    // separator.
                    self.separator_due = true;
                } else {
                    self.seen_paragraph = true;
                }
            }
            // The separator belongs in front of the paragraph it follows. It
            // is appended only when the window has room for it, so it waits
            // for the next output instead of being pushed past the window
            // bound or dropped.
            if self.separator_due {
                let Some(room) = target.checked_sub(self.window.len()) else {
                    break;
                };
                if room < PARAGRAPH_SEPARATOR.len() {
                    break;
                }
                self.window.push_str(PARAGRAPH_SEPARATOR);
                self.separator_due = false;
            }
            let Some(room) = target.checked_sub(self.window.len()) else {
                break;
            };
            let mut take = self.paragraph.len().min(room);
            while take > 0 && !self.paragraph.is_char_boundary(take) {
                take -= 1;
            }
            if take == 0 {
                break;
            }
            self.window.push_str(&self.paragraph[..take]);
            self.paragraph = &self.paragraph[take..];
        }
    }

    /// The cut position of the next chunk inside the window.
    ///
    /// A chunk that has to cut the window prefers a paragraph boundary, so a
    /// paragraph stays whole whenever the budget allows it.
    fn preferred_cut(&self, prefix_len: usize) -> usize {
        if prefix_len >= self.window.len() {
            return self.window.len();
        }
        self.window[..prefix_len]
            .rfind(PARAGRAPH_SEPARATOR)
            .filter(|position| *position > 0)
            .unwrap_or(prefix_len)
    }

    /// The first character of the window and the tokens it needs.
    fn first_character(&self) -> Result<(char, usize), ChunkingError> {
        let Some(character) = self.window.chars().next() else {
            return Err(ChunkingError::TokenBudgetExceeded {
                max_tokens: self.max_tokens,
            });
        };
        let tokens = self.tokenizer.count(&self.window[..character.len_utf8()]);
        Ok((character, tokens))
    }

    /// Cuts the content of the next chunk out of the window.
    ///
    /// The window is the candidate, so one cut encodes one bounded window
    /// instead of an accumulated prefix. The overlap is shortened when the
    /// content needs the room: the overlap is optional, and forward progress
    /// is never traded for it.
    fn cut_content(
        &mut self,
        overlap: &mut String,
        overlap_tokens: &mut usize,
    ) -> Result<usize, ChunkingError> {
        let mut limit = self.content_limit(*overlap_tokens);
        let mut shortenings = 0usize;
        loop {
            let Some(prefix) = self.tokenizer.engine().prefix_within(&self.window, limit) else {
                let (character, tokens) = self.first_character()?;
                if tokens > self.max_tokens {
                    return Err(ChunkingError::CharacterExceedsBudget {
                        character,
                        tokens,
                        max_tokens: self.max_tokens,
                    });
                }
                // The overlap is optional: give the content the room it needs.
                let allowed = self.max_tokens - tokens;
                *overlap = self.tokenizer.suffix(overlap, allowed).to_string();
                *overlap_tokens = self.tokenizer.count(overlap);
                limit = self.content_limit(*overlap_tokens);
                shortenings += 1;
                if shortenings > CONTENT_SHORTEN_STEPS {
                    return Err(ChunkingError::TokenBudgetExceeded {
                        max_tokens: self.max_tokens,
                    });
                }
                continue;
            };
            let cut = self.preferred_cut(prefix.len());
            let content = &self.window[..cut];
            let mut text = String::with_capacity(overlap.len() + content.len());
            text.push_str(overlap);
            text.push_str(content);
            if self.tokenizer.count(&text) <= self.max_tokens {
                return Ok(cut);
            }
            // The token counts of the overlap and the content do not simply
            // add up at their boundary. The content is shortened first, so
            // the overlap stays the repeated suffix of the previous chunk.
            shortenings += 1;
            if cut == 0 || shortenings > CONTENT_SHORTEN_STEPS {
                return Err(ChunkingError::TokenBudgetExceeded {
                    max_tokens: self.max_tokens,
                });
            }
            limit = limit.saturating_sub(1).max(1);
        }
    }

    /// Whether the pinned envelope admits one complete input.
    fn envelope_fits(&self, text: &str) -> bool {
        match &self.input_policy {
            Some(policy) => {
                policy.count_input(policy.document_prefix(), text)
                    <= policy.max_input_tokens().unwrap_or(usize::MAX)
            }
            None => true,
        }
    }

    /// The exact envelope token count of one already-built chunk.
    fn envelope_count(&self, text: &str) -> usize {
        match &self.input_policy {
            Some(policy) => policy.count_input(policy.document_prefix(), text),
            None => 0,
        }
    }

    /// The largest positive byte prefix of the window that fits `overlap`.
    ///
    /// Token counts are not monotonic in prefix length, so the UTF-8
    /// boundaries are scanned from largest to smallest instead of being
    /// binary-searched. Returns `0` when not even one character fits beside
    /// the overlap.
    fn largest_content_prefix(&self, overlap: &str, max_len: usize) -> usize {
        let text = &self.window[..max_len];
        let mut boundaries = Vec::with_capacity(text.chars().count() + 1);
        boundaries.push(0);
        boundaries.extend(
            text.char_indices()
                .map(|(index, character)| index + character.len_utf8()),
        );
        for characters in (1..boundaries.len()).rev() {
            let end = boundaries[characters];
            if self.envelope_fits(&format!("{overlap}{}", &text[..end])) {
                return end;
            }
        }
        0
    }

    /// Produces the next chunk of the document.
    fn next_chunk(&mut self) -> Result<Option<DocumentChunk>, ChunkingError> {
        let mut overlap = self.next_overlap.take().unwrap_or_default();
        let mut overlap_tokens = self.tokenizer.count(&overlap);
        self.fill_window(overlap_tokens);
        if self.window.is_empty() {
            // The normalized source is exhausted.
            return Ok(None);
        }
        let cut = self.cut_content(&mut overlap, &mut overlap_tokens)?;
        let mut content_len = cut;
        if self.input_policy.is_some() {
            loop {
                let candidate = format!("{overlap}{}", &self.window[..content_len]);
                if self.envelope_fits(&candidate) {
                    break;
                }
                let fit = self.largest_content_prefix(&overlap, content_len);
                if fit > 0 {
                    content_len = fit;
                    break;
                }
                // The overlap is optional: give it up until one source
                // character can make positive progress.
                if overlap.is_empty() {
                    let first = self.window.chars().next().unwrap_or('\0');
                    let count = self.envelope_count(&format!("{first}"));
                    let (max_input_tokens, prefix_chars) = match &self.input_policy {
                        Some(policy) => (
                            policy.max_input_tokens().unwrap_or(usize::MAX),
                            policy.document_prefix().chars().count(),
                        ),
                        None => (usize::MAX, 0),
                    };
                    return Err(ChunkingError::InputEnvelopeExceeded {
                        role: "document",
                        count,
                        max_input_tokens,
                        prefix_chars,
                    });
                }
                let mut chars = overlap.chars();
                chars.next();
                overlap = chars.as_str().to_string();
            }
        }
        let content = self.window[..content_len].to_string();
        self.window.drain(..content_len);
        Ok(Some(self.build_chunk(&overlap, &content)))
    }

    /// Builds the chunk of one cut content.
    ///
    /// The next overlap is the bounded suffix of the content, so every chunk
    /// after the first repeats a suffix of the content before it.
    fn build_chunk(&mut self, overlap: &str, content: &str) -> DocumentChunk {
        let index = self.next_index;
        self.next_index += 1;
        let mut text = String::with_capacity(overlap.len() + content.len());
        text.push_str(overlap);
        text.push_str(content);
        self.next_overlap = Some(
            self.tokenizer
                .suffix(content, self.overlap_tokens)
                .to_string(),
        );
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

impl Iterator for TokenChunkDocumentIter<'_> {
    type Item = Result<DocumentChunk, ChunkingError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.finished {
            return None;
        }
        if let Some(error) = self.failure.take() {
            self.finished = true;
            return Some(Err(error));
        }
        match self.next_chunk() {
            Ok(Some(chunk)) => Some(Ok(chunk)),
            Ok(None) => {
                self.finished = true;
                None
            }
            Err(error) => {
                self.finished = true;
                Some(Err(error))
            }
        }
    }
}

/// The prefix of `window` that the first `take` tokens cover.
fn prefix_candidate<'a>(
    engine: &TokenizerEngine,
    window: &'a str,
    tokens: &[u32],
    take: usize,
    limit: usize,
) -> Option<&'a str> {
    let bytes = engine.decode_bytes(&tokens[..take])?;
    let mut end = bytes.len().min(window.len());
    while end > 0 && !window.is_char_boundary(end) {
        end -= 1;
    }
    if end == 0 {
        return None;
    }
    let candidate = &window[..end];
    if engine.count(candidate) <= limit {
        Some(candidate)
    } else {
        None
    }
}
