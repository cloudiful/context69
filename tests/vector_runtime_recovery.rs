//! Delayed-Qdrant recovery: a runtime that is unavailable for the whole
//! startup budget becomes usable afterwards for every consumer, and a runtime
//! that never becomes available stays honestly unavailable.
//!
//! The case uses the loopback-only test index constructor (no network) and a
//! deterministic sleeper, so it proves the recovery sequencing without a real
//! Qdrant deployment or a slow real-time wait.

#![cfg(feature = "integration-test-helpers")]

use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};

use anyhow::Result;
use async_trait::async_trait;
use context69::{
    embedding::{EmbeddingIdentity, EmbeddingProvider},
    qdrant_index::QdrantIndex,
    vector_runtime::{
        VectorRuntime,
        recovery::{RecoveryPolicy, Sleeper, VectorConnector, VectorPair, VectorRecovery},
    },
};
use tokio_util::sync::CancellationToken;

struct TestProvider;

#[async_trait]
impl EmbeddingProvider for TestProvider {
    async fn embed_texts(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        Ok(texts.iter().map(|_| vec![0.0; 8]).collect())
    }
}

struct DelayedConnector {
    available: AtomicBool,
    attach_calls: AtomicUsize,
    collection: Mutex<String>,
}

#[async_trait]
impl VectorConnector for DelayedConnector {
    async fn pair(&self) -> Result<VectorPair> {
        Ok(VectorPair {
            embedding: Arc::new(TestProvider),
            identity: EmbeddingIdentity {
                base_url: "https://embeddings.example/v1".to_string(),
                model: "model-a".to_string(),
                dimensions: 8,
            },
            collection: self.collection.lock().expect("collection").clone(),
        })
    }

    async fn attach(&self, pair: &VectorPair) -> Result<Arc<QdrantIndex>> {
        self.attach_calls.fetch_add(1, Ordering::SeqCst);
        if !self.available.load(Ordering::Acquire) {
            return Err(anyhow::anyhow!(
                "qdrant collection {} is unreachable: connection refused",
                pair.collection
            ));
        }
        Ok(Arc::new(QdrantIndex::for_test_unreachable(
            "http://127.0.0.1:1",
            &pair.collection,
            8,
        )?))
    }

    async fn probe(&self, _snapshot: &context69::vector_runtime::VectorSnapshot) -> Result<()> {
        // The paired snapshot is what a probe would use; this case never
        // reaches a probe because the sleeper is stepped only after the
        // publication assertion.
        Ok(())
    }

    fn configuration_fingerprint(&self) -> String {
        "delayed-runtime".to_string()
    }
}

#[derive(Default)]
struct CountingSleeper {
    sleeps: AtomicUsize,
}

#[async_trait]
impl Sleeper for CountingSleeper {
    async fn sleep(&self, _duration: Duration) {
        self.sleeps.fetch_add(1, Ordering::SeqCst);
        tokio::task::yield_now().await;
    }
}

struct StepSleeper {
    notify: tokio::sync::Notify,
}

impl StepSleeper {
    fn new() -> Self {
        Self {
            notify: tokio::sync::Notify::new(),
        }
    }

    fn step(&self) {
        self.notify.notify_one();
    }
}

#[async_trait]
impl Sleeper for StepSleeper {
    async fn sleep(&self, _duration: Duration) {
        self.notify.notified().await;
    }
}

fn configured_runtime() -> VectorRuntime {
    VectorRuntime::configured(Arc::new(AtomicBool::new(false)))
}

