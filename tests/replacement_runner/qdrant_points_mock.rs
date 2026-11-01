//! The in-process Qdrant Points mock of the replacement runner tests.
//!
//! The mock answers exactly the three RPCs the staged replacement executor
//! issues: `upsert` (with recorded insert-only mode and payload preservation),
//! `get` (payload reads for the point verification) and `delete` (the physical
//! tombstone cleanup). Every other RPC stays `unimplemented`, so a runner that
//! reached for another write path would fail the test instead of silently
//! succeeding.
//!
//! The server binds an ephemeral loopback port and is dropped with the test,
//! so no case can reach an external service or a real Qdrant deployment.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use qdrant_client::qdrant::point_id::PointIdOptions;
use qdrant_client::qdrant::points_selector::PointsSelectorOneOf;
use qdrant_client::qdrant::points_server::{Points, PointsServer};
use qdrant_client::qdrant::{
    ClearPayloadPoints, CountPoints, CountResponse, CreateFieldIndexCollection,
    CreateVectorNameRequest, DeleteFieldIndexCollection, DeletePayloadPoints, DeletePointVectors,
    DeletePoints, DeleteVectorNameRequest, DiscoverBatchPoints, DiscoverBatchResponse,
    DiscoverPoints, DiscoverResponse, FacetCounts, FacetResponse, GetPoints, GetResponse, PointId,
    PointsOperationResponse, QueryBatchPoints, QueryBatchResponse, QueryGroupsResponse,
    QueryPointGroups, QueryPoints, QueryResponse, RecommendBatchPoints, RecommendBatchResponse,
    RecommendGroupsResponse, RecommendPointGroups, RecommendPoints, RecommendResponse,
    RetrievedPoint, ScrollPoints, ScrollResponse, SearchBatchPoints, SearchBatchResponse,
    SearchGroupsResponse, SearchMatrixOffsetsResponse, SearchMatrixPairsResponse,
    SearchMatrixPoints, SearchPointGroups, SearchPoints, SearchResponse, SetPayloadPoints,
    UpdateBatchPoints, UpdateBatchResponse, UpdatePointVectors, UpdateResult, UpdateStatus,
    UpsertPoints, Value,
};
use tonic::transport::Server;
use tonic::transport::server::TcpIncoming;
use tonic::{Request, Response, Status};

/// One stored point of the mock collection.
#[derive(Debug, Clone)]
pub struct StoredPoint {
    pub payload: HashMap<String, Value>,
    pub vector: Vec<f32>,
}

/// One recorded `upsert` request.
#[derive(Debug, Clone)]
pub struct UpsertCall {
    /// The point ids the request carried, in request order.
    pub point_ids: Vec<String>,
    /// Whether the request asked for insert-only mode.
    pub insert_only: bool,
    /// The requested points, keyed by id.
    pub points: HashMap<String, StoredPoint>,
}

/// One recorded `delete` request.
#[derive(Debug, Clone)]
pub struct DeleteCall {
    pub point_ids: Vec<String>,
}

#[derive(Default)]
struct PointsState {
    points: HashMap<String, StoredPoint>,
    upserts: Vec<UpsertCall>,
    deletes: Vec<DeleteCall>,
    fail_deletes: bool,
    /// Ids the `get` handler hides, as if a concurrent writer had removed
    /// them between the staged write and the verification read.
    omit_from_get: std::collections::BTreeSet<String>,
}

/// A running Points mock, dropped with the test.
pub struct QdrantPointsMock {
    url: String,
    state: Arc<Mutex<PointsState>>,
    shutdown: Option<tokio::sync::oneshot::Sender<()>>,
    server: tokio::task::JoinHandle<()>,
}

impl QdrantPointsMock {
    /// Serve on loopback until the test ends.
    pub async fn start() -> Self {
        let incoming = TcpIncoming::bind("127.0.0.1:0".parse().expect("a valid bind address"))
            .expect("an ephemeral loopback port must be available");
        let address = incoming
            .local_addr()
            .expect("a bound incoming must report its address");
        let state = Arc::new(Mutex::new(PointsState::default()));
        let (shutdown, shutdown_rx) = tokio::sync::oneshot::channel();
        let service = PointsMock {
            state: Arc::clone(&state),
        };
        let server = tokio::spawn(async move {
            let served = Server::builder()
                .add_service(PointsServer::new(service))
                .serve_with_incoming_shutdown(incoming, async {
                    let _ = shutdown_rx.await;
                });
            served.await.expect("the mock gRPC server must serve");
        });
        Self {
            url: format!("http://{address}"),
            state,
            shutdown: Some(shutdown),
            server,
        }
    }

