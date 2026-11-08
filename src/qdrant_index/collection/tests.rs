use qdrant_client::qdrant::FieldType;

use super::super::errors::RebuildTargetCollisionReason;
use super::{
    CODE_PAYLOAD_FIELD_INDEXES, PAYLOAD_FIELD_INDEXES, TargetPreparation,
    is_qdrant_field_index_already_exists, metadata_payload_key, target_preparation,
};

/// Build a `QdrantError` whose `Display` output mirrors the shape the
/// qdrant-client produces for a gRPC `AlreadyExists` response so the
/// idempotency matcher is exercised against real substring patterns
/// without depending on `tonic` directly. `ConversionError` is the only
/// enum variant that takes a free-form `String` we control end-to-end.
fn error_mirroring_response_error(text: &str) -> qdrant_client::QdrantError {
    qdrant_client::QdrantError::ConversionError(format!("Error in the response: {text}"))
}

#[test]
fn a_created_target_of_the_matching_dimension_is_only_attached() {
    assert_eq!(
        target_preparation(true, Some(1024), 1024),
        TargetPreparation::Attach,
        "a target this attempt created is attached, never recreated"
    );
}

#[test]
fn a_created_target_of_another_dimension_is_never_adopted() {
    assert_eq!(
        target_preparation(true, Some(768), 1024),
        TargetPreparation::Collision(RebuildTargetCollisionReason::DimensionMismatch),
        "a different vector space is refused, not adopted or emptied"
    );
}

#[test]
fn a_created_target_that_is_gone_is_never_written_under_another_name() {
    assert_eq!(
        target_preparation(true, None, 1024),
        TargetPreparation::Collision(RebuildTargetCollisionReason::Missing),
        "the owned target of this attempt is gone, so the attempt writes nothing"
    );
}

#[test]
fn an_absent_target_without_an_owner_is_created() {
    assert_eq!(
        target_preparation(false, None, 1024),
        TargetPreparation::Create,
        "a target that does not exist yet may be created by this attempt"
    );
}

#[test]
fn an_existing_target_without_an_owner_is_a_collision() {
    assert_eq!(
        target_preparation(false, Some(1024), 1024),
        TargetPreparation::Collision(RebuildTargetCollisionReason::AlreadyExists),
        "an unmarked target that exists is ambiguous and is never adopted"
    );
}

#[test]
fn metadata_payload_key_prefixes_namespace() {
    assert_eq!(
        metadata_payload_key("library_file_id"),
        "metadata_index.library_file_id"
    );
}

#[test]
fn payload_index_already_exists_is_treated_as_idempotent_success() {
    for message in [
        "AlreadyExists Field Index already exists {}",
        "AlreadyExists field index already exists on this collection {}",
        "AlreadyExists payload index already exists for field 'library_file_id' {}",
        "AlreadyExists Index already exists for this field name {}",
    ] {
        let error = error_mirroring_response_error(message);
        assert!(
            is_qdrant_field_index_already_exists(&error),
            "expected positive match for {message:?}"
        );
    }
}

#[test]
fn field_index_already_exists_does_not_match_other_codes() {
    // Without the gRPC `AlreadyExists` code, the helper must never match,
    // even if the message text mentions field-index existed.
    for message in [
        "PermissionDenied permission denied for collection {}",
        "InvalidArgument validation failed {}",
        "Unauthenticated transport error: connection refused {}",
        "Internal field index already exists {}",
    ] {
        let error = error_mirroring_response_error(message);
        assert!(
            !is_qdrant_field_index_already_exists(&error),
            "expected negative match for {message:?}"
        );
    }
}

#[test]
fn field_index_already_exists_rejects_unrelated_already_mentions() {
    // Even on the AlreadyExists code, random text containing "already"
    // without the field-index hint must not be classified idempotent.
    for message in [
        "AlreadyExists collection already exists {}",
        "AlreadyExists snapshot already exists {}",
        "AlreadyExists some random unrelated text {}",
    ] {
        let error = error_mirroring_response_error(message);
        assert!(
            !is_qdrant_field_index_already_exists(&error),
            "expected negative match for {message:?}"
        );
    }
}

