//! Durable retirement of the document vector collection a successful rebuild
//! replaced.
//!
//! Activation records the previous active identity and the replacement in one
//! transaction, so the pending queue never describes an uncommitted switch.
//! This module owns the persistence mechanics and the pure endpoint proof:
//! which Qdrant endpoint is allowed to delete one physical collection. It never
//! resolves the current runtime settings and never logs an endpoint.

use anyhow::{Context, Result};
use sqlx::FromRow;
use uuid::Uuid;

use super::Database;
use crate::embedding::embedding_runtime::vector_identity_fingerprint;

/// Fixed drain interval before a retired collection may be deleted.
///
/// In-flight search snapshots may still read the old collection right after
/// activation, so the first eligible deletion waits one bounded grace period.
pub const RETIREMENT_GRACE_SECONDS: i32 = 300;

/// The previous active identity, read inside the activation transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveRetirementCandidate {
    pub collection_name: String,
    pub fingerprint: String,
    pub embedding_base_url: String,
    pub embedding_model: String,
    pub dimensions: usize,
    /// The endpoint saved with the identity; `None` on a legacy row.
    pub qdrant_url: Option<String>,
}

/// One physical collection that a proven endpoint may delete.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetirementTarget {
    pub endpoint: String,
    pub collection_name: String,
}

/// One claimed retirement row. The lease token fences completion and failure.
#[derive(Debug, Clone)]
pub struct ClaimedVectorCollectionRetirement {
    pub id: i64,
    pub endpoint: String,
    pub collection_name: String,
    pub attempts: i32,
    pub lease_token: Uuid,
}

#[derive(FromRow)]
struct ActiveRetirementCandidateRow {
    collection_name: String,
    fingerprint: String,
    embedding_base_url: String,
    embedding_model: String,
    dimensions: i64,
    qdrant_url: Option<String>,
}

#[derive(FromRow)]
struct ClaimedVectorCollectionRetirementRow {
    id: i64,
    endpoint: String,
    collection_name: String,
    attempts: i32,
    lease_token: Option<Uuid>,
}

/// The endpoint that provably owns the candidate collection, if any.
///
/// A nonempty saved endpoint is direct evidence and is used independently of
/// the endpoint the new attempt writes to: a replacement may activate against
/// another endpoint while the replaced collection still lives on the endpoint
/// it was saved with. An empty saved endpoint is ineligible and never falls
/// through to reconstruction.
///
/// A legacy row has no endpoint at all: only a complete fingerprint
/// reconstruction from the attempt's endpoint, the attempt's logical base
/// collection, and the old row's embedding URL, model, and dimensions proves
/// one. A mismatch is never guessed and never falls back to a current setting
/// or a physical generated name.
pub fn prove_retirement_endpoint(
    candidate: &ActiveRetirementCandidate,
    attempt_endpoint: &str,
    attempt_base_collection: &str,
) -> Option<String> {
    if let Some(recorded) = candidate.qdrant_url.as_deref() {
        let recorded = recorded.trim();
        return (!recorded.is_empty()).then(|| recorded.to_string());
    }

    let attempt_endpoint = attempt_endpoint.trim();
    if attempt_endpoint.is_empty() {
        return None;
    }
    let base_collection = attempt_base_collection.trim();
    if base_collection.is_empty() || candidate.fingerprint.is_empty() {
        return None;
    }
    let reconstructed = vector_identity_fingerprint(
        attempt_endpoint,
        base_collection,
        &candidate.embedding_base_url,
        &candidate.embedding_model,
        candidate.dimensions,
    );
    (reconstructed == candidate.fingerprint).then(|| attempt_endpoint.to_string())
}

/// The pending target for a replaced collection.
///
/// The replacement is never enqueued, an empty identity is refused, and a name
/// that is still the replacement physical collection is excluded rather than
/// deleted.
pub fn retirement_target(
    candidate: &ActiveRetirementCandidate,
    proven_endpoint: &str,
    replacement_collection: &str,
) -> Option<RetirementTarget> {
    let endpoint = proven_endpoint.trim();
    let collection_name = candidate.collection_name.trim();
    if endpoint.is_empty() || collection_name.is_empty() {
        return None;
    }
    if collection_name == replacement_collection.trim() {
        return None;
    }
    Some(RetirementTarget {
        endpoint: endpoint.to_string(),
        collection_name: collection_name.to_string(),
    })
}

