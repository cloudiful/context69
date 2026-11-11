//! Pure tokenizer preset descriptor units.

use super::preset_descriptors;

#[test]
fn the_descriptors_report_the_pinned_license_and_a_process_local_readiness() {
    let descriptors = preset_descriptors();
    let gemma = descriptors
        .iter()
        .find(|descriptor| descriptor.id == crate::tokenizer_presets::EMBEDDINGGEMMA2_R_F9972AC)
        .expect("the EmbeddingGemma 2 preset is described");
    assert_eq!(gemma.label, "EmbeddingGemma 2");
    assert_eq!(gemma.license, "apache-2.0");
    assert_eq!(
        gemma.license_url,
        "https://www.apache.org/licenses/LICENSE-2.0"
    );
    assert_eq!(
        gemma.sha256,
        "4d777ef5bdc1aa36227abdfb77c3e49e7b9c892d16e1b6bda41c393504828be4"
    );
    assert_eq!(gemma.size_bytes, 32_170_510);
    assert_eq!(gemma.source_repo, "google/embeddinggemma-2");
    assert_eq!(gemma.revision, "f9972ac22e696a105c482202646979e33e4b5a1e");
    assert!(
        descriptors
            .iter()
            .all(|descriptor| descriptor.id != "jina_v5_small_retrieval_r6856e76"),
        "the retired preset is not described"
    );
}