#[test]
fn payload_field_indexes_preserve_baseline_types() {
    // Exact (field, FieldType) pairs. Collapsing any integer field to
    // Keyword would either fail the Qdrant create_field_index call
    // (wrong schema on integer values) or silently regress filter
    // behaviour; this table pins the baseline types.
    let expected: &[(&str, FieldType)] = &[
        ("group_key", FieldType::Keyword),
        ("group_path", FieldType::Keyword),
        ("group_id", FieldType::Integer),
        ("visibility", FieldType::Keyword),
        ("source_key", FieldType::Keyword),
        ("document_id", FieldType::Integer),
        ("published_ts", FieldType::Integer),
        ("library_file_id", FieldType::Keyword),
    ];
    assert_eq!(PAYLOAD_FIELD_INDEXES, expected);
}

#[test]
fn payload_field_indexes_pin_integer_fields_explicitly() {
    // Dedicated guard for the integer group/document/timestamp fields so
    // a future refactor that accidentally drops them to Keyword fails
    // loudly instead of rebuilding the Qdrant payload schema.
    for field in ["group_id", "document_id", "published_ts"] {
        let entry = PAYLOAD_FIELD_INDEXES
            .iter()
            .find(|(name, _)| *name == field)
            .unwrap_or_else(|| panic!("payload field index missing for {field}"));
        assert_eq!(
            entry.1,
            FieldType::Integer,
            "{field} must remain an Integer payload index"
        );
    }
}

#[test]
fn payload_field_indexes_pin_library_file_id_as_keyword() {
    // The cleanup filter uses `Condition::matches("library_file_id", ...)`,
    // so the field must be indexed as Keyword. A regression to any other
    // type would silently break the fast cleanup path.
    let entry = PAYLOAD_FIELD_INDEXES
        .iter()
        .find(|(name, _)| *name == super::LIBRARY_FILE_ID_FIELD)
        .expect("library_file_id must be present in the payload index table");
    assert_eq!(entry.1, FieldType::Keyword);
}

/// A minimal Collections stub for the pre-create authority boundary.
mod gated_collections {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use qdrant_client::qdrant::collections_server::{Collections, CollectionsServer};
    use qdrant_client::qdrant::{
        ChangeAliases, CollectionClusterInfoRequest, CollectionClusterInfoResponse,
        CollectionExists, CollectionExistsRequest, CollectionExistsResponse,
        CollectionOperationResponse, CreateCollection, CreateShardKeyRequest,
        CreateShardKeyResponse, DeleteCollection, DeleteShardKeyRequest, DeleteShardKeyResponse,
        GetCollectionInfoRequest, GetCollectionInfoResponse, ListAliasesRequest,
        ListAliasesResponse, ListCollectionAliasesRequest, ListCollectionsRequest,
        ListCollectionsResponse, ListShardKeysRequest, ListShardKeysResponse, UpdateCollection,
        UpdateCollectionClusterSetupRequest, UpdateCollectionClusterSetupResponse,
    };
    use tokio::sync::Notify;
    use tonic::transport::Server;
    use tonic::transport::server::TcpIncoming;
    use tonic::{Request, Response, Status};

    pub struct GatedCollections {
        url: String,
        gate: Arc<Notify>,
        observed: Arc<Notify>,
        create_calls: Arc<AtomicUsize>,
        shutdown: Option<tokio::sync::oneshot::Sender<()>>,
        server: tokio::task::JoinHandle<()>,
    }

