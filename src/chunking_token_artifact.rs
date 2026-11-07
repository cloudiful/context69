//! Verified, operator-provisioned local Hugging Face tokenizer artifacts.
//!
//! An artifact is one standard `tokenizer.json` file named by the lowercase
//! SHA256 of its own bytes: `<artifact-dir>/<64-hex>.json`. The catalog verifies
//! the name, the bytes, and the readable tokenizer at startup, then serves the
//! parsed engines by hash. Nothing is downloaded, no weights or model names are
//! consulted, and no arbitrary path is resolved.
//!
//! The catalog is immutable for one process: adding an artifact requires a
//! restart, and an old artifact must be kept while a task pinned its hash.

use std::{
    collections::BTreeMap,
    fs,
    path::Path,
    sync::{Arc, OnceLock},
};

use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};

pub use context69_contracts::settings::TokenizerArtifactDescriptor;

/// The largest artifact the catalog accepts.
pub const TOKENIZER_ARTIFACT_MAX_BYTES: u64 = 32 * 1024 * 1024;

/// The process-wide catalog installed once at startup.
static INSTALLED: OnceLock<Arc<TokenizerCatalog>> = OnceLock::new();

/// The immutable set of verified local tokenizer artifacts.
#[derive(Debug)]
pub struct TokenizerCatalog {
    entries: BTreeMap<String, Arc<tokenizers::Tokenizer>>,
    descriptors: Vec<TokenizerArtifactDescriptor>,
}

impl TokenizerCatalog {
    /// An empty catalog: legacy tokenizers keep working without any artifact.
    pub fn empty() -> Arc<Self> {
        Arc::new(Self {
            entries: BTreeMap::new(),
            descriptors: Vec::new(),
        })
    }

    /// Installs the process catalog. A second install is rejected instead of
    /// silently replacing the authority the runtime already resolved against.
    pub fn install(catalog: Arc<Self>) -> Result<()> {
        INSTALLED
            .set(catalog)
            .map_err(|_| anyhow::anyhow!("the tokenizer catalog is already installed"))
    }

    /// The installed catalog, or the empty catalog when none was installed.
    pub fn installed_or_empty() -> Arc<Self> {
        INSTALLED
            .get()
            .cloned()
            .unwrap_or_else(TokenizerCatalog::empty)
    }

    /// Loads and installs the process catalog once.
    ///
    /// A later call observes the catalog installed first, so a second boot
    /// path or a test never replaces the authority the runtime already holds.
    pub fn ensure_installed(directory: &Path) -> Result<Arc<Self>> {
        if let Some(installed) = INSTALLED.get() {
            return Ok(Arc::clone(installed));
        }
        let catalog = Self::load(directory)?;
        match INSTALLED.set(Arc::clone(&catalog)) {
            Ok(()) => Ok(catalog),
            Err(_) => Ok(Self::installed_or_empty()),
        }
    }

    /// Scans one operator directory.
    ///
    /// A missing directory is the legacy default and yields an empty catalog.
    /// Every `*.json` entry must be a regular file named by its own lowercase
    /// SHA256 and must parse as a standard tokenizer; anything else is refused
    /// with the file name or hash alone, never with the file contents.
    pub fn load(directory: &Path) -> Result<Arc<Self>> {
        if !directory.exists() {
            return Ok(Self::empty());
        }
        if !directory.is_dir() {
            bail!(
                "the tokenizer artifact path {} is not a directory",
                directory.display()
            );
        }
        let mut paths = Vec::new();
        for entry in fs::read_dir(directory)
            .with_context(|| format!("read tokenizer artifact directory {}", directory.display()))?
        {
            let entry = entry.with_context(|| {
                format!(
                    "read an entry of tokenizer artifact directory {}",
                    directory.display()
                )
            })?;
            let path = entry.path();
            if path.extension().and_then(|value| value.to_str()) != Some("json") {
                continue;
            }
            paths.push(path);
        }
        paths.sort();

        let mut entries = BTreeMap::new();
        let mut descriptors = Vec::new();
        for path in paths {
            let name = path
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or("<unnamed>")
                .to_string();
            let stem = path
                .file_stem()
                .and_then(|value| value.to_str())
                .unwrap_or_default();
            if !is_lower_sha256(stem) {
                bail!("the tokenizer artifact {name} must be named by its lowercase SHA256 hex");
            }
            let metadata = fs::metadata(&path)
                .with_context(|| format!("read tokenizer artifact metadata for {name}"))?;
            if !metadata.is_file() {
                bail!("the tokenizer artifact {name} is not a regular file");
            }
            if metadata.len() > TOKENIZER_ARTIFACT_MAX_BYTES {
                bail!("the tokenizer artifact {name} exceeds {TOKENIZER_ARTIFACT_MAX_BYTES} bytes");
            }
            let bytes =
                fs::read(&path).with_context(|| format!("read tokenizer artifact {name}"))?;
            let actual = hash_bytes(&bytes);
            if actual != stem {
                bail!("the tokenizer artifact {name} does not match its name hash {actual}");
            }
            let mut tokenizer = tokenizers::Tokenizer::from_bytes(&bytes).map_err(|_| {
                anyhow::anyhow!("the tokenizer artifact {name} is not a readable tokenizer.json")
            })?;
            tokenizer.with_truncation(None).map_err(|_| {
                anyhow::anyhow!("the tokenizer artifact {name} disables truncation")
            })?;
            tokenizer.with_padding(None);
            entries.insert(stem.to_string(), Arc::new(tokenizer));
            descriptors.push(TokenizerArtifactDescriptor {
                sha256: stem.to_string(),
                size_bytes: bytes.len() as u64,
            });
        }
        descriptors.sort_by(|left, right| left.sha256.cmp(&right.sha256));
        Ok(Arc::new(Self {
            entries,
            descriptors,
        }))
    }