    /// The URL the fixture hands to `QdrantIndex`.
    pub fn url(&self) -> &str {
        &self.url
    }

    /// Every `upsert` request the mock answered.
    pub fn upsert_calls(&self) -> Vec<UpsertCall> {
        self.state
            .lock()
            .expect("the upsert log must be readable")
            .upserts
            .clone()
    }

    /// Every `delete` request the mock answered.
    pub fn delete_calls(&self) -> Vec<DeleteCall> {
        self.state
            .lock()
            .expect("the delete log must be readable")
            .deletes
            .clone()
    }

    /// The ids currently stored in the mock collection.
    pub fn stored_point_ids(&self) -> Vec<String> {
        let mut ids: Vec<String> = self
            .state
            .lock()
            .expect("the store must be readable")
            .points
            .keys()
            .cloned()
            .collect();
        ids.sort();
        ids
    }

    /// The stored point of one id.
    pub fn stored_point(&self, id: &str) -> Option<StoredPoint> {
        self.state
            .lock()
            .expect("the store must be readable")
            .points
            .get(id)
            .cloned()
    }

    /// Removes stored points, as if another writer had deleted them.
    pub fn remove_points(&self, ids: &[String]) {
        let mut state = self.state.lock().expect("the store must be writable");
        for id in ids {
            state.points.remove(id);
        }
    }

    /// Inserts a pre-existing point the mock never received through `upsert`.
    ///
    /// Used to model a foreign or stale point that already owns an id before
    /// the replacement stages.
    pub fn insert_preexisting(
        &self,
        id: &str,
        document_id: i64,
        record_hash: &str,
        dimensions: usize,
    ) {
        let payload = HashMap::from([
            ("document_id".to_string(), integer_value(document_id)),
            ("record_hash".to_string(), string_value(record_hash)),
        ]);
        let mut state = self.state.lock().expect("the store must be writable");
        state.points.insert(
            id.to_string(),
            StoredPoint {
                payload,
                vector: vec![0.0; dimensions],
            },
        );
    }

    /// Makes every subsequent `delete` fail, as a broken physical cleanup.
    pub fn set_fail_deletes(&self, fail: bool) {
        self.state
            .lock()
            .expect("the store must be writable")
            .fail_deletes = fail;
    }

    /// Hides ids from the `get` handler, as if the point had vanished after
    /// the upsert; the stored point itself is untouched.
    pub fn omit_from_get(&self, ids: &[String]) {
        self.state
            .lock()
            .expect("the store must be writable")
            .omit_from_get
            .extend(ids.iter().cloned());
    }
}

impl Drop for QdrantPointsMock {
    fn drop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        self.server.abort();
    }
}

fn integer_value(value: i64) -> Value {
    Value {
        kind: Some(qdrant_client::qdrant::value::Kind::IntegerValue(value)),
    }
}

fn string_value(value: &str) -> Value {
    Value {
        kind: Some(qdrant_client::qdrant::value::Kind::StringValue(
            value.to_string(),
        )),
    }
}

fn point_id_string(id: &PointId) -> String {
    match id.point_id_options.as_ref() {
        Some(PointIdOptions::Uuid(value)) => value.clone(),
        Some(PointIdOptions::Num(value)) => value.to_string(),
        None => String::new(),
    }
}

struct PointsMock {
    state: Arc<Mutex<PointsState>>,
}