    impl GatedCollections {
        pub async fn start() -> Self {
            let incoming = TcpIncoming::bind("127.0.0.1:0".parse().expect("bind address"))
                .expect("bind an ephemeral port");
            let address = incoming.local_addr().expect("bound address");
            let gate = Arc::new(Notify::new());
            let observed = Arc::new(Notify::new());
            let create_calls = Arc::new(AtomicUsize::new(0));
            let (shutdown, shutdown_rx) = tokio::sync::oneshot::channel();
            let service = Stub {
                gate: Arc::clone(&gate),
                observed: Arc::clone(&observed),
                create_calls: Arc::clone(&create_calls),
            };
            let server = tokio::spawn(async move {
                Server::builder()
                    .add_service(CollectionsServer::new(service))
                    .serve_with_incoming_shutdown(incoming, async {
                        let _ = shutdown_rx.await;
                    })
                    .await
                    .expect("serve the gated collections stub");
            });
            Self {
                url: format!("http://{address}"),
                gate,
                observed,
                create_calls,
                shutdown: Some(shutdown),
                server,
            }
        }

        pub fn url(&self) -> &str {
            &self.url
        }

        pub fn gate(&self) -> Arc<Notify> {
            Arc::clone(&self.gate)
        }

        pub fn observed(&self) -> Arc<Notify> {
            Arc::clone(&self.observed)
        }

        pub fn create_calls(&self) -> Arc<AtomicUsize> {
            Arc::clone(&self.create_calls)
        }

        pub async fn shutdown(mut self) {
            if let Some(shutdown) = self.shutdown.take() {
                let _ = shutdown.send(());
            }
            let _ = self.server.await;
        }
    }

    struct Stub {
        gate: Arc<Notify>,
        observed: Arc<Notify>,
        create_calls: Arc<AtomicUsize>,
    }

    #[tonic::async_trait]
    impl Collections for Stub {
        async fn list(
            &self,
            _request: Request<ListCollectionsRequest>,
        ) -> Result<Response<ListCollectionsResponse>, Status> {
            Err(Status::unimplemented(
                "the gated stub answers only existence and create",
            ))
        }

        async fn get(
            &self,
            _request: Request<GetCollectionInfoRequest>,
        ) -> Result<Response<GetCollectionInfoResponse>, Status> {
            Err(Status::unimplemented(
                "the gated stub answers only existence and create",
            ))
        }

        async fn collection_exists(
            &self,
            _request: Request<CollectionExistsRequest>,
        ) -> Result<Response<CollectionExistsResponse>, Status> {
            self.observed.notify_one();
            self.gate.notified().await;
            Ok(Response::new(CollectionExistsResponse {
                result: Some(CollectionExists { exists: false }),
                time: 0.0,
            }))
        }

        async fn collection_cluster_info(
            &self,
            _request: Request<CollectionClusterInfoRequest>,
        ) -> Result<Response<CollectionClusterInfoResponse>, Status> {
            Err(Status::unimplemented(
                "the gated stub answers only existence and create",
            ))
        }

        async fn list_aliases(
            &self,
            _request: Request<ListAliasesRequest>,
        ) -> Result<Response<ListAliasesResponse>, Status> {
            Err(Status::unimplemented(
                "the gated stub answers only existence and create",
            ))
        }

        async fn list_collection_aliases(
            &self,
            _request: Request<ListCollectionAliasesRequest>,
        ) -> Result<Response<ListAliasesResponse>, Status> {
            Err(Status::unimplemented(
                "the gated stub answers only existence and create",
            ))
        }

        async fn list_shard_keys(
            &self,
            _request: Request<ListShardKeysRequest>,
        ) -> Result<Response<ListShardKeysResponse>, Status> {
            Err(Status::unimplemented(
                "the gated stub answers only existence and create",
            ))
        }

        async fn create(
            &self,
            _request: Request<CreateCollection>,
        ) -> Result<Response<CollectionOperationResponse>, Status> {
            self.create_calls.fetch_add(1, Ordering::AcqRel);
            Ok(Response::new(CollectionOperationResponse {
                result: true,
                time: 0.0,
            }))
        }

