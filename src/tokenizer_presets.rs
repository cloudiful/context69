//! The known tokenizer presets this deployment can prepare.
//!
//! One table is the whole authority: the identifier, the pinned revision, the
//! expected byte size, the expected SHA256, and the license the upstream
//! project publishes. A preset never carries a URL a client controls — the
//! downloader builds the retrieval URL from these constants.
//!
//! A preset describes one `tokenizer.json` file. It never describes weights,
//! a model configuration, or a vocabulary beyond that file. The built-in
//! EmbeddingGemma 2 preset is baked into the runtime images at
//! `<artifact-dir>/<sha256>.json`; the same constants also let a bare
//! installation retrieve that exact file into its writable preset cache.

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

/// The identifier of the bundled EmbeddingGemma 2 tokenizer preset.
pub const EMBEDDINGGEMMA2_R_F9972AC: &str = "embeddinggemma2_r_f9972ac";

/// Every known preset.
///
/// A new revision or a new hash is a new preset identifier: an identifier
/// never changes meaning, so a pinned task keeps the file it was pinned to.
pub const PRESETS: &[TokenizerPreset] = &[TokenizerPreset {
    id: EMBEDDINGGEMMA2_R_F9972AC,
    label: "EmbeddingGemma 2",
    repo: "google/embeddinggemma-2",
    revision: "f9972ac22e696a105c482202646979e33e4b5a1e",
    file: "tokenizer.json",
    sha256: "4d777ef5bdc1aa36227abdfb77c3e49e7b9c892d16e1b6bda41c393504828be4",
    size_bytes: 32_170_510,
    license: "apache-2.0",
    license_url: "https://www.apache.org/licenses/LICENSE-2.0",
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
        let preset = preset_by_id(EMBEDDINGGEMMA2_R_F9972AC).expect("the preset");
        assert_eq!(
            preset.download_url(),
            "https://huggingface.co/google/embeddinggemma-2/resolve/\
             f9972ac22e696a105c482202646979e33e4b5a1e/tokenizer.json"
        );
        assert_eq!(
            preset.source_url(),
            "https://huggingface.co/google/embeddinggemma-2/blob/\
             f9972ac22e696a105c482202646979e33e4b5a1e"
        );
        assert_eq!(preset.file, "tokenizer.json");
        assert_eq!(preset.size_bytes, 32_170_510);
        assert_eq!(
            preset.sha256,
            "4d777ef5bdc1aa36227abdfb77c3e49e7b9c892d16e1b6bda41c393504828be4"
        );
        assert_eq!(preset.license, "apache-2.0");
        assert_eq!(
            preset.license_url,
            "https://www.apache.org/licenses/LICENSE-2.0"
        );
    }

    #[test]
    fn an_unknown_identifier_names_no_preset() {
        assert!(preset_by_id("jina_v5_small_retrieval").is_none());
        assert!(preset_by_id("embeddinggemma2_r").is_none());
        assert!(preset_by_id("").is_none());
    }

    #[test]
    fn both_image_download_stages_carry_the_exact_pin_and_size_check() {
        let preset = preset_by_id(EMBEDDINGGEMMA2_R_F9972AC).expect("the preset");
        let size_check = format!(
            "test \"$(wc -c < /tmp/tokenizer.json)\" -eq {}",
            preset.size_bytes
        );
        let destination = format!(
            "COPY --from=tokenizer-artifact /tmp/tokenizer.json \
             /var/lib/context69/tokenizers/{}.json",
            preset.sha256
        );
        for (dockerfile, download_url) in [
            (include_str!("../Dockerfile"), preset.download_url()),
            (
                include_str!("../forgejo.Dockerfile"),
                format!(
                    "https://huggingface.cloud1ful.com/{}/resolve/{}/{}",
                    preset.repo, preset.revision, preset.file
                ),
            ),
        ] {
            let checksum = format!("ADD --checksum=sha256:{} {}", preset.sha256, download_url);
            assert!(dockerfile.contains(&checksum), "the download checksum pin");
            assert!(dockerfile.contains(&size_check), "the exact size check");
            assert!(
                dockerfile.contains(&destination),
                "the artifact destination"
            );
        }
    }
}