impl Database {
    /// The previous active identity, read on the caller's connection.
    pub async fn read_active_vector_index_for_retirement_in(
        &self,
        connection: &mut sqlx::PgConnection,
    ) -> Result<Option<ActiveRetirementCandidate>> {
        let row = sqlx::query_file_as!(
            ActiveRetirementCandidateRow,
            "src/sql/db/vector_index_state/read_active_for_retirement.sql"
        )
        .fetch_optional(&mut *connection)
        .await
        .context("read the active vector index for retirement")?;
        row.map(|row| {
            let dimensions = usize::try_from(row.dimensions)
                .context("the active vector dimensions must be non-negative")?;
            Ok(ActiveRetirementCandidate {
                collection_name: row.collection_name,
                fingerprint: row.fingerprint,
                embedding_base_url: row.embedding_base_url,
                embedding_model: row.embedding_model,
                dimensions,
                qdrant_url: row.qdrant_url,
            })
        })
        .transpose()
    }

    /// Enqueue one proven target in the activation transaction.
    ///
    /// The unique endpoint/name key makes a repeat a no-op, and the first
    /// attempt is never due before the grace period has elapsed.
    pub async fn enqueue_vector_collection_retirement_in(
        &self,
        connection: &mut sqlx::PgConnection,
        endpoint: &str,
        collection_name: &str,
        grace_seconds: i32,
    ) -> Result<bool> {
        let result = sqlx::query_file!(
            "src/sql/db/vector_collection_retirement/enqueue.sql",
            endpoint,
            collection_name,
            grace_seconds
        )
        .execute(&mut *connection)
        .await
        .context("enqueue the retired vector collection")?;
        Ok(result.rows_affected() == 1)
    }

    /// Claim at most one due retirement row with a fenced lease.
    pub async fn claim_vector_collection_retirement(
        &self,
        lease_token: Uuid,
        lease_seconds: i32,
    ) -> Result<Option<ClaimedVectorCollectionRetirement>> {
        let row = sqlx::query_file_as!(
            ClaimedVectorCollectionRetirementRow,
            "src/sql/db/vector_collection_retirement/claim.sql",
            lease_token,
            lease_seconds
        )
        .fetch_optional(&self.pool)
        .await
        .context("claim a retired vector collection")?;
        row.map(|row| {
            let returned_lease = row.lease_token.ok_or_else(|| {
                anyhow::anyhow!("a claimed vector collection retirement has no lease")
            })?;
            Ok(ClaimedVectorCollectionRetirement {
                id: row.id,
                endpoint: row.endpoint,
                collection_name: row.collection_name,
                attempts: row.attempts,
                lease_token: returned_lease,
            })
        })
        .transpose()
    }

    /// Remove a drained row only while the exact lease still owns it.
    pub async fn complete_vector_collection_retirement(
        &self,
        id: i64,
        lease_token: Uuid,
    ) -> Result<bool> {
        let result = sqlx::query_file!(
            "src/sql/db/vector_collection_retirement/complete.sql",
            id,
            lease_token
        )
        .execute(&self.pool)
        .await
        .context("complete a retired vector collection")?;
        Ok(result.rows_affected() == 1)
    }