#[tokio::test]
async fn a_runtime_unavailable_for_the_whole_budget_heals_every_consumer_later() {
    let runtime = configured_runtime();
    let connector = Arc::new(DelayedConnector {
        available: AtomicBool::new(false),
        attach_calls: AtomicUsize::new(0),
        collection: Mutex::new("context69".to_string()),
    });
    let sleeper = Arc::new(CountingSleeper::default());
    let recovery = VectorRecovery::new(runtime.clone(), connector.clone(), sleeper.clone());

    assert!(
        !recovery.run_startup(&CancellationToken::new()).await,
        "a runtime unavailable for the whole budget must not fake a connection"
    );
    assert!(runtime.snapshot().is_none());
    assert!(!runtime.is_ready());
    assert!(runtime.require_ready().is_err());

    // The remote appears after the startup deadline.
    connector.available.store(true, Ordering::Release);
    let step = Arc::new(StepSleeper::new());
    let supervisor = VectorRecovery::new(runtime.clone(), connector.clone(), step.clone());
    let shutdown = CancellationToken::new();
    let handle = tokio::spawn({
        let shutdown = shutdown.clone();
        async move { supervisor.supervise(shutdown).await }
    });

    for _ in 0..50 {
        tokio::task::yield_now().await;
        if runtime.snapshot().is_some() {
            break;
        }
    }

    let published = runtime
        .snapshot()
        .expect("the recovered runtime must publish a pair");
    assert!(runtime.is_ready());
    assert_eq!(published.generation, runtime.generation());

    // Every existing consumer takes the same coherent pair from the handle.
    for _consumer in 0..7 {
        let snapshot = runtime.require_ready().expect("consumer sees the pair");
        assert_eq!(snapshot.generation, published.generation);
        assert_eq!(Arc::as_ptr(&snapshot.index), Arc::as_ptr(&published.index));
        assert_eq!(snapshot.collection, "context69");
    }

    step.step();
    shutdown.cancel();
    let _ = handle.await;
}

#[tokio::test]
async fn a_runtime_that_never_appears_stays_unavailable_without_a_retry_storm() {
    let runtime = configured_runtime();
    let connector = Arc::new(DelayedConnector {
        available: AtomicBool::new(false),
        attach_calls: AtomicUsize::new(0),
        collection: Mutex::new("context69".to_string()),
    });
    let sleeper = Arc::new(CountingSleeper::default());
    let policy = RecoveryPolicy {
        startup_budget: Duration::from_secs(30),
        ..RecoveryPolicy::default()
    };
    let recovery =
        VectorRecovery::new(runtime.clone(), connector.clone(), sleeper).with_policy(policy);

    assert!(!recovery.run_startup(&CancellationToken::new()).await);
    let attempts = connector.attach_calls.load(Ordering::SeqCst);
    assert!(attempts >= 1);
    assert!(
        attempts <= 6,
        "startup must stay bounded, observed {attempts} attempts"
    );
    assert!(runtime.snapshot().is_none());
    assert!(!runtime.is_ready());
}

/// A connector whose attempts are counted, so a dormant instance can prove it
/// never touched a provider or an endpoint.
struct CountingConnector {
    pair_calls: AtomicUsize,
    attach_calls: AtomicUsize,
}

impl CountingConnector {
    fn new() -> Self {
        Self {
            pair_calls: AtomicUsize::new(0),
            attach_calls: AtomicUsize::new(0),
        }
    }
}

#[async_trait]
impl VectorConnector for CountingConnector {
    async fn pair(&self) -> Result<VectorPair> {
        self.pair_calls.fetch_add(1, Ordering::SeqCst);
        Ok(VectorPair {
            embedding: Arc::new(TestProvider),
            identity: EmbeddingIdentity {
                base_url: "https://embeddings.example/v1".to_string(),
                model: "model-a".to_string(),
                dimensions: 8,
            },
            collection: "context69".to_string(),
        })
    }

    async fn attach(&self, pair: &VectorPair) -> Result<Arc<QdrantIndex>> {
        self.attach_calls.fetch_add(1, Ordering::SeqCst);
        Ok(Arc::new(QdrantIndex::for_test_unreachable(
            "http://127.0.0.1:1",
            &pair.collection,
            8,
        )?))
    }

    async fn probe(&self, _snapshot: &context69::vector_runtime::VectorSnapshot) -> Result<()> {
        Ok(())
    }

