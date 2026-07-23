use anyhow::{Context, Result, anyhow};
use qdrant_client::{
    Qdrant,
    qdrant::{
        CreateCollectionBuilder, CreateFieldIndexCollectionBuilder, Distance, FieldType,
        VectorParamsBuilder,
    },
};
use tracing::warn;

use super::{QdrantIndex, collection_vector_size};
use crate::config::QdrantConfig;

impl QdrantIndex {
    pub async fn ensure_metadata_field_index(&self, path: &str, data_type: &str) -> Result<()> {
        let field_type = match data_type {
            "keyword" => FieldType::Keyword,
            "datetime" => FieldType::Datetime,
            "integer" => FieldType::Integer,
            "boolean" => FieldType::Bool,
            "float" => FieldType::Float,
            value => return Err(anyhow!("unsupported qdrant metadata type {value}")),
        };
        self.client
            .create_field_index(
                CreateFieldIndexCollectionBuilder::new(
                    self.collection_name.clone(),
                    metadata_payload_key(path),
                    field_type,
                )
                .wait(true),
            )
            .await?;
        Ok(())
    }

    pub async fn delete_metadata_field_index(&self, path: &str) -> Result<()> {
        self.client
            .delete_field_index(
                qdrant_client::qdrant::DeleteFieldIndexCollectionBuilder::new(
                    self.collection_name.clone(),
                    metadata_payload_key(path),
                )
                .wait(true),
            )
            .await?;
        Ok(())
    }

    pub async fn connect(config: &QdrantConfig, dimensions: usize) -> Result<(Self, bool)> {
        let client = Qdrant::from_url(&config.url).build()?;
        let index = Self {
            client,
            collection_name: config.collection_name.clone(),
            dimensions,
        };
        let recreated = index
            .ensure_collection(dimensions, config.recreate_on_dimension_mismatch)
            .await
            .with_context(|| {
                format!(
                    "failed to initialize qdrant collection '{}' at {}; qdrant-client uses the gRPC endpoint, usually port 6334, not the REST endpoint on 6333",
                    config.collection_name, config.url
                )
            })?;
        Ok((index, recreated))
    }

    async fn ensure_collection(
        &self,
        dimensions: usize,
        recreate_on_dimension_mismatch: bool,
    ) -> Result<bool> {
        if self.client.collection_exists(&self.collection_name).await? {
            let collection = self
                .client
                .collection_info(&self.collection_name)
                .await?
                .result
                .context("missing qdrant collection info")?;
            let actual_dimensions = collection_vector_size(&collection)
                .context("missing qdrant vector size in collection config")?;
            if actual_dimensions != dimensions {
                if recreate_on_dimension_mismatch {
                    warn!(
                        collection_name = self.collection_name,
                        expected_dimensions = dimensions,
                        actual_dimensions,
                        "qdrant collection dimension mismatch detected, recreating collection"
                    );
                    self.recreate_collection().await?;
                    return Ok(true);
                }
                return Err(anyhow!(
                    "qdrant collection {} dimension mismatch: expected {}, found {}",
                    self.collection_name,
                    dimensions,
                    actual_dimensions
                ));
            }
            return Ok(false);
        }

        self.create_collection(dimensions).await?;
        Ok(true)
    }

    pub async fn recreate_collection(&self) -> Result<()> {
        if self.client.collection_exists(&self.collection_name).await? {
            self.client.delete_collection(&self.collection_name).await?;
        }
        self.create_collection(self.dimensions).await
    }

    async fn create_collection(&self, dimensions: usize) -> Result<()> {
        self.client
            .create_collection(
                CreateCollectionBuilder::new(&self.collection_name).vectors_config(
                    VectorParamsBuilder::new(dimensions as u64, Distance::Cosine),
                ),
            )
            .await?;

        for (field, field_type) in [
            ("group_key", FieldType::Keyword),
            ("group_path", FieldType::Keyword),
            ("group_id", FieldType::Integer),
            ("visibility", FieldType::Keyword),
            ("source_key", FieldType::Keyword),
            ("document_id", FieldType::Integer),
            ("published_ts", FieldType::Integer),
        ] {
            self.client
                .create_field_index(
                    CreateFieldIndexCollectionBuilder::new(
                        self.collection_name.clone(),
                        field,
                        field_type,
                    )
                    .wait(true),
                )
                .await
                .ok();
        }
        Ok(())
    }
}

pub(crate) fn metadata_payload_key(path: &str) -> String {
    format!("metadata_index.{path}")
}
