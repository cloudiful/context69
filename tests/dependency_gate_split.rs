//! Phase 1 split tests: canonical keys, error routing, gate recovery,
//! legacy alias lookup, and no cross-gate recovery.
//!
//! Unit tests run always; DB integration tests are gated on
//! `CONTEXT69_TEST_DATABASE_URL` and use a per-test mutex to avoid
//! parallel interference, matching `tests/qdrant_cleanup_failure.rs`.

use context69::services::library::LibraryDependency;

// Unit: canonicalization
#[test]
fn canonical_key_maps_legacy_alias() {
    assert_eq!(
        LibraryDependency::canonical_key("embedding_vector"),
        "embedding"
    );
    assert_eq!(LibraryDependency::canonical_key("embedding"), "embedding");
    assert_eq!(LibraryDependency::canonical_key("qdrant"), "qdrant");
    assert_eq!(LibraryDependency::canonical_key("s3"), "s3");
    assert_eq!(LibraryDependency::canonical_key("docling"), "docling");
}

#[test]
fn canonical_str_is_distinct_for_qdrant_and_embedding() {
    assert_eq!(LibraryDependency::Embedding.canonical_str(), "embedding");
    assert_eq!(LibraryDependency::Qdrant.canonical_str(), "qdrant");
    assert_eq!(
        LibraryDependency::EmbeddingVector.canonical_str(),
        "embedding"
    );
    assert_eq!(LibraryDependency::S3.canonical_str(), "s3");
    assert_eq!(LibraryDependency::Docling.canonical_str(), "docling");
    assert_ne!(
        LibraryDependency::Embedding.canonical_str(),
        LibraryDependency::Qdrant.canonical_str()
    );
}

#[test]
fn from_str_handles_legacy_alias() {
    assert_eq!(
        "embedding_vector".parse::<LibraryDependency>().unwrap(),
        LibraryDependency::Embedding
    );
    assert_eq!(
        "embedding".parse::<LibraryDependency>().unwrap(),
        LibraryDependency::Embedding
    );
    assert_eq!(
        "qdrant".parse::<LibraryDependency>().unwrap(),
        LibraryDependency::Qdrant
    );
    assert_eq!(
        "s3".parse::<LibraryDependency>().unwrap(),
        LibraryDependency::S3
    );
    assert_eq!(
        "docling".parse::<LibraryDependency>().unwrap(),
        LibraryDependency::Docling
    );
}

// Integration: gate recovery and legacy alias lookup
#[cfg(test)]
mod integration {
    use super::*;
    use context69::db::Database;
    use context69::library_store::LibraryStore;
    use uuid::Uuid;

    static SUITE_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    async fn connect_db() -> Option<Database> {
        let url = std::env::var("CONTEXT69_TEST_DATABASE_URL").ok()?;
        Some(
            Database::connect(&url)
                .await
                .expect("connect test database"),
        )
    }

    async fn prepare_isolated_db() -> Option<(tokio::sync::MutexGuard<'static, ()>, Database)> {
        let guard = SUITE_LOCK.lock().await;
        let db = connect_db().await?;
        Some((guard, db))
    }

    #[tokio::test]
    async fn legacy_embedding_vector_alias_is_found_for_canonical_embedding() {
        let Some((_guard, db)) = prepare_isolated_db().await else {
            eprintln!("CONTEXT69_TEST_DATABASE_URL not set; skipping legacy alias test");
            return;
        };
        let store = LibraryStore::new(db.clone());
        // Ensure clean slate for this test's keys
        for key in ["embedding", "embedding_vector", "qdrant"] {
            sqlx::query("DELETE FROM context69.library_dependency_gates WHERE dependency_key = $1")
                .bind(key)
                .execute(db.pool())
                .await
                .expect("delete gate");
        }
        // Insert only the legacy row as production had before phase 1
        sqlx::query(
            "INSERT INTO context69.library_dependency_gates (dependency_key, state) VALUES ('embedding_vector', 'closed') ON CONFLICT (dependency_key) DO UPDATE SET state='closed', failure_count=0, last_error=NULL, next_probe_at=NULL, probe_lease_token=NULL, probe_lease_expires_at=NULL, updated_at=now()",
        )
        .execute(db.pool())
        .await
        .expect("insert legacy");

        // Simulate dependency_wait_until logic: canonical lookup should find legacy
        let gates = store.list_dependency_gates().await.expect("list gates");
        let canonical = LibraryDependency::canonical_key("embedding");
        let found = gates
            .iter()
            .find(|gate| gate.dependency_key == canonical)
            .or_else(|| {
                gates.iter().find(|gate| {
                    LibraryDependency::canonical_key(&gate.dependency_key) == canonical
                })
            });
        assert!(
            found.is_some(),
            "canonical embedding should be satisfied by legacy embedding_vector row"
        );
        assert_eq!(found.unwrap().dependency_key, "embedding_vector");
        assert_eq!(found.unwrap().state, "closed");

        // Now ensure canonical embedding is created and verify both can coexist
        store
            .ensure_dependency_gate("embedding")
            .await
            .expect("ensure embedding");
        let gates2 = store.list_dependency_gates().await.expect("list gates2");
        assert!(gates2.iter().any(|g| g.dependency_key == "embedding"));
        // Cleanup
        for key in ["embedding", "embedding_vector", "qdrant"] {
            sqlx::query("DELETE FROM context69.library_dependency_gates WHERE dependency_key = $1")
                .bind(key)
                .execute(db.pool())
                .await
                .expect("cleanup");
        }
        // Re-insert defaults for other tests
        for key in ["s3", "docling", "embedding_vector"] {
            sqlx::query(
                "INSERT INTO context69.library_dependency_gates (dependency_key) VALUES ($1) ON CONFLICT (dependency_key) DO NOTHING",
            )
            .bind(key)
            .execute(db.pool())
            .await
            .expect("reinsert");
        }
    }