    fn configuration_fingerprint(&self) -> String {
        "counting-runtime".to_string()
    }
}

#[tokio::test]
async fn a_dormant_instance_touches_nothing_until_configuration_is_committed() {
    let runtime = VectorRuntime::unconfigured();
    let connector = Arc::new(CountingConnector::new());
    let supervisor = VectorRecovery::new(
        runtime.clone(),
        connector.clone(),
        Arc::new(CountingSleeper::default()),
    );
    let shutdown = CancellationToken::new();
    let handle = tokio::spawn({
        let shutdown = shutdown.clone();
        async move { supervisor.supervise(shutdown).await }
    });

    for _ in 0..50 {
        tokio::task::yield_now().await;
    }
    assert_eq!(
        connector.pair_calls.load(Ordering::SeqCst),
        0,
        "a dormant instance must not build a provider"
    );
    assert_eq!(connector.attach_calls.load(Ordering::SeqCst), 0);
    assert!(runtime.snapshot().is_none());
    assert!(!runtime.is_ready());
    assert!(runtime.require_ready().is_err());

    // The first committed settings save marks the runtime configured and starts
    // a new generation, exactly as the application's observer does.
    runtime.mark_configured(true);
    runtime.begin_generation();

    for _ in 0..200 {
        tokio::task::yield_now().await;
        if runtime.snapshot().is_some() {
            break;
        }
    }

    let published = runtime
        .snapshot()
        .expect("activation must publish a pair without a restart");
    assert!(runtime.is_ready());
    assert_eq!(published.generation, runtime.generation());
    assert!(connector.pair_calls.load(Ordering::SeqCst) >= 1);
    assert!(connector.attach_calls.load(Ordering::SeqCst) >= 1);

    shutdown.cancel();
    let _ = handle.await;
}

#[tokio::test]
async fn a_configuration_change_makes_a_stale_completion_unable_to_reopen_readiness() {
    let runtime = configured_runtime();
    let connector = Arc::new(CountingConnector::new());
    let recovery = VectorRecovery::new(
        runtime.clone(),
        connector.clone(),
        Arc::new(CountingSleeper::default()),
    );

    assert!(recovery.run_startup(&CancellationToken::new()).await);
    let stale_generation = runtime.generation();
    assert!(runtime.is_ready());

    // A committed settings save advances the generation and withdraws readiness
    // from the pair the old generation published.
    runtime.begin_generation();
    assert!(!runtime.is_ready());
    assert!(
        !runtime.restore_ready(stale_generation),
        "a stale generation must never reopen readiness"
    );

    assert!(recovery.run_startup(&CancellationToken::new()).await);
    let current = runtime.snapshot().expect("the new generation publishes");
    assert_ne!(current.generation, stale_generation);
    assert_eq!(current.generation, runtime.generation());
    assert!(runtime.is_ready());
}

#[tokio::test]
async fn a_mismatched_persisted_identity_publishes_but_stays_unready() {
    let runtime = configured_runtime();
    runtime.set_requires_rebuild(true);
    let connector = Arc::new(CountingConnector::new());
    let recovery = VectorRecovery::new(
        runtime.clone(),
        connector.clone(),
        Arc::new(CountingSleeper::default()),
    );

    assert!(recovery.run_startup(&CancellationToken::new()).await);
    let published = runtime
        .snapshot()
        .expect("the configured identity still attaches for an explicit rebuild");
    assert!(runtime.requires_rebuild());
    assert!(
        !runtime.is_ready(),
        "recovery publication must not report readiness for a mismatched persisted identity"
    );
    assert!(runtime.require_ready().is_err());

    // A successful explicit rebuild is the only thing that reopens the pair.
    let rebuild = runtime.begin_rebuild();
    runtime.set_requires_rebuild(false);
    assert!(runtime.complete_rebuild(rebuild));
    assert!(runtime.is_ready());
    assert_eq!(
        runtime
            .require_ready()
            .expect("the rebuilt pair is ready")
            .generation,
        published.generation + 1
    );
}

