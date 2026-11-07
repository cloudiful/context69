//! The known tokenizer presets this deployment can prepare.
//!
//! One table is the whole authority: the identifier, the pinned revision, the
//! expected byte size, the expected SHA256, and the license the upstream
//! project publishes. A preset never carries a URL a client controls — the
//! downloader builds the retrieval URL from these constants.
//!
//! A preset describes one `tokenizer.json` file. It never describes weights,
//! a model configuration, or a vocabulary beyond that file, and the product
//! neither bundles nor redistributes it: an operator initiates the retrieval
//! in their own deployment.

/// One pinned tokenizer file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenizerPreset {
    /// The stable identifier a request names.
    pub id: &'static str,
    /// The operator-facing label.
    pub label: &'static str,
    /// The upstream repository, `owner/name`.
    pub repo: &'static str,
    /// The pinned upstream revision.
    pub revision: &'static str,
    /// The file inside the repository.
    pub file: &'static str,
    /// The expected SHA256 of the file bytes.
    pub sha256: &'static str,
    /// The expected size of the file in bytes.
    pub size_bytes: u64,
    /// The license identifier the upstream project publishes.
    pub license: &'static str,
    /// The official license URL.
    pub license_url: &'static str,
}

impl TokenizerPreset {
    /// The upstream source URL of the pinned revision.
    pub fn source_url(&self) -> String {
        format!(
            "https://huggingface.co/{}/blob/{}",
            self.repo, self.revision
        )
    }

    /// The retrieval URL, built only from the pinned constants.
    pub fn download_url(&self) -> String {
        format!(
            "https://huggingface.co/{}/resolve/{}/{}",
            self.repo, self.revision, self.file
        )
    }
}

/// The identifier of the Jina embeddings v5 text small retrieval preset.
pub const JINA_V5_SMALL_RETRIEVAL_R6856E76: &str = "jina_v5_small_retrieval_r6856e76";

/// Every known preset.
///
/// A new revision or a new hash is a new preset identifier: an identifier
/// never changes meaning, so a pinned task keeps the file it was pinned to.
pub const PRESETS: &[TokenizerPreset] = &[TokenizerPreset {
    id: JINA_V5_SMALL_RETRIEVAL_R6856E76,
    label: "Jina embeddings v5 text small retrieval",
    repo: "jinaai/jina-embeddings-v5-text-small-retrieval",
    revision: "6856e76bb72982e58de0620458a4e8b3614da340",
    file: "tokenizer.json",
    sha256: "aeb13307a71acd8fe81861d94ad54ab689df773318809eed3cbe794b4492dae4",
    size_bytes: 11_422_654,
    license: "cc-by-nc-4.0",
    license_url: "https://creativecommons.org/licenses/by-nc/4.0/",
}];

/// The preset one identifier names, or `None` for an unknown identifier.
pub fn preset_by_id(id: &str) -> Option<&'static TokenizerPreset> {
    PRESETS.iter().find(|preset| preset.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_retrieval_url_is_built_from_the_pinned_constants_only() {
        let preset = preset_by_id(JINA_V5_SMALL_RETRIEVAL_R6856E76).expect("the preset");
        assert_eq!(
            preset.download_url(),
            "https://huggingface.co/jinaai/jina-embeddings-v5-text-small-retrieval/resolve/\
             6856e76bb72982e58de0620458a4e8b3614da340/tokenizer.json"
        );
        assert_eq!(
            preset.source_url(),
            "https://huggingface.co/jinaai/jina-embeddings-v5-text-small-retrieval/blob/\
             6856e76bb72982e58de0620458a4e8b3614da340"
        );
    }

    #[test]
    fn an_unknown_identifier_names_no_preset() {
        assert!(preset_by_id("jina_v5_small_retrieval").is_none());
        assert!(preset_by_id("").is_none());
    }
}