    #[tokio::test]
    async fn qdrant_and_embedding_gates_recover_independently() {
        let Some((_guard, db)) = prepare_isolated_db().await else {
            eprintln!("CONTEXT69_TEST_DATABASE_URL not set; skipping independent recovery test");
            return;
        };
        let store = LibraryStore::new(db.clone());
        let fp = "test-fp-independent";
        // Clean
        for key in ["embedding", "qdrant", "embedding_vector"] {
            sqlx::query("DELETE FROM context69.library_dependency_gates WHERE dependency_key = $1")
                .bind(key)
                .execute(db.pool())
                .await
                .expect("delete");
        }
        for key in ["embedding", "qdrant", "embedding_vector"] {
            store.ensure_dependency_gate(key).await.expect("ensure");
            // Configure as closed
            store
                .configure_dependency_gate(key, true, None, fp)
                .await
                .expect("configure closed");
        }
        // Trip qdrant open via failure
        store
            .record_dependency_failure("qdrant", Uuid::nil(), "qdrant test failure")
            .await
            .expect("qdrant failure");
        let gates = store.list_dependency_gates().await.expect("list");
        let qdrant = gates.iter().find(|g| g.dependency_key == "qdrant").unwrap();
        let embedding = gates
            .iter()
            .find(|g| g.dependency_key == "embedding")
            .unwrap();
        assert_eq!(qdrant.state, "open", "qdrant should be open after failure");
        assert_eq!(
            embedding.state, "closed",
            "embedding must stay closed when only qdrant failed"
        );

        // Trip embedding open, ensure qdrant stays open (both open)
        store
            .record_dependency_failure("embedding", Uuid::nil(), "embedding test failure")
            .await
            .expect("embedding failure");
        let gates2 = store.list_dependency_gates().await.expect("list2");
        let qdrant2 = gates2
            .iter()
            .find(|g| g.dependency_key == "qdrant")
            .unwrap();
        let embedding2 = gates2
            .iter()
            .find(|g| g.dependency_key == "embedding")
            .unwrap();
        assert_eq!(qdrant2.state, "open");
        assert_eq!(embedding2.state, "open");

        // Recover qdrant via success with lease
        // Need to reserve probe first to get half_open
        let token = Uuid::new_v4();
        // Force probe: set next_probe_at to past by updating failure then waiting?
        // Instead directly call success with nil token if closed? But qdrant is open, so success requires half_open + token.
        // We can test the transition logic: reserve probe then success.
        // Make qdrant probeable by setting next_probe_at to now()
        sqlx::query(
            "UPDATE context69.library_dependency_gates SET next_probe_at = now() - interval '1 second' WHERE dependency_key = 'qdrant'",
        )
        .execute(db.pool())
        .await
        .expect("force probe");
        let reserved = store
            .reserve_dependency_probe("qdrant", token, 120)
            .await
            .expect("reserve");
        assert!(reserved.is_some(), "should reserve qdrant probe");
        store
            .record_dependency_success("qdrant", token)
            .await
            .expect("qdrant success");
        let gates3 = store.list_dependency_gates().await.expect("list3");
        let qdrant3 = gates3
            .iter()
            .find(|g| g.dependency_key == "qdrant")
            .unwrap();
        let embedding3 = gates3
            .iter()
            .find(|g| g.dependency_key == "embedding")
            .unwrap();
        assert_eq!(
            qdrant3.state, "closed",
            "qdrant should be closed after successful probe"
        );
        assert_eq!(
            embedding3.state, "open",
            "embedding must stay open when only qdrant recovered"
        );

        // Cleanup: close embedding as well
        sqlx::query(
            "UPDATE context69.library_dependency_gates SET next_probe_at = now() - interval '1 second' WHERE dependency_key = 'embedding'",
        )
        .execute(db.pool())
        .await
        .expect("force embed probe");
        let token2 = Uuid::new_v4();
        store
            .reserve_dependency_probe("embedding", token2, 120)
            .await
            .expect("reserve embed");
        store
            .record_dependency_success("embedding", token2)
            .await
            .expect("embed success");

        // Final cleanup: ensure gates are closed and remove test rows
        for key in ["embedding", "qdrant", "embedding_vector"] {
            sqlx::query("DELETE FROM context69.library_dependency_gates WHERE dependency_key = $1")
                .bind(key)
                .execute(db.pool())
                .await
                .expect("cleanup");
        }
        for key in ["s3", "docling", "embedding_vector"] {
            sqlx::query(
                "INSERT INTO context69.library_dependency_gates (dependency_key) VALUES ($1) ON CONFLICT (dependency_key) DO NOTHING",
            )
            .bind(key)
            .execute(db.pool())
            .await
            .expect("reinsert");
        }
    }