    /// The verified engine for one hash.
    pub fn get(&self, sha256: &str) -> Option<Arc<tokenizers::Tokenizer>> {
        self.entries.get(sha256).cloned()
    }

    /// The verified engine for one hash, or an actionable refusal.
    pub fn require(&self, sha256: &str) -> Result<Arc<tokenizers::Tokenizer>> {
        self.get(sha256).ok_or_else(|| {
            anyhow::anyhow!(
                "the local tokenizer artifact {sha256} is not available in the verified catalog; provision {sha256}.json and restart"
            )
        })
    }

    /// The public descriptors of every verified artifact.
    pub fn descriptors(&self) -> &[TokenizerArtifactDescriptor] {
        &self.descriptors
    }
}

/// The installed verified engine for one hash.
pub fn require_installed(sha256: &str) -> Result<Arc<tokenizers::Tokenizer>> {
    TokenizerCatalog::installed_or_empty().require(sha256)
}

/// Whether a value is exactly 64 lowercase hexadecimal characters.
pub fn is_lower_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}

/// The lowercase SHA256 hex of the bytes.
pub fn hash_bytes(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut hex = String::with_capacity(64);
    for byte in digest {
        hex.push_str(&format!("{byte:02x}"));
    }
    hex
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        path::{Path, PathBuf},
        time::{SystemTime, UNIX_EPOCH},
    };

    use crate::chunking::chunking_token::TokenizerRef;
    use crate::chunking::{ChunkBudget, ChunkingRuntime};

    /// A GPT-2 style byte-to-character mapping for a byte-level vocabulary.
    fn byte_char(byte: u8) -> char {
        let value = byte as u32;
        if (33..=126).contains(&value)
            || (161..=172).contains(&value)
            || (174..=255).contains(&value)
        {
            char::from_u32(value).expect("a printable byte maps to itself")
        } else {
            char::from_u32(256 + value).expect("a control byte maps into the private range")
        }
    }

    /// A standard tiny byte-level BPE `tokenizer.json` with one token per byte.
    fn tiny_byte_level_json(truncate: bool) -> Vec<u8> {
        let vocab: serde_json::Map<String, serde_json::Value> = (0u8..=255)
            .map(|byte| (byte_char(byte).to_string(), serde_json::json!(byte as u32)))
            .collect();
        let mut value = serde_json::json!({
            "version": "1.0",
            "truncation": null,
            "padding": null,
            "added_tokens": [],
            "normalizer": null,
            "pre_tokenizer": {
                "type": "ByteLevel",
                "add_prefix_space": false,
                "trim_offsets": true,
                "use_regex": true
            },
            "post_processor": null,
            "decoder": {
                "type": "ByteLevel",
                "add_prefix_space": true,
                "trim_offsets": true,
                "use_regex": true
            },
            "model": {
                "type": "BPE",
                "dropout": null,
                "unk_token": null,
                "continuing_subword_prefix": null,
                "end_of_word_suffix": null,
                "fuse_unk": false,
                "byte_fallback": false,
                "vocab": vocab,
                "merges": []
            }
        });
        if truncate {
            value["truncation"] = serde_json::json!({
                "max_length": 1,
                "strategy": "LongestFirst",
                "stride": 0,
                "direction": "Right"
            });
        }
        serde_json::to_vec(&value).expect("the tiny tokenizer serializes")
    }

    fn temp_directory() -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos();
        let directory = std::env::temp_dir().join(format!("context69-tokenizers-{unique}"));
        std::fs::create_dir_all(&directory).expect("create the artifact directory");
        directory
    }

    fn write_artifact(directory: &Path, name: &str, bytes: &[u8]) -> String {
        std::fs::write(directory.join(format!("{name}.json")), bytes).expect("write the artifact");
        name.to_string()
    }

    #[test]
    fn a_verified_artifact_loads_and_its_hash_resolves() {
        let directory = temp_directory();
        let bytes = tiny_byte_level_json(false);
        let sha256 = hash_bytes(&bytes);
        write_artifact(&directory, &sha256, &bytes);

        let catalog = TokenizerCatalog::load(&directory).expect("load the catalog");
        assert_eq!(catalog.descriptors().len(), 1);
        assert_eq!(catalog.descriptors()[0].sha256, sha256);
        assert_eq!(catalog.descriptors()[0].size_bytes, bytes.len() as u64);
        let engine = catalog.require(&sha256).expect("the artifact resolves");
        assert_eq!(
            engine
                .encode("hello", false)
                .expect("encode")
                .get_ids()
                .len(),
            5
        );

        std::fs::remove_dir_all(&directory).expect("remove the artifact directory");
    }

    #[test]
    fn truncation_and_padding_are_disabled_before_counting() {
        let directory = temp_directory();
        let bytes = tiny_byte_level_json(true);
        let sha256 = hash_bytes(&bytes);
        write_artifact(&directory, &sha256, &bytes);

        let catalog = TokenizerCatalog::load(&directory).expect("load the catalog");
        let engine = catalog.require(&sha256).expect("the artifact resolves");
        assert_eq!(
            engine
                .encode("hello", false)
                .expect("encode")
                .get_ids()
                .len(),
            5,
            "loading must clear truncation so the count is the candidate's own tokens"
        );

        std::fs::remove_dir_all(&directory).expect("remove the artifact directory");
    }

    #[test]
    fn a_name_that_does_not_match_the_bytes_is_refused() {
        let directory = temp_directory();
        let bytes = tiny_byte_level_json(false);
        let wrong = "0".repeat(64);
        write_artifact(&directory, &wrong, &bytes);

        let error = TokenizerCatalog::load(&directory).expect_err("a mismatch is refused");
        assert!(error.to_string().contains(&wrong), "{error}");
        assert!(!error.to_string().contains("pre_tokenizer"), "no contents");

        std::fs::remove_dir_all(&directory).expect("remove the artifact directory");
    }

    #[test]
    fn a_missing_directory_is_an_empty_catalog_and_non_json_is_ignored() {
        let missing = std::env::temp_dir().join("context69-tokenizers-does-not-exist");
        assert!(
            TokenizerCatalog::load(&missing)
                .expect("missing is empty")
                .descriptors()
                .is_empty()
        );

        let directory = temp_directory();
        std::fs::write(directory.join("notes.txt"), b"ignored").expect("write a non-json file");
        assert!(
            TokenizerCatalog::load(&directory)
                .expect("load")
                .descriptors()
                .is_empty()
        );
        std::fs::remove_dir_all(&directory).expect("remove the artifact directory");
    }

    #[test]
    fn a_pinned_hash_cuts_with_the_loaded_artifact_and_keeps_old_identity() {
        let directory = temp_directory();
        let bytes = tiny_byte_level_json(false);
        let sha256 = hash_bytes(&bytes);
        write_artifact(&directory, &sha256, &bytes);
        let catalog = TokenizerCatalog::load(&directory).expect("load the catalog");

        let budget = ChunkBudget::tokens_ref(
            TokenizerRef::HfLocal {
                sha256: sha256.clone(),
            },
            3,
            0,
        );
        assert_eq!(budget.identity(), format!("tokens:hf_local:{sha256}:3:0"));
        let runtime = ChunkingRuntime::with_catalog(budget.clone(), catalog);
        let resolved = runtime
            .resolve(&budget)
            .expect("resolve the pinned artifact");
        assert_eq!(resolved.budget().identity(), budget.identity());

        let document = crate::domain::NormalizedDocument {
            external_id: "doc-1".to_string(),
            title: "title".to_string(),
            summary: None,
            body_text: "hello".to_string(),
            source_uri: "context69://test".to_string(),
            published_at: None,
            updated_at: chrono::Utc::now(),
            metadata_json: serde_json::json!({}),
            record_hash: "hash-1".to_string(),
        };
        let chunks = resolved
            .chunk_document(1, "file_library", &document)
            .expect("the artifact chunks the source");
        assert!(
            chunks
                .iter()
                .all(|chunk| chunk.text == "hello" || chunk.text.len() <= 3)
        );
        assert_eq!(
            chunks
                .iter()
                .map(|chunk| chunk.text.as_str())
                .collect::<String>(),
            "hello",
            "the cut never changes the source text"
        );

        assert_eq!(
            ChunkBudget::tokens(
                crate::chunking::chunking_token::TokenizerKind::O200kBase,
                3,
                0
            )
            .identity(),
            "tokens:o200k_base:3:0",
            "legacy identities stay byte-identical"
        );

        std::fs::remove_dir_all(&directory).expect("remove the artifact directory");
    }
}

