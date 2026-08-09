use anyhow::{Context, Result, anyhow};
use async_trait::async_trait;
use context69_extraction::{ExtractionPublication, ExtractionPublisher};
use serde_json::json;
use tracing::info;

use crate::{
    contracts::Visibility,
    db::Database,
    domain::{AccessScope, ChunkPayload},
    qdrant_index::QdrantIndex,
};

const EXTRACTION_METADATA_KEY: &str = "extraction";

#[derive(Clone)]
pub struct ExtractionPublisherAdapter {
    db: Database,
    index: Option<QdrantIndex>,
}

impl ExtractionPublisherAdapter {
    pub fn new(db: Database, index: Option<QdrantIndex>) -> Self {
        Self { db, index }
    }
}

#[async_trait]
impl ExtractionPublisher for ExtractionPublisherAdapter {
    async fn publish(&self, publication: &ExtractionPublication<'_>) -> Result<()> {
        let scope = AccessScope {
            user_id: None,
            include_public: true,
            private_group_ids: vec![publication.group_id],
            group_path: None,
            scoped_group_id: None,
        };
        let document = self
            .db
            .get_document(publication.document_id, &scope)
            .await?
            .context("extraction document not found")?;
        let merged_metadata = merge_extraction_metadata(
            &document.metadata_json,
            publication.template_key,
            publication.result_json,
        )?;
        let visibility = publication
            .visibility
            .parse::<Visibility>()
            .context("invalid extraction document visibility")?;
        let payloads = document
            .chunks
            .iter()
            .map(|chunk| ChunkPayload {
                chunk_id: chunk.chunk_id,
                document_id: publication.document_id,
                group_id: publication.group_id,
                group_key: publication.group_key.to_string(),
                group_path: publication.group_path.to_string(),
                visibility,
                source_key: publication.source_key.to_string(),
                external_id: publication.external_id.to_string(),
                title: document.title.clone(),
                summary: document.summary.clone(),
                source_uri: publication.source_uri.to_string(),
                published_at: publication.published_at,
                updated_at_source: publication.updated_at,
                record_hash: document.record_hash.clone(),
                chunk_index: chunk.chunk_index,
                chunk_text: chunk.text.clone(),
                metadata_json: merged_metadata.clone(),
                content_locale: "original".to_string(),
                source_locale: None,
                translation_provider: None,
            })
            .collect::<Vec<_>>();

        let Some(first) = payloads.first() else {
            return Err(anyhow!("extraction document has no chunks"));
        };
        self.db
            .update_library_document_business_fields(publication.document_id, first)
            .await?;
        if let Some(index) = &self.index {
            index.update_chunk_payloads(&payloads).await?;
        }
        info!(
            document_id = publication.document_id,
            template_key = publication.template_key,
            batch_size = payloads.len(),
            "extraction result published to document metadata"
        );
        Ok(())
    }
}

fn merge_extraction_metadata(
    current: &serde_json::Value,
    template_key: &str,
    result: &serde_json::Value,
) -> Result<serde_json::Value> {
    let mut merged = current.clone();
    let obj = merged
        .as_object_mut()
        .ok_or_else(|| anyhow!("document metadata_json must be an object"))?;
    let extractions = obj
        .entry(EXTRACTION_METADATA_KEY.to_string())
        .or_insert_with(|| json!({}));
    extractions
        .as_object_mut()
        .ok_or_else(|| anyhow!("metadata extraction namespace must be an object"))?
        .insert(template_key.to_string(), result.clone());
    Ok(merged)
}

#[cfg(test)]
mod tests {
    use super::merge_extraction_metadata;
    use serde_json::json;

    #[test]
    fn merges_extraction_result_into_metadata_namespace() {
        let current = json!({"ts_code": "600519.SH"});
        let merged =
            merge_extraction_metadata(&current, "stock.news.v1", &json!({"direction": "positive"}))
                .unwrap();
        assert_eq!(
            merged["extraction"]["stock.news.v1"]["direction"],
            "positive"
        );
        assert_eq!(merged["ts_code"], "600519.SH");
    }

    #[test]
    fn preserves_existing_extraction_namespace() {
        let current = json!({"extraction": {"other.v1": {"a": 1}}});
        let merged =
            merge_extraction_metadata(&current, "stock.news.v1", &json!({"b": 2})).unwrap();
        assert_eq!(merged["extraction"]["other.v1"]["a"], 1);
        assert_eq!(merged["extraction"]["stock.news.v1"]["b"], 2);
    }
}