    /// Record one failed attempt only while the exact lease still owns it.
    pub async fn fail_vector_collection_retirement(
        &self,
        id: i64,
        lease_token: Uuid,
        retry_after_seconds: i32,
        reason: &str,
    ) -> Result<bool> {
        let result = sqlx::query_file!(
            "src/sql/db/vector_collection_retirement/fail.sql",
            id,
            lease_token,
            retry_after_seconds,
            reason
        )
        .execute(&self.pool)
        .await
        .context("record a failed vector collection retirement")?;
        Ok(result.rows_affected() == 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(collection: &str, fingerprint: &str) -> ActiveRetirementCandidate {
        ActiveRetirementCandidate {
            collection_name: collection.to_string(),
            fingerprint: fingerprint.to_string(),
            embedding_base_url: "https://embed.example/v1".to_string(),
            embedding_model: "model-a".to_string(),
            dimensions: 768,
            qdrant_url: None,
        }
    }

    #[test]
    fn a_saved_endpoint_is_used_independently_of_the_new_attempt_endpoint() {
        let mut saved = candidate("context69_chunks", "fingerprint-a");
        saved.qdrant_url = Some("https://old-qdrant.example:6334".to_string());

        assert_eq!(
            prove_retirement_endpoint(
                &saved,
                "https://new-qdrant.example:6334",
                "context69_chunks"
            ),
            Some("https://old-qdrant.example:6334".to_string()),
            "the saved endpoint owns the old collection even when the replacement writes elsewhere"
        );
        assert_eq!(
            prove_retirement_endpoint(
                &saved,
                "https://old-qdrant.example:6334",
                "context69_chunks"
            ),
            Some("https://old-qdrant.example:6334".to_string())
        );
    }

    #[test]
    fn a_legacy_endpoint_is_proven_only_by_a_complete_fingerprint_match() {
        let fingerprint = vector_identity_fingerprint(
            "https://qdrant.example:6334",
            "context69_chunks",
            "https://embed.example/v1",
            "model-a",
            768,
        );
        let legacy = candidate("context69_chunks__abc__123", &fingerprint);

        assert_eq!(
            prove_retirement_endpoint(&legacy, "https://qdrant.example:6334", "context69_chunks"),
            Some("https://qdrant.example:6334".to_string())
        );
        assert_eq!(
            prove_retirement_endpoint(&legacy, "https://other.example:6334", "context69_chunks"),
            None,
            "another endpoint never proves a legacy identity"
        );
        assert_eq!(
            prove_retirement_endpoint(&legacy, "https://qdrant.example:6334", "another_base"),
            None,
            "the reconstruction uses the logical base collection, never the physical name"
        );
    }

    #[test]
    fn an_empty_attempt_endpoint_never_proves_a_legacy_identity() {
        let legacy = candidate("context69_chunks", "");
        assert_eq!(
            prove_retirement_endpoint(&legacy, "", "context69_chunks"),
            None
        );
        assert_eq!(
            prove_retirement_endpoint(&legacy, "   ", "context69_chunks"),
            None
        );
    }

    #[test]
    fn an_empty_saved_endpoint_is_ineligible_instead_of_falling_back() {
        let fingerprint = vector_identity_fingerprint(
            "https://qdrant.example:6334",
            "context69_chunks",
            "https://embed.example/v1",
            "model-a",
            768,
        );
        let mut saved_empty = candidate("context69_chunks", &fingerprint);
        saved_empty.qdrant_url = Some(String::new());
        assert_eq!(
            prove_retirement_endpoint(
                &saved_empty,
                "https://qdrant.example:6334",
                "context69_chunks"
            ),
            None,
            "an empty saved endpoint must not fall through to legacy reconstruction"
        );
        saved_empty.qdrant_url = Some("   ".to_string());
        assert_eq!(
            prove_retirement_endpoint(
                &saved_empty,
                "https://qdrant.example:6334",
                "context69_chunks"
            ),
            None
        );
    }

    #[test]
    fn the_replacement_collection_is_never_enqueued() {
        let old = candidate("context69_chunks", "fingerprint-a");
        assert_eq!(
            retirement_target(&old, "https://qdrant.example:6334", "context69_chunks__new"),
            Some(RetirementTarget {
                endpoint: "https://qdrant.example:6334".to_string(),
                collection_name: "context69_chunks".to_string(),
            })
        );
        assert_eq!(
            retirement_target(&old, "https://qdrant.example:6334", "context69_chunks"),
            None,
            "the physical destination is excluded"
        );
        assert_eq!(
            retirement_target(&old, "", "context69_chunks__new"),
            None,
            "an unproven endpoint enqueues nothing"
        );
    }
}
