use anyhow::Result;
use async_trait::async_trait;
use context69_extraction::ExtractionReadiness;
use context69_translation::TranslationReadiness;
use uuid::Uuid;

use crate::{
    library_store::LibraryStore,
    services::library::{
        LIBRARY_DEPENDENCY_PROBE_LEASE_TTL_SECS, LibraryDependency, log_dependency_transition,
        report_embedding_vector_processing_error_with_lease,
    },
};

#[derive(Clone)]
pub struct LibraryEmbeddingVectorReadiness {
    pub store: LibraryStore,
    pub configuration_fingerprint: String,
}

#[async_trait]
impl ExtractionReadiness for LibraryEmbeddingVectorReadiness {
    async fn is_ready(&self) -> Result<bool> {
        let gates = self.store.list_dependency_gates().await?;
        let canonical = LibraryDependency::Embedding.canonical_str();
        let gate = gates
            .iter()
            .find(|gate| LibraryDependency::canonical_key(&gate.dependency_key) == canonical);
        Ok(gate.is_some_and(|gate| gate.state == "closed"))
    }
}

#[async_trait]
impl TranslationReadiness for LibraryEmbeddingVectorReadiness {
    async fn is_ready(&self) -> Result<bool> {
        let gates = self.store.list_dependency_gates().await?;
        let canonical = LibraryDependency::Embedding.canonical_str();
        let gate = gates
            .iter()
            .find(|gate| LibraryDependency::canonical_key(&gate.dependency_key) == canonical);
        Ok(gate.is_some_and(|gate| gate.state == "closed"))
    }

    async fn reserve_probe(&self) -> Result<Option<Uuid>> {
        let token = Uuid::new_v4();
        let transition = self
            .store
            .reserve_dependency_probe(
                LibraryDependency::Embedding.canonical_str(),
                token,
                LIBRARY_DEPENDENCY_PROBE_LEASE_TTL_SECS,
            )
            .await?;
        if let Some(transition) = transition {
            log_dependency_transition(&transition);
            Ok(Some(token))
        } else {
            Ok(None)
        }
    }

    async fn is_ready_for_probe(&self, probe_token: Uuid) -> Result<bool> {
        let gates = self.store.list_dependency_gates().await?;
        let canonical = LibraryDependency::Embedding.canonical_str();
        let gate = gates
            .iter()
            .find(|gate| LibraryDependency::canonical_key(&gate.dependency_key) == canonical);
        Ok(gate.is_some_and(|gate| {
            gate.state == "closed"
                || (gate.state == "half_open" && gate.probe_lease_token == Some(probe_token))
        }))
    }

    async fn report_processing_error(&self, error: &str) -> Result<bool> {
        report_embedding_vector_processing_error_with_lease(
            &self.store,
            &self.configuration_fingerprint,
            Uuid::nil(),
            error,
        )
        .await
    }

    async fn report_processing_error_with_probe(
        &self,
        error: &str,
        probe_token: Option<Uuid>,
    ) -> Result<bool> {
        let handled = report_embedding_vector_processing_error_with_lease(
            &self.store,
            &self.configuration_fingerprint,
            probe_token.unwrap_or_else(Uuid::nil),
            error,
        )
        .await?;
        Ok(handled)
    }

    async fn complete_probe(&self, probe_token: Uuid) -> Result<()> {
        if let Some(transition) = self
            .store
            .release_dependency_probe(LibraryDependency::Embedding.canonical_str(), probe_token)
            .await?
        {
            log_dependency_transition(&transition);
        }
        Ok(())
    }

    async fn abandon_probe(&self, probe_token: Uuid) -> Result<()> {
        if let Some(transition) = self
            .store
            .abandon_dependency_probe(LibraryDependency::Embedding.canonical_str(), probe_token)
            .await?
        {
            log_dependency_transition(&transition);
        }
        Ok(())
    }
}
