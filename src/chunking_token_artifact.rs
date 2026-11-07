//! Verified, operator-provisioned local Hugging Face tokenizer artifacts.
//!
//! The mounted catalog is one immutable map of standard `tokenizer.json` files
//! named by the lowercase SHA256 of their own bytes:
//! `<artifact-dir>/<64-hex>.json`. A mounted file is verified when the catalog
//! is built, so adding one requires a restart. A known preset fills its own
//! once-cell instead: immediately after an explicit provisioning, or from a
//! verified cache file at startup. A filled cell is never replaced.

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::{Arc, OnceLock},
};

use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};

pub use context69_contracts::settings::TokenizerArtifactDescriptor;

use crate::tokenizer_presets::{PRESETS, TokenizerPreset};

/// The largest tokenizer file this deployment accepts, mounted or retrieved.
///
/// The catalog refuses a mounted artifact above it, and a retrieval refuses a
/// preset above it before any request is built.
pub const TOKENIZER_ARTIFACT_MAX_BYTES: u64 = 32 * 1024 * 1024;

/// The process-wide catalog installed once at startup.
static INSTALLED: OnceLock<Arc<TokenizerCatalog>> = OnceLock::new();

/// The preset engine cells of this process.
///
/// The cells are shared by every catalog that uses the default slots, so a
/// preset activated after startup is visible to the runtime that already
/// resolved against the catalog. A test builds its own slots instead of
/// touching these.
static GLOBAL_PRESET_SLOTS: OnceLock<Arc<PresetSlots>> = OnceLock::new();

/// The engine cell of one known preset.
///
/// The cell is filled at most once: the engine a hash resolves to stays the
/// same one for the lifetime of the process.
#[derive(Debug)]
pub struct PresetSlot {
    engine: OnceLock<Arc<tokenizers::Tokenizer>>,
    /// Serializes the preparation of this preset within this process.
    lock: tokio::sync::Mutex<()>,
}

impl PresetSlot {
    fn new() -> Self {
        Self {
            engine: OnceLock::new(),
            lock: tokio::sync::Mutex::new(()),
        }
    }

    /// A cell for a preset a test authored, kept out of the process cells.
    #[cfg(test)]
    pub(crate) fn for_test() -> Self {
        Self::new()
    }

    /// The verified engine of this preset, when it is prepared.
    pub fn engine(&self) -> Option<Arc<tokenizers::Tokenizer>> {
        self.engine.get().cloned()
    }

    /// Fills the cell with a verified engine.
    ///
    /// A cell that is already filled keeps its engine: the hash resolves to the
    /// same immutable engine for the lifetime of the process.
    pub fn activate(&self, engine: Arc<tokenizers::Tokenizer>) -> bool {
        self.engine.set(engine).is_ok()
    }

    /// The preparation lock of this preset.
    pub fn lock(&self) -> &tokio::sync::Mutex<()> {
        &self.lock
    }
}

/// The engine cells of the known presets, one per known preset.
#[derive(Debug)]
pub struct PresetSlots {
    slots: Vec<PresetSlot>,
}

impl PresetSlots {
    /// The cells of every known preset.
    pub fn new() -> Self {
        Self {
            slots: PRESETS.iter().map(|_| PresetSlot::new()).collect(),
        }
    }

    /// The cell of one known preset.
    pub fn slot(&self, preset_id: &str) -> Option<&PresetSlot> {
        let index = PRESETS.iter().position(|preset| preset.id == preset_id)?;
        self.slots.get(index)
    }

    /// The cell of the known preset with this hash.
    pub fn slot_for_sha256(&self, sha256: &str) -> Option<&PresetSlot> {
        let index = PRESETS.iter().position(|preset| preset.sha256 == sha256)?;
        self.slots.get(index)
    }
}

impl Default for PresetSlots {
    fn default() -> Self {
        Self::new()
    }
}

/// The preset cells of this process.
pub fn global_preset_slots() -> Arc<PresetSlots> {
    GLOBAL_PRESET_SLOTS
        .get_or_init(|| Arc::new(PresetSlots::new()))
        .clone()
}