/// One embedding input policy resolved against the immutable catalog.
///
/// The already-loaded tokenizer is held for the duration of one counting pass,
/// so a provider or chunking path never touches the filesystem per token.
#[derive(Clone)]
pub struct ResolvedEmbeddingInputPolicy {
    policy: context69_contracts::settings::EmbeddingInputPolicy,
    tokenizer: super::chunking_token::ResolvedTokenizer,
}

impl ResolvedEmbeddingInputPolicy {
    /// The policy value this resolution carries.
    pub fn policy(&self) -> &context69_contracts::settings::EmbeddingInputPolicy {
        &self.policy
    }

    /// The configured full input-token cap, or `None` when the guard is off.
    pub fn max_input_tokens(&self) -> Option<usize> {
        self.policy.max_input_tokens
    }

    /// The server-added document prefix, counted only and never sent.
    pub fn document_prefix(&self) -> &str {
        &self.policy.document_server_prefix
    }

    /// The server-added query prefix, counted only and never sent.
    pub fn query_prefix(&self) -> &str {
        &self.policy.query_server_prefix
    }

    /// Exact token count of one server prefix plus the input.
    ///
    /// The two parts are encoded together, so a token that spans their
    /// boundary is counted once instead of being estimated by addition.
    pub fn count_input(&self, server_prefix: &str, input: &str) -> usize {
        let text = format!("{server_prefix}{input}");
        self.tokenizer
            .count_with_special(&text, self.policy.add_special_tokens)
    }