    #[tokio::test]
    async fn ensure_qdrant_gate_is_created_idempotently() {
        let Some((_guard, db)) = prepare_isolated_db().await else {
            eprintln!("CONTEXT69_TEST_DATABASE_URL not set; skipping ensure test");
            return;
        };
        let store = LibraryStore::new(db.clone());
        // Delete qdrant if exists
        sqlx::query(
            "DELETE FROM context69.library_dependency_gates WHERE dependency_key = 'qdrant'",
        )
        .execute(db.pool())
        .await
        .expect("delete qdrant");
        // Ensure creates it
        store
            .ensure_dependency_gate("qdrant")
            .await
            .expect("ensure qdrant");
        let gates = store.list_dependency_gates().await.expect("list");
        assert!(gates.iter().any(|g| g.dependency_key == "qdrant"));
        // Second ensure is idempotent
        store
            .ensure_dependency_gate("qdrant")
            .await
            .expect("ensure again");
        let gates2 = store.list_dependency_gates().await.expect("list2");
        assert_eq!(
            gates
                .iter()
                .filter(|g| g.dependency_key == "qdrant")
                .count(),
            1
        );
        assert_eq!(
            gates2
                .iter()
                .filter(|g| g.dependency_key == "qdrant")
                .count(),
            1
        );
        // Cleanup
        sqlx::query(
            "DELETE FROM context69.library_dependency_gates WHERE dependency_key = 'qdrant'",
        )
        .execute(db.pool())
        .await
        .expect("cleanup qdrant");
    }

    #[tokio::test]
    async fn no_cross_gate_recovery() {
        let Some((_guard, db)) = prepare_isolated_db().await else {
            eprintln!("CONTEXT69_TEST_DATABASE_URL not set; skipping cross-gate test");
            return;
        };
        let store = LibraryStore::new(db.clone());
        let fp = "test-fp-cross";
        for key in ["embedding", "qdrant"] {
            sqlx::query("DELETE FROM context69.library_dependency_gates WHERE dependency_key = $1")
                .bind(key)
                .execute(db.pool())
                .await
                .expect("delete");
            store.ensure_dependency_gate(key).await.expect("ensure");
            store
                .configure_dependency_gate(key, true, None, fp)
                .await
                .expect("configure");
        }
        // Open both
        store
            .record_dependency_failure("embedding", Uuid::nil(), "e fail")
            .await
            .expect("e fail");
        store
            .record_dependency_failure("qdrant", Uuid::nil(), "q fail")
            .await
            .expect("q fail");
        // Make only embedding probeable and recover it
        sqlx::query(
            "UPDATE context69.library_dependency_gates SET next_probe_at = now() - interval '1 second' WHERE dependency_key = 'embedding'",
        )
        .execute(db.pool())
        .await
        .expect("probe embed");
        let token = Uuid::new_v4();
        store
            .reserve_dependency_probe("embedding", token, 120)
            .await
            .expect("reserve");
        store
            .record_dependency_success("embedding", token)
            .await
            .expect("success");
        let gates = store.list_dependency_gates().await.expect("list");
        let embedding = gates
            .iter()
            .find(|g| g.dependency_key == "embedding")
            .unwrap();
        let qdrant = gates.iter().find(|g| g.dependency_key == "qdrant").unwrap();
        assert_eq!(embedding.state, "closed");
        assert_eq!(
            qdrant.state, "open",
            "qdrant must remain open when only embedding recovered"
        );
        // Cleanup
        for key in ["embedding", "qdrant", "embedding_vector"] {
            sqlx::query("DELETE FROM context69.library_dependency_gates WHERE dependency_key = $1")
                .bind(key)
                .execute(db.pool())
                .await
                .expect("cleanup");
        }
        for key in ["s3", "docling", "embedding_vector"] {
            sqlx::query(
                "INSERT INTO context69.library_dependency_gates (dependency_key) VALUES ($1) ON CONFLICT (dependency_key) DO NOTHING",
            )
            .bind(key)
            .execute(db.pool())
            .await
            .expect("reinsert");
        }
    }
}