/// The immutable set of verified local tokenizer artifacts.
#[derive(Debug)]
pub struct TokenizerCatalog {
    entries: BTreeMap<String, Arc<tokenizers::Tokenizer>>,
    descriptors: Vec<TokenizerArtifactDescriptor>,
    /// The engine cells of the known presets.
    ///
    /// They live with the catalog so an activated preset is visible to every
    /// consumer that already holds this catalog.
    presets: Arc<PresetSlots>,
    /// The writable directory the prepared presets are cached in.
    preset_cache_dir: Option<PathBuf>,
}

impl TokenizerCatalog {
    /// An empty catalog: legacy tokenizers keep working without any artifact.
    pub fn empty() -> Arc<Self> {
        Self::empty_with_preset_slots(global_preset_slots())
    }

    /// An empty catalog with its own preset cells.
    ///
    /// It keeps no writable preset cache, so it describes a catalog that was
    /// not built from a deployment configuration.
    pub fn empty_with_preset_slots(presets: Arc<PresetSlots>) -> Arc<Self> {
        Self::empty_with_preset_cache(presets, None)
    }

    /// An empty catalog with its own preset cells and writable preset cache.
    ///
    /// A missing operator artifact directory yields this catalog instead of an
    /// error, and the configured cache directory stays part of it: a fresh
    /// installation must still be able to prepare a pinned preset.
    pub fn empty_with_preset_cache(
        presets: Arc<PresetSlots>,
        preset_cache_dir: Option<PathBuf>,
    ) -> Arc<Self> {
        Arc::new(Self {
            entries: BTreeMap::new(),
            descriptors: Vec::new(),
            presets,
            preset_cache_dir,
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

    /// Loads and installs the process catalog with the known presets.
    ///
    /// The operator directory is scanned exactly as before, and the writable
    /// cache directory is scanned for the known presets only: a file named by
    /// the hash of a known preset is verified and parsed, and its engine fills
    /// that preset's cell. A preset without a verified cached file stays
    /// unprepared, and nothing is fetched: a restart after a preparation
    /// activates the preset offline.
    pub fn ensure_installed_with_presets(
        directory: &Path,
        preset_cache_dir: &Path,
    ) -> Result<Arc<Self>> {
        if let Some(installed) = INSTALLED.get() {
            return Ok(Arc::clone(installed));
        }
        let slots = global_preset_slots();
        let catalog = Self::load_with_presets(
            directory,
            Arc::clone(&slots),
            Some(preset_cache_dir.to_path_buf()),
        )?;
        activate_cached_presets(&slots, preset_cache_dir)?;
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
        Self::load_with_presets(directory, global_preset_slots(), None)
    }

    /// Scans one operator directory into its own preset cells.
    ///
    /// A test uses this to keep its activations out of the process-wide cells.
    pub fn load_with_presets(
        directory: &Path,
        presets: Arc<PresetSlots>,
        preset_cache_dir: Option<PathBuf>,
    ) -> Result<Arc<Self>> {
        if !directory.exists() {
            // A missing operator directory is the legacy default: no mounted
            // artifact is available, and the configured preset cache stays
            // usable so a fresh installation can still prepare a preset.
            return Ok(Self::empty_with_preset_cache(presets, preset_cache_dir));
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
            let tokenizer = parse_verified_tokenizer(&bytes).with_context(|| {
                format!("the tokenizer artifact {name} is not a readable tokenizer.json")
            })?;
            entries.insert(stem.to_string(), tokenizer);
            descriptors.push(TokenizerArtifactDescriptor {
                sha256: stem.to_string(),
                size_bytes: bytes.len() as u64,
            });
        }
        descriptors.sort_by(|left, right| left.sha256.cmp(&right.sha256));
        Ok(Arc::new(Self {
            entries,
            descriptors,
            presets,
            preset_cache_dir,
        }))
    }

    /// The verified engine for one hash.
    ///
    /// The mounted artifacts are consulted first, then the known preset cells:
    /// a prepared preset resolves by its pinned hash exactly like an
    /// operator-mounted artifact does.
    pub fn get(&self, sha256: &str) -> Option<Arc<tokenizers::Tokenizer>> {
        if let Some(tokenizer) = self.entries.get(sha256) {
            return Some(Arc::clone(tokenizer));
        }
        self.presets
            .slot_for_sha256(sha256)
            .and_then(PresetSlot::engine)
    }

    /// The verified engine for one hash, or an actionable refusal.
    pub fn require(&self, sha256: &str) -> Result<Arc<tokenizers::Tokenizer>> {
        self.get(sha256).ok_or_else(|| {
            anyhow::anyhow!(
                "the local tokenizer artifact {sha256} is not available in the verified catalog; prepare the preset or provision {sha256}.json"
            )
        })
    }

    /// The public descriptors of every verified artifact.
    pub fn descriptors(&self) -> &[TokenizerArtifactDescriptor] {
        &self.descriptors
    }

    /// The engine cells of the known presets.
    pub fn preset_slots(&self) -> &Arc<PresetSlots> {
        &self.presets
    }

    /// The writable directory the prepared presets are cached in.
    pub fn preset_cache_dir(&self) -> Option<&Path> {
        self.preset_cache_dir.as_deref()
    }
}

/// The installed verified engine for one hash.
pub fn require_installed(sha256: &str) -> Result<Arc<tokenizers::Tokenizer>> {
    TokenizerCatalog::installed_or_empty().require(sha256)
}

/// Fills the known preset cells from the verified files in the cache directory.
///
/// Only the known presets are considered, and only a file named by the pinned
/// hash of one of them is read. A file that does not verify leaves its preset
/// unprepared instead of failing the boot.
pub fn activate_cached_presets(slots: &PresetSlots, cache_dir: &Path) -> Result<()> {
    for preset in PRESETS {
        let Some(slot) = slots.slot(preset.id) else {
            continue;
        };
        if slot.engine().is_some() {
            continue;
        }
        let path = cache_dir.join(format!("{}.json", preset.sha256));
        if !path.exists() {
            continue;
        }
        if let Err(error) = activate_cached_preset(slot, &path, preset) {
            tracing::warn!(
                preset = preset.id,
                error = %format!("{error:#}"),
                "the cached tokenizer preset could not be verified; it stays unprepared"
            );
        }
    }
    Ok(())
}

/// Activates one cached preset file into its own cell.
///
/// The startup scan and a preparation share this path, so a file that is
/// written by one process and read by the next is verified exactly the same
/// way. A file that fails any check leaves the cell empty instead of failing
/// the boot.
pub fn activate_cached_preset(
    slot: &PresetSlot,
    path: &Path,
    preset: &TokenizerPreset,
) -> Result<()> {
    if slot.engine().is_some() {
        return Ok(());
    }
    let engine = verified_preset_engine(path, preset)?;
    slot.activate(engine);
    Ok(())
}

/// The verified engine of one cached preset file, without activating it.
///
/// Padding and truncation are disabled before the engine is used. The caller
/// fills the preset cell only after the entry is durable, so a failed
/// durability step never leaves a ready preset behind.
pub(crate) fn verified_preset_engine(
    path: &Path,
    preset: &TokenizerPreset,
) -> Result<Arc<tokenizers::Tokenizer>> {
    let metadata = fs::symlink_metadata(path).with_context(|| {
        format!(
            "read the cached tokenizer preset metadata {}",
            path.display()
        )
    })?;
    if !metadata.file_type().is_file() {
        bail!(
            "the cached tokenizer preset {} is not a regular file",
            path.display()
        );
    }
    if metadata.len() != preset.size_bytes {
        bail!(
            "the cached tokenizer preset {} does not have the expected size of {} bytes",
            path.display(),
            preset.size_bytes
        );
    }
    let bytes = fs::read(path)
        .with_context(|| format!("read the cached tokenizer preset {}", path.display()))?;
    let actual = hash_bytes(&bytes);
    if actual != preset.sha256 {
        bail!(
            "the cached tokenizer preset {} does not match its pinned hash {actual}",
            path.display()
        );
    }
    parse_verified_tokenizer(&bytes)
}

/// The verified engine of one tokenizer file.
///
/// Padding and truncation are disabled before the engine is used, so a count
/// is the candidate's own tokens.
pub fn parse_verified_tokenizer(bytes: &[u8]) -> Result<Arc<tokenizers::Tokenizer>> {
    let mut tokenizer = tokenizers::Tokenizer::from_bytes(bytes)
        .map_err(|_| anyhow::anyhow!("the tokenizer file is not a readable tokenizer.json"))?;
    tokenizer
        .with_truncation(None)
        .map_err(|_| anyhow::anyhow!("the tokenizer file disables truncation"))?;
    tokenizer.with_padding(None);
    Ok(Arc::new(tokenizer))
}

/// A standard tokenizer file a test authored.
///
/// It is a byte-level BPE with one token per byte, so a retrieval or a cache
/// entry can be exercised without the pinned preset file. It is deliberately
/// not the pinned file and never carries the official hash.
#[cfg(test)]
pub(crate) fn test_tokenizer_json(truncate: bool) -> Vec<u8> {
    let byte_char = |byte: u8| -> char {
        let value = byte as u32;
        if (33..=126).contains(&value)
            || (161..=172).contains(&value)
            || (174..=255).contains(&value)
        {
            char::from_u32(value).expect("a printable byte maps to itself")
        } else {
            char::from_u32(256 + value).expect("a control byte maps into the private range")
        }
    };
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
    use std::path::Path;

    use crate::chunking::chunking_token::TokenizerRef;
    use crate::chunking::{ChunkBudget, ChunkingRuntime};
    use crate::tokenizer_artifact_download::test_support::test_directory;

    #[test]
    fn review_missing_operator_directory_retains_preset_cache_configuration() {
        let directory = test_directory("chunking-missing-operator");
        let missing = directory.join("missing");
        let cache = directory.join("cache");
        let catalog = TokenizerCatalog::load_with_presets(
            &missing,
            Arc::new(PresetSlots::new()),
            Some(cache.clone()),
        )
        .expect("load");
        let actual = catalog.preset_cache_dir().map(Path::to_path_buf);
        std::fs::remove_dir_all(&directory).expect("cleanup");
        assert_eq!(actual, Some(cache));
    }

    fn write_artifact(directory: &Path, name: &str, bytes: &[u8]) -> String {
        std::fs::write(directory.join(format!("{name}.json")), bytes).expect("write the artifact");
        name.to_string()
    }

    #[test]
    fn a_verified_artifact_loads_and_its_hash_resolves() {
        let directory = test_directory("chunking-verified-artifact");
        let bytes = test_tokenizer_json(false);
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
        let directory = test_directory("chunking-truncation");
        let bytes = test_tokenizer_json(true);
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
        let directory = test_directory("chunking-name-mismatch");
        let bytes = test_tokenizer_json(false);
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

        let directory = test_directory("chunking-non-json");
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
        let directory = test_directory("chunking-pinned-cut");
        let bytes = test_tokenizer_json(false);
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

    #[test]
    fn a_preset_cell_keeps_the_engine_it_was_filled_with() {
        let slot = PresetSlot::for_test();
        let first = parse_verified_tokenizer(&test_tokenizer_json(false))
            .expect("parse the first tokenizer");
        assert!(
            slot.activate(Arc::clone(&first)),
            "the empty cell accepts an engine"
        );

        let second = parse_verified_tokenizer(&test_tokenizer_json(true))
            .expect("parse the second tokenizer");
        assert!(
            !slot.activate(second),
            "a filled cell never replaces its engine"
        );
        assert!(
            Arc::ptr_eq(&slot.engine().expect("the engine stays active"), &first),
            "the hash resolves to the same immutable engine"
        );
    }

    #[test]
    fn a_cached_preset_that_does_not_verify_stays_unprepared() {
        let directory = test_directory("chunking-cached-preset");
        let bytes = test_tokenizer_json(false);
        let preset = crate::tokenizer_artifact_download::test_support::pinned_test_preset(&bytes);
        let mut other = bytes.clone();
        let last = other.len() - 2;
        other[last] = if other[last] == b' ' { b'!' } else { b' ' };
        let path = directory.join(format!("{}.json", preset.sha256));
        std::fs::write(&path, &other).expect("write the mismatched cache file");
        let slot = PresetSlot::for_test();

        activate_cached_preset(&slot, &path, &preset)
            .expect_err("a mismatched cache file is refused");

        assert!(slot.engine().is_none(), "nothing is activated");
        std::fs::remove_dir_all(&directory).expect("remove the cache directory");
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
