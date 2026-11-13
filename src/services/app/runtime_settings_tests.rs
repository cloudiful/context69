//! Tests for the runtime settings service seam (issue #681 work unit 4A3b-3).
//!
//! Pure normalization and key-resolution cases; no database or external
//! service is contacted.

use super::{
    RuntimeSettingsMode, apply_runtime_settings, normalize_legacy_qdrant_url,
    qdrant_grpc_url_from_rest_port,
};
use crate::{
    chunking::ChunkBudgetMode,
    chunking::chunking_token::TokenizerRef,
    db::{
        StoredRuntimeChunkingSettings, StoredRuntimeEmbeddingSettings,
        StoredRuntimeFileLibrarySettings, StoredRuntimeQdrantSettings, StoredRuntimeS3Settings,
        StoredRuntimeSchedulerSettings, StoredRuntimeSettings,
    },
};

fn stored_runtime_settings() -> StoredRuntimeSettings {
    StoredRuntimeSettings {
        qdrant: StoredRuntimeQdrantSettings {
            url: "http://qdrant:6334".to_string(),
            collection_name: "context69".to_string(),
            recreate_on_dimension_mismatch: false,
        },
        embedding: StoredRuntimeEmbeddingSettings {
            base_url: "https://embedding.invalid/v1".to_string(),
            api_key: Some("resolved-embedding-key".to_string()),
            model: "text-embedding-3-large".to_string(),
            query_model: None,
            dimensions: 3072,
            timeout_secs: 30,
        },
        scheduler: StoredRuntimeSchedulerSettings {
            interval_secs: 300,
            run_on_start: false,
            max_concurrency: 1,
            job_id: "context69-sync".to_string(),
            valkey_url: None,
        },
        chunking: StoredRuntimeChunkingSettings {
            max_chars: 1200,
            overlap_chars: 200,
            unit: ChunkBudgetMode::Chars,
            tokenizer: TokenizerRef::Cl100kBase,
            max_tokens: 1024,
            overlap_tokens: 128,
        },
        file_library: StoredRuntimeFileLibrarySettings {
            storage_root: "/tmp/library".to_string(),
            max_upload_size_mb: 128,
            trusted_proxy_enabled: false,
            registry_max_ids_per_file: Some(16384),
            registry_max_ids_per_run: Some(8192),
            s3: Some(StoredRuntimeS3Settings {
                endpoint: "https://objects.invalid".to_string(),
                region: "internal".to_string(),
                bucket: "library".to_string(),
                prefix: "staging".to_string(),
                path_style: true,
                access_key: "AKIA-EXAMPLE".to_string(),
                secret_key: Some("resolved-s3-secret".to_string()),
            }),
        },
    }
}

#[test]
fn upgrades_qdrant_rest_port_to_grpc_port() {
    assert_eq!(
        qdrant_grpc_url_from_rest_port("http://qdrant:6333").as_deref(),
        Some("http://qdrant:6334")
    );
    assert_eq!(
        qdrant_grpc_url_from_rest_port("http://qdrant:6333/").as_deref(),
        Some("http://qdrant:6334")
    );
}

#[test]
fn recovery_normalization_never_offers_the_legacy_rewrite_for_persistence() {
    let mut stored = stored_runtime_settings();
    stored.qdrant.url = "http://qdrant:6333".to_string();

    let (in_memory, rewrite) =
        normalize_legacy_qdrant_url(stored.clone(), RuntimeSettingsMode::Readonly);
    assert_eq!(in_memory.qdrant.url, "http://qdrant:6334");
    assert!(
        rewrite.is_none(),
        "the recovery read must not produce a row to save"
    );
    assert_eq!(
        stored.qdrant.url, "http://qdrant:6333",
        "the stored row still holds the legacy REST URL after a recovery read"
    );

    let (in_memory, rewrite) = normalize_legacy_qdrant_url(stored, RuntimeSettingsMode::Persist);
    assert_eq!(in_memory.qdrant.url, "http://qdrant:6334");
    assert_eq!(
        rewrite
            .expect("normal boot persists the rewrite")
            .qdrant
            .url,
        "http://qdrant:6334"
    );
}

#[test]
fn a_grpc_runtime_url_is_left_alone_in_both_modes() {
    let stored = stored_runtime_settings();
    for mode in [RuntimeSettingsMode::Readonly, RuntimeSettingsMode::Persist] {
        let (in_memory, rewrite) = normalize_legacy_qdrant_url(stored.clone(), mode);
        assert_eq!(in_memory.qdrant.url, "http://qdrant:6334");
        assert!(rewrite.is_none());
    }
}

#[test]
fn keeps_qdrant_grpc_port_unchanged() {
    assert_eq!(qdrant_grpc_url_from_rest_port("http://qdrant:6334"), None);
}

#[test]
fn the_resolved_keys_reach_object_storage_and_embedding_at_startup() {
    let mut config = crate::config::Config::default();
    apply_runtime_settings(&mut config, &stored_runtime_settings());

    // Object storage is built from this config, so the resolved S3 secret
    // key and the non-secret access key both have to arrive unchanged.
    let s3 = config
        .file_library
        .s3
        .as_ref()
        .expect("configured s3 storage is applied to the effective config");
    assert_eq!(s3.secret_key, "resolved-s3-secret");
    assert_eq!(s3.access_key, "AKIA-EXAMPLE");
    assert_eq!(s3.bucket, "library");
    assert!(s3.path_style);
    assert_eq!(
        config.embedding.api_key.as_deref(),
        Some("resolved-embedding-key")
    );
}