#[tonic::async_trait]
impl Points for PointsMock {
    async fn upsert(
        &self,
        request: Request<UpsertPoints>,
    ) -> Result<Response<PointsOperationResponse>, Status> {
        let request = request.into_inner();
        let insert_only =
            request.update_mode == Some(qdrant_client::qdrant::UpdateMode::InsertOnly as i32);
        let requested: HashMap<String, StoredPoint> = request
            .points
            .into_iter()
            .filter_map(|point| {
                let id = point.id.as_ref().map(point_id_string)?;
                Some((
                    id,
                    StoredPoint {
                        payload: point.payload,
                        vector: point
                            .vectors
                            .and_then(|vectors| vectors.vectors_options)
                            .map(|vectors| match vectors {
                                qdrant_client::qdrant::vectors::VectorsOptions::Vector(vector) => {
                                    vector
                                        .vector
                                        .map(|vector| match vector {
                                            qdrant_client::qdrant::vector::Vector::Dense(dense) => {
                                                dense.data
                                            }
                                            _ => Vec::new(),
                                        })
                                        .unwrap_or_default()
                                }
                                _ => Vec::new(),
                            })
                            .unwrap_or_default(),
                    },
                ))
            })
            .collect();
        let call = UpsertCall {
            point_ids: requested.keys().cloned().collect(),
            insert_only,
            points: requested,
        };
        let mut state = self.state.lock().expect("the store must be writable");
        for (id, point) in &call.points {
            // Insert-only semantics: an existing id keeps its stored payload
            // and vector; only absent ids are inserted.
            if !(insert_only && state.points.contains_key(id)) {
                state.points.insert(id.clone(), point.clone());
            }
        }
        state.upserts.push(call);
        Ok(Response::new(PointsOperationResponse {
            result: Some(UpdateResult {
                operation_id: Some(0),
                status: UpdateStatus::Completed as i32,
            }),
            time: 0.0,
            usage: None,
        }))
    }

    async fn get(&self, request: Request<GetPoints>) -> Result<Response<GetResponse>, Status> {
        let request = request.into_inner();
        let ids: Vec<String> = request.ids.iter().map(point_id_string).collect();
        let state = self.state.lock().expect("the store must be readable");
        let points = ids
            .into_iter()
            .filter(|id| !state.omit_from_get.contains(id))
            .filter_map(|id| {
                state.points.get(&id).map(|stored| RetrievedPoint {
                    id: Some(PointId {
                        point_id_options: Some(PointIdOptions::Uuid(id)),
                    }),
                    payload: stored.payload.clone(),
                    vectors: None,
                    shard_key: None,
                    order_value: None,
                })
            })
            .collect();
        Ok(Response::new(GetResponse {
            result: points,
            time: 0.0,
            usage: None,
        }))
    }

    async fn delete(
        &self,
        request: Request<DeletePoints>,
    ) -> Result<Response<PointsOperationResponse>, Status> {
        let request = request.into_inner();
        let ids: Vec<String> = match request.points {
            Some(selector) => match selector.points_selector_one_of {
                Some(PointsSelectorOneOf::Points(list)) => {
                    list.ids.iter().map(point_id_string).collect()
                }
                _ => Vec::new(),
            },
            None => Vec::new(),
        };
        let mut state = self.state.lock().expect("the store must be writable");
        if state.fail_deletes {
            return Err(Status::internal("mock delete failure"));
        }
        for id in &ids {
            state.points.remove(id);
        }
        state.deletes.push(DeleteCall { point_ids: ids });
        Ok(Response::new(PointsOperationResponse {
            result: Some(UpdateResult {
                operation_id: Some(0),
                status: UpdateStatus::Completed as i32,
            }),
            time: 0.0,
            usage: None,
        }))
    }