    /// Refuses one input whose exact counted envelope exceeds the cap.
    pub fn ensure_input_fits(
        &self,
        role: &'static str,
        server_prefix: &str,
        input: &str,
    ) -> Result<(), super::ChunkingError> {
        let Some(max_input_tokens) = self.max_input_tokens() else {
            return Ok(());
        };
        let count = self.count_input(server_prefix, input);
        if count > max_input_tokens {
            return Err(super::ChunkingError::InputEnvelopeExceeded {
                role,
                count,
                max_input_tokens,
                prefix_chars: server_prefix.chars().count(),
            });
        }
        Ok(())
    }
}

impl TokenizerCatalog {
    /// Resolves one embedding input policy against the verified artifacts.
    pub fn resolve_input_policy(
        &self,
        policy: &context69_contracts::settings::EmbeddingInputPolicy,
    ) -> Result<ResolvedEmbeddingInputPolicy, super::ChunkingError> {
        policy
            .validate()
            .map_err(|reason| super::ChunkingError::InvalidInputPolicy { reason })?;
        let tokenizer = match policy.tokenizer {
            context69_contracts::settings::EmbeddingTokenizer::Cl100kBase => {
                super::chunking_token::ResolvedTokenizer::from_static(
                    super::chunking_token::TokenizerKind::Cl100kBase,
                )
            }
            context69_contracts::settings::EmbeddingTokenizer::O200kBase => {
                super::chunking_token::ResolvedTokenizer::from_static(
                    super::chunking_token::TokenizerKind::O200kBase,
                )
            }
            context69_contracts::settings::EmbeddingTokenizer::HfLocal => {
                let sha256 = policy
                    .tokenizer_sha256
                    .clone()
                    .expect("validated hf_local policy carries a hash");
                let tokenizer = self.require(&sha256).map_err(|error| {
                    super::ChunkingError::TokenizerUnavailable {
                        sha256: sha256.clone(),
                        reason: error.to_string(),
                    }
                })?;
                super::chunking_token::ResolvedTokenizer::from_hf(sha256, tokenizer)
            }
        };
        Ok(ResolvedEmbeddingInputPolicy {
            policy: policy.clone(),
            tokenizer,
        })
    }

    /// Resolves one policy against the installed catalog.
    pub fn resolve_installed_input_policy(
        policy: &context69_contracts::settings::EmbeddingInputPolicy,
    ) -> Result<ResolvedEmbeddingInputPolicy, super::ChunkingError> {
        TokenizerCatalog::installed_or_empty().resolve_input_policy(policy)
    }
}