#[tokio::test]
async fn cancelling_a_dormant_supervisor_ends_it_without_touching_a_dependency() {
    let runtime = VectorRuntime::unconfigured();
    let connector = Arc::new(CountingConnector::new());
    let supervisor = VectorRecovery::new(
        runtime.clone(),
        connector.clone(),
        Arc::new(StepSleeper::new()),
    );
    let shutdown = CancellationToken::new();
    let handle = tokio::spawn({
        let shutdown = shutdown.clone();
        async move { supervisor.supervise(shutdown).await }
    });

    for _ in 0..20 {
        tokio::task::yield_now().await;
    }
    shutdown.cancel();
    tokio::time::timeout(Duration::from_secs(2), handle)
        .await
        .expect("cancellation must end the dormant supervisor promptly")
        .expect("the supervisor task must not panic");
    assert_eq!(connector.pair_calls.load(Ordering::SeqCst), 0);
    assert_eq!(connector.attach_calls.load(Ordering::SeqCst), 0);
    assert!(runtime.snapshot().is_none());
}

/// The production unrecorded-collection bootstrap, driven against a loopback
/// Qdrant mock: an unknown populated collection at the configured dimension
/// yields a read-only index that recovery publishes for the explicit rebuild
/// only, never as readiness and never with an adopted identity.
#[tokio::test]
async fn an_unknown_populated_collection_is_recoverable_only_through_an_explicit_rebuild() {
    use context69::config::QdrantConfig;
    use context69::vector_runtime::attach_unrecorded_collection_for_test;
    use qdrant_client::qdrant::collections_server::{Collections, CollectionsServer};
    use qdrant_client::qdrant::points_server::{Points, PointsServer};
    use qdrant_client::qdrant::{
        CollectionConfig, CollectionExistsRequest, CollectionExistsResponse, CollectionInfo,
        CollectionParams, CountPoints, CountResponse, CountResult, GetCollectionInfoRequest,
        GetCollectionInfoResponse, VectorParams, VectorsConfig, vectors_config,
    };
    use tonic::transport::Server;
    use tonic::transport::server::TcpIncoming;
    use tonic::{Request, Response, Status};

    struct UnknownCollectionMock;

    #[tonic::async_trait]
    impl Collections for UnknownCollectionMock {
        async fn collection_exists(
            &self,
            _request: Request<CollectionExistsRequest>,
        ) -> Result<Response<CollectionExistsResponse>, Status> {
            Ok(Response::new(CollectionExistsResponse {
                result: Some(qdrant_client::qdrant::CollectionExists { exists: true }),
                time: 0.0,
            }))
        }

        async fn get(
            &self,
            _request: Request<GetCollectionInfoRequest>,
        ) -> Result<Response<GetCollectionInfoResponse>, Status> {
            let params = CollectionParams {
                vectors_config: Some(VectorsConfig {
                    config: Some(vectors_config::Config::Params(VectorParams {
                        size: 8,
                        ..VectorParams::default()
                    })),
                }),
                ..CollectionParams::default()
            };
            Ok(Response::new(GetCollectionInfoResponse {
                result: Some(CollectionInfo {
                    points_count: Some(5),
                    config: Some(CollectionConfig {
                        params: Some(params),
                        ..CollectionConfig::default()
                    }),
                    ..CollectionInfo::default()
                }),
                time: 0.0,
            }))
        }

        async fn list(
            &self,
            _request: Request<qdrant_client::qdrant::ListCollectionsRequest>,
        ) -> Result<Response<qdrant_client::qdrant::ListCollectionsResponse>, Status> {
            Err(Status::unimplemented(
                "the rebuild seam mock serves collection info only",
            ))
        }

        async fn collection_cluster_info(
            &self,
            _request: Request<qdrant_client::qdrant::CollectionClusterInfoRequest>,
        ) -> Result<Response<qdrant_client::qdrant::CollectionClusterInfoResponse>, Status>
        {
            Err(Status::unimplemented(
                "the rebuild seam mock serves collection info only",
            ))
        }

        async fn list_aliases(
            &self,
            _request: Request<qdrant_client::qdrant::ListAliasesRequest>,
        ) -> Result<Response<qdrant_client::qdrant::ListAliasesResponse>, Status> {
            Err(Status::unimplemented(
                "the rebuild seam mock serves collection info only",
            ))
        }

        async fn list_collection_aliases(
            &self,
            _request: Request<qdrant_client::qdrant::ListCollectionAliasesRequest>,
        ) -> Result<Response<qdrant_client::qdrant::ListAliasesResponse>, Status> {
            Err(Status::unimplemented(
                "the rebuild seam mock serves collection info only",
            ))
        }

        async fn list_shard_keys(
            &self,
            _request: Request<qdrant_client::qdrant::ListShardKeysRequest>,
        ) -> Result<Response<qdrant_client::qdrant::ListShardKeysResponse>, Status> {
            Err(Status::unimplemented(
                "the rebuild seam mock serves collection info only",
            ))
        }

        async fn create(
            &self,
            _request: Request<qdrant_client::qdrant::CreateCollection>,
        ) -> Result<Response<qdrant_client::qdrant::CollectionOperationResponse>, Status> {
            Err(Status::unimplemented(
                "the rebuild seam mock must never create",
            ))
        }

        async fn update(
            &self,
            _request: Request<qdrant_client::qdrant::UpdateCollection>,
        ) -> Result<Response<qdrant_client::qdrant::CollectionOperationResponse>, Status> {
            Err(Status::unimplemented(
                "the rebuild seam mock must never update",
            ))
        }

        async fn delete(
            &self,
            _request: Request<qdrant_client::qdrant::DeleteCollection>,
        ) -> Result<Response<qdrant_client::qdrant::CollectionOperationResponse>, Status> {
            Err(Status::unimplemented(
                "the rebuild seam mock must never delete",
            ))
        }

        async fn update_aliases(
            &self,
            _request: Request<qdrant_client::qdrant::ChangeAliases>,
        ) -> Result<Response<qdrant_client::qdrant::CollectionOperationResponse>, Status> {
            Err(Status::unimplemented(
                "the rebuild seam mock must never update aliases",
            ))
        }

        async fn update_collection_cluster_setup(
            &self,
            _request: Request<qdrant_client::qdrant::UpdateCollectionClusterSetupRequest>,
        ) -> Result<Response<qdrant_client::qdrant::UpdateCollectionClusterSetupResponse>, Status>
        {
            Err(Status::unimplemented(
                "the rebuild seam mock must never update clusters",
            ))
        }

        async fn create_shard_key(
            &self,
            _request: Request<qdrant_client::qdrant::CreateShardKeyRequest>,
        ) -> Result<Response<qdrant_client::qdrant::CreateShardKeyResponse>, Status> {
            Err(Status::unimplemented(
                "the rebuild seam mock must never create shards",
            ))
        }

        async fn delete_shard_key(
            &self,
            _request: Request<qdrant_client::qdrant::DeleteShardKeyRequest>,
        ) -> Result<Response<qdrant_client::qdrant::DeleteShardKeyResponse>, Status> {
            Err(Status::unimplemented(
                "the rebuild seam mock must never delete shards",
            ))
        }
    }

    struct CountOnlyMock;

    #[tonic::async_trait]
    impl Points for CountOnlyMock {
        async fn count(
            &self,
            _request: Request<CountPoints>,
        ) -> Result<Response<CountResponse>, Status> {
            Ok(Response::new(CountResponse {
                result: Some(CountResult { count: 5 }),
                time: 0.0,
                usage: None,
            }))
        }

        async fn upsert(
            &self,
            _request: Request<qdrant_client::qdrant::UpsertPoints>,
        ) -> Result<Response<qdrant_client::qdrant::PointsOperationResponse>, Status> {
            Err(Status::unimplemented("the rebuild seam mock is read-only"))
        }

        async fn delete(
            &self,
            _request: Request<qdrant_client::qdrant::DeletePoints>,
        ) -> Result<Response<qdrant_client::qdrant::PointsOperationResponse>, Status> {
            Err(Status::unimplemented("the rebuild seam mock is read-only"))
        }

        async fn get(
            &self,
            _request: Request<qdrant_client::qdrant::GetPoints>,
        ) -> Result<Response<qdrant_client::qdrant::GetResponse>, Status> {
            Err(Status::unimplemented("the rebuild seam mock is read-only"))
        }

        async fn update_vectors(
            &self,
            _request: Request<qdrant_client::qdrant::UpdatePointVectors>,
        ) -> Result<Response<qdrant_client::qdrant::PointsOperationResponse>, Status> {
            Err(Status::unimplemented("the rebuild seam mock is read-only"))
        }

        async fn delete_vectors(
            &self,
            _request: Request<qdrant_client::qdrant::DeletePointVectors>,
        ) -> Result<Response<qdrant_client::qdrant::PointsOperationResponse>, Status> {
            Err(Status::unimplemented("the rebuild seam mock is read-only"))
        }

        async fn set_payload(
            &self,
            _request: Request<qdrant_client::qdrant::SetPayloadPoints>,
        ) -> Result<Response<qdrant_client::qdrant::PointsOperationResponse>, Status> {
            Err(Status::unimplemented("the rebuild seam mock is read-only"))
        }

        async fn overwrite_payload(
            &self,
            _request: Request<qdrant_client::qdrant::SetPayloadPoints>,
        ) -> Result<Response<qdrant_client::qdrant::PointsOperationResponse>, Status> {
            Err(Status::unimplemented("the rebuild seam mock is read-only"))
        }

        async fn delete_payload(
            &self,
            _request: Request<qdrant_client::qdrant::DeletePayloadPoints>,
        ) -> Result<Response<qdrant_client::qdrant::PointsOperationResponse>, Status> {
            Err(Status::unimplemented("the rebuild seam mock is read-only"))
        }

        async fn clear_payload(
            &self,
            _request: Request<qdrant_client::qdrant::ClearPayloadPoints>,
        ) -> Result<Response<qdrant_client::qdrant::PointsOperationResponse>, Status> {
            Err(Status::unimplemented("the rebuild seam mock is read-only"))
        }

        async fn create_field_index(
            &self,
            _request: Request<qdrant_client::qdrant::CreateFieldIndexCollection>,
        ) -> Result<Response<qdrant_client::qdrant::PointsOperationResponse>, Status> {
            Err(Status::unimplemented("the rebuild seam mock is read-only"))
        }

        async fn delete_field_index(
            &self,
            _request: Request<qdrant_client::qdrant::DeleteFieldIndexCollection>,
        ) -> Result<Response<qdrant_client::qdrant::PointsOperationResponse>, Status> {
            Err(Status::unimplemented("the rebuild seam mock is read-only"))
        }

        async fn search(
            &self,
            _request: Request<qdrant_client::qdrant::SearchPoints>,
        ) -> Result<Response<qdrant_client::qdrant::SearchResponse>, Status> {
            Err(Status::unimplemented("the rebuild seam mock is read-only"))
        }

        async fn search_batch(
            &self,
            _request: Request<qdrant_client::qdrant::SearchBatchPoints>,
        ) -> Result<Response<qdrant_client::qdrant::SearchBatchResponse>, Status> {
            Err(Status::unimplemented("the rebuild seam mock is read-only"))
        }

        async fn search_groups(
            &self,
            _request: Request<qdrant_client::qdrant::SearchPointGroups>,
        ) -> Result<Response<qdrant_client::qdrant::SearchGroupsResponse>, Status> {
            Err(Status::unimplemented("the rebuild seam mock is read-only"))
        }

        async fn scroll(
            &self,
            _request: Request<qdrant_client::qdrant::ScrollPoints>,
        ) -> Result<Response<qdrant_client::qdrant::ScrollResponse>, Status> {
            Err(Status::unimplemented("the rebuild seam mock is read-only"))
        }

        async fn recommend(
            &self,
            _request: Request<qdrant_client::qdrant::RecommendPoints>,
        ) -> Result<Response<qdrant_client::qdrant::RecommendResponse>, Status> {
            Err(Status::unimplemented("the rebuild seam mock is read-only"))
        }

        async fn recommend_batch(
            &self,
            _request: Request<qdrant_client::qdrant::RecommendBatchPoints>,
        ) -> Result<Response<qdrant_client::qdrant::RecommendBatchResponse>, Status> {
            Err(Status::unimplemented("the rebuild seam mock is read-only"))
        }

        async fn recommend_groups(
            &self,
            _request: Request<qdrant_client::qdrant::RecommendPointGroups>,
        ) -> Result<Response<qdrant_client::qdrant::RecommendGroupsResponse>, Status> {
            Err(Status::unimplemented("the rebuild seam mock is read-only"))
        }

        async fn discover(
            &self,
            _request: Request<qdrant_client::qdrant::DiscoverPoints>,
        ) -> Result<Response<qdrant_client::qdrant::DiscoverResponse>, Status> {
            Err(Status::unimplemented("the rebuild seam mock is read-only"))
        }

        async fn discover_batch(
            &self,
            _request: Request<qdrant_client::qdrant::DiscoverBatchPoints>,
        ) -> Result<Response<qdrant_client::qdrant::DiscoverBatchResponse>, Status> {
            Err(Status::unimplemented("the rebuild seam mock is read-only"))
        }

        async fn update_batch(
            &self,
            _request: Request<qdrant_client::qdrant::UpdateBatchPoints>,
        ) -> Result<Response<qdrant_client::qdrant::UpdateBatchResponse>, Status> {
            Err(Status::unimplemented("the rebuild seam mock is read-only"))
        }

        async fn query(
            &self,
            _request: Request<qdrant_client::qdrant::QueryPoints>,
        ) -> Result<Response<qdrant_client::qdrant::QueryResponse>, Status> {
            Err(Status::unimplemented("the rebuild seam mock is read-only"))
        }

        async fn query_batch(
            &self,
            _request: Request<qdrant_client::qdrant::QueryBatchPoints>,
        ) -> Result<Response<qdrant_client::qdrant::QueryBatchResponse>, Status> {
            Err(Status::unimplemented("the rebuild seam mock is read-only"))
        }

        async fn query_groups(
            &self,
            _request: Request<qdrant_client::qdrant::QueryPointGroups>,
        ) -> Result<Response<qdrant_client::qdrant::QueryGroupsResponse>, Status> {
            Err(Status::unimplemented("the rebuild seam mock is read-only"))
        }

        async fn facet(
            &self,
            _request: Request<qdrant_client::qdrant::FacetCounts>,
        ) -> Result<Response<qdrant_client::qdrant::FacetResponse>, Status> {
            Err(Status::unimplemented("the rebuild seam mock is read-only"))
        }

        async fn search_matrix_pairs(
            &self,
            _request: Request<qdrant_client::qdrant::SearchMatrixPoints>,
        ) -> Result<Response<qdrant_client::qdrant::SearchMatrixPairsResponse>, Status> {
            Err(Status::unimplemented("the rebuild seam mock is read-only"))
        }

        async fn search_matrix_offsets(
            &self,
            _request: Request<qdrant_client::qdrant::SearchMatrixPoints>,
        ) -> Result<Response<qdrant_client::qdrant::SearchMatrixOffsetsResponse>, Status> {
            Err(Status::unimplemented("the rebuild seam mock is read-only"))
        }

        async fn create_vector_name(
            &self,
            _request: Request<qdrant_client::qdrant::CreateVectorNameRequest>,
        ) -> Result<Response<qdrant_client::qdrant::PointsOperationResponse>, Status> {
            Err(Status::unimplemented("the rebuild seam mock is read-only"))
        }

        async fn delete_vector_name(
            &self,
            _request: Request<qdrant_client::qdrant::DeleteVectorNameRequest>,
        ) -> Result<Response<qdrant_client::qdrant::PointsOperationResponse>, Status> {
            Err(Status::unimplemented("the rebuild seam mock is read-only"))
        }
    }

    let incoming = TcpIncoming::bind("127.0.0.1:0".parse().expect("bind address"))
        .expect("an ephemeral loopback port must be available");
    let address = incoming.local_addr().expect("bound address");
    let (shutdown, shutdown_rx) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        Server::builder()
            .add_service(CollectionsServer::new(UnknownCollectionMock))
            .add_service(PointsServer::new(CountOnlyMock))
            .serve_with_incoming_shutdown(incoming, async {
                let _ = shutdown_rx.await;
            })
            .await
            .expect("the loopback mock must serve");
    });

    let qdrant = QdrantConfig {
        url: format!("http://{address}"),
        collection_name: "unknown_collection".to_string(),
        // The legacy destructive toggle stays inert.
        recreate_on_dimension_mismatch: true,
    };
    let runtime = configured_runtime();
    runtime.set_configured_fingerprint(Some("https://embeddings.example/v1|model-a|8".to_string()));
    let (rebuild_only, index) = attach_unrecorded_collection_for_test(&qdrant, 8)
        .await
        .expect("an unknown populated same-dimension collection must attach read-only");
    assert!(rebuild_only);
    let index = index.expect("the rebuild path publishes a read-only index");
    runtime.set_requires_rebuild(true);

    let connector = Arc::new(RebuildSeamConnector {
        index: Mutex::new(Some(index)),
    });
    let recovery = VectorRecovery::new(
        runtime.clone(),
        connector,
        Arc::new(CountingSleeper::default()),
    );
    assert!(recovery.run_startup(&CancellationToken::new()).await);

    // The pair is published so the explicit rebuild can use it, but it is not
    // readiness and it never adopted the unknown identity.
    assert!(runtime.snapshot().is_some());
    assert!(runtime.requires_rebuild());
    assert!(!runtime.is_ready());
    assert!(runtime.require_ready().is_err());
    assert!(
        runtime.require_snapshot().is_ok(),
        "manual rebuild admission needs the snapshot"
    );
    let generation = runtime
        .try_begin_rebuild()
        .expect("the explicit rebuild is admitted from the rebuild-only state");
    runtime.set_requires_rebuild(false);
    assert!(runtime.complete_rebuild(generation));
    assert!(runtime.is_ready());

    let _ = shutdown.send(());
    let _ = server.await;
}

struct RebuildSeamConnector {
    index: Mutex<Option<Arc<QdrantIndex>>>,
}

#[async_trait]
impl VectorConnector for RebuildSeamConnector {
    async fn pair(&self) -> Result<VectorPair> {
        Ok(VectorPair {
            embedding: Arc::new(TestProvider),
            identity: EmbeddingIdentity {
                base_url: "https://embeddings.example/v1".to_string(),
                model: "model-a".to_string(),
                dimensions: 8,
            },
            collection: "unknown_collection".to_string(),
        })
    }

    async fn attach(&self, _pair: &VectorPair) -> Result<Arc<QdrantIndex>> {
        self.index
            .lock()
            .expect("index")
            .clone()
            .ok_or_else(|| anyhow::anyhow!("the rebuild-only index is published once"))
    }

    async fn probe(&self, _snapshot: &context69::vector_runtime::VectorSnapshot) -> Result<()> {
        Ok(())
    }

    fn configuration_fingerprint(&self) -> String {
        "rebuild-seam".to_string()
    }
}