        async fn update(
            &self,
            _request: Request<UpdateCollection>,
        ) -> Result<Response<CollectionOperationResponse>, Status> {
            Err(Status::unimplemented(
                "the gated stub answers only existence and create",
            ))
        }

        async fn delete(
            &self,
            _request: Request<DeleteCollection>,
        ) -> Result<Response<CollectionOperationResponse>, Status> {
            Err(Status::unimplemented(
                "the gated stub answers only existence and create",
            ))
        }

        async fn update_aliases(
            &self,
            _request: Request<ChangeAliases>,
        ) -> Result<Response<CollectionOperationResponse>, Status> {
            Err(Status::unimplemented(
                "the gated stub answers only existence and create",
            ))
        }

        async fn update_collection_cluster_setup(
            &self,
            _request: Request<UpdateCollectionClusterSetupRequest>,
        ) -> Result<Response<UpdateCollectionClusterSetupResponse>, Status> {
            Err(Status::unimplemented(
                "the gated stub answers only existence and create",
            ))
        }

        async fn create_shard_key(
            &self,
            _request: Request<CreateShardKeyRequest>,
        ) -> Result<Response<CreateShardKeyResponse>, Status> {
            Err(Status::unimplemented(
                "the gated stub answers only existence and create",
            ))
        }

        async fn delete_shard_key(
            &self,
            _request: Request<DeleteShardKeyRequest>,
        ) -> Result<Response<DeleteShardKeyResponse>, Status> {
            Err(Status::unimplemented(
                "the gated stub answers only existence and create",
            ))
        }
    }
}

#[tokio::test]
async fn a_cancelled_authority_during_existence_discovery_starts_no_create() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    let mock = gated_collections::GatedCollections::start().await;
    let config = crate::config::QdrantConfig {
        url: mock.url().to_string(),
        collection_name: "gated-base".to_string(),
        recreate_on_dimension_mismatch: false,
    };
    let cancelled = Arc::new(AtomicBool::new(false));
    let authority_cancelled = Arc::clone(&cancelled);
    let gate = mock.gate();
    let observed = mock.observed();
    let create_calls = mock.create_calls();

    let handle = tokio::spawn(async move {
        super::QdrantIndex::prepare_rebuild_target(&config, "gated-target", 8, false, move || {
            let cancelled = Arc::clone(&authority_cancelled);
            async move {
                if cancelled.load(Ordering::Acquire) {
                    Err(anyhow::anyhow!("the owner cancelled before the create"))
                } else {
                    Ok(())
                }
            }
        })
        .await
    });

    observed.notified().await;
    cancelled.store(true, Ordering::Release);
    gate.notify_one();

    let result = handle.await.expect("join the preparation");
    assert!(
        result.is_err(),
        "the cancelled authority refuses target preparation"
    );
    assert_eq!(
        create_calls.load(Ordering::Acquire),
        0,
        "a cancelled authority starts no create RPC"
    );
    mock.shutdown().await;
}

#[test]
fn the_code_collection_indexes_canonical_value_types() {
    let types = CODE_PAYLOAD_FIELD_INDEXES
        .iter()
        .map(|(field, field_type)| (*field, *field_type))
        .collect::<Vec<_>>();
    for keyword in [
        "repository_key",
        "generation_key",
        "file_key",
        "path",
        "language",
    ] {
        assert!(
            types.contains(&(keyword, FieldType::Keyword)),
            "{keyword} must be a keyword index"
        );
    }
    for integer in ["group_id", "chunk_index", "start_line", "end_line"] {
        assert!(
            types.contains(&(integer, FieldType::Integer)),
            "{integer} must be an integer index"
        );
    }
    assert!(
        !CODE_PAYLOAD_FIELD_INDEXES
            .iter()
            .any(|(field, _)| *field == "chunk_key"),
        "the point id carries the chunk key; no redundant filter index is guessed"
    );
}