    async fn update_vectors(
        &self,
        _request: Request<UpdatePointVectors>,
    ) -> Result<Response<PointsOperationResponse>, Status> {
        Err(Status::unimplemented("update_vectors"))
    }
    async fn delete_vectors(
        &self,
        _request: Request<DeletePointVectors>,
    ) -> Result<Response<PointsOperationResponse>, Status> {
        Err(Status::unimplemented("delete_vectors"))
    }
    async fn set_payload(
        &self,
        _request: Request<SetPayloadPoints>,
    ) -> Result<Response<PointsOperationResponse>, Status> {
        Err(Status::unimplemented("set_payload"))
    }
    async fn overwrite_payload(
        &self,
        _request: Request<SetPayloadPoints>,
    ) -> Result<Response<PointsOperationResponse>, Status> {
        Err(Status::unimplemented("overwrite_payload"))
    }
    async fn delete_payload(
        &self,
        _request: Request<DeletePayloadPoints>,
    ) -> Result<Response<PointsOperationResponse>, Status> {
        Err(Status::unimplemented("delete_payload"))
    }
    async fn clear_payload(
        &self,
        _request: Request<ClearPayloadPoints>,
    ) -> Result<Response<PointsOperationResponse>, Status> {
        Err(Status::unimplemented("clear_payload"))
    }
    async fn create_field_index(
        &self,
        _request: Request<CreateFieldIndexCollection>,
    ) -> Result<Response<PointsOperationResponse>, Status> {
        Err(Status::unimplemented("create_field_index"))
    }
    async fn delete_field_index(
        &self,
        _request: Request<DeleteFieldIndexCollection>,
    ) -> Result<Response<PointsOperationResponse>, Status> {
        Err(Status::unimplemented("delete_field_index"))
    }
    async fn create_vector_name(
        &self,
        _request: Request<CreateVectorNameRequest>,
    ) -> Result<Response<PointsOperationResponse>, Status> {
        Err(Status::unimplemented("create_vector_name"))
    }
    async fn delete_vector_name(
        &self,
        _request: Request<DeleteVectorNameRequest>,
    ) -> Result<Response<PointsOperationResponse>, Status> {
        Err(Status::unimplemented("delete_vector_name"))
    }
    async fn search(
        &self,
        _request: Request<SearchPoints>,
    ) -> Result<Response<SearchResponse>, Status> {
        Err(Status::unimplemented("search"))
    }
    async fn search_batch(
        &self,
        _request: Request<SearchBatchPoints>,
    ) -> Result<Response<SearchBatchResponse>, Status> {
        Err(Status::unimplemented("search_batch"))
    }
    async fn search_groups(
        &self,
        _request: Request<SearchPointGroups>,
    ) -> Result<Response<SearchGroupsResponse>, Status> {
        Err(Status::unimplemented("search_groups"))
    }
    async fn scroll(
        &self,
        _request: Request<ScrollPoints>,
    ) -> Result<Response<ScrollResponse>, Status> {
        Err(Status::unimplemented("scroll"))
    }
    async fn recommend(
        &self,
        _request: Request<RecommendPoints>,
    ) -> Result<Response<RecommendResponse>, Status> {
        Err(Status::unimplemented("recommend"))
    }
    async fn recommend_batch(
        &self,
        _request: Request<RecommendBatchPoints>,
    ) -> Result<Response<RecommendBatchResponse>, Status> {
        Err(Status::unimplemented("recommend_batch"))
    }
    async fn recommend_groups(
        &self,
        _request: Request<RecommendPointGroups>,
    ) -> Result<Response<RecommendGroupsResponse>, Status> {
        Err(Status::unimplemented("recommend_groups"))
    }
    async fn discover(
        &self,
        _request: Request<DiscoverPoints>,
    ) -> Result<Response<DiscoverResponse>, Status> {
        Err(Status::unimplemented("discover"))
    }
    async fn discover_batch(
        &self,
        _request: Request<DiscoverBatchPoints>,
    ) -> Result<Response<DiscoverBatchResponse>, Status> {
        Err(Status::unimplemented("discover_batch"))
    }
    async fn count(
        &self,
        _request: Request<CountPoints>,
    ) -> Result<Response<CountResponse>, Status> {
        Err(Status::unimplemented("count"))
    }
    async fn update_batch(
        &self,
        _request: Request<UpdateBatchPoints>,
    ) -> Result<Response<UpdateBatchResponse>, Status> {
        Err(Status::unimplemented("update_batch"))
    }
    async fn query(
        &self,
        _request: Request<QueryPoints>,
    ) -> Result<Response<QueryResponse>, Status> {
        Err(Status::unimplemented("query"))
    }
    async fn query_batch(
        &self,
        _request: Request<QueryBatchPoints>,
    ) -> Result<Response<QueryBatchResponse>, Status> {
        Err(Status::unimplemented("query_batch"))
    }
    async fn query_groups(
        &self,
        _request: Request<QueryPointGroups>,
    ) -> Result<Response<QueryGroupsResponse>, Status> {
        Err(Status::unimplemented("query_groups"))
    }
    async fn facet(
        &self,
        _request: Request<FacetCounts>,
    ) -> Result<Response<FacetResponse>, Status> {
        Err(Status::unimplemented("facet"))
    }
    async fn search_matrix_pairs(
        &self,
        _request: Request<SearchMatrixPoints>,
    ) -> Result<Response<SearchMatrixPairsResponse>, Status> {
        Err(Status::unimplemented("search_matrix_pairs"))
    }
    async fn search_matrix_offsets(
        &self,
        _request: Request<SearchMatrixPoints>,
    ) -> Result<Response<SearchMatrixOffsetsResponse>, Status> {
        Err(Status::unimplemented("search_matrix_offsets"))
    }
}
