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
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};

use qdrant_client::qdrant::condition::ConditionOneOf;
use qdrant_client::qdrant::r#match::MatchValue;
use qdrant_client::qdrant::point_id::PointIdOptions;
use qdrant_client::qdrant::points_selector::PointsSelectorOneOf;
use qdrant_client::qdrant::points_server::{Points, PointsServer};
use qdrant_client::qdrant::points_update_operation;
use qdrant_client::qdrant::{
    ClearPayloadPoints, CountPoints, CountResponse, CreateFieldIndexCollection,
    CreateVectorNameRequest, DeleteFieldIndexCollection, DeletePayloadPoints, DeletePointVectors,
    DeletePoints, DeleteVectorNameRequest, DiscoverBatchPoints, DiscoverBatchResponse,
    DiscoverPoints, DiscoverResponse, FacetCounts, FacetResponse, Filter, GetPoints, GetResponse,
    PointId, PointsOperationResponse, QueryBatchPoints, QueryBatchResponse, QueryGroupsResponse,
    QueryPointGroups, QueryPoints, QueryResponse, RecommendBatchPoints, RecommendBatchResponse,
    RecommendGroupsResponse, RecommendPointGroups, RecommendPoints, RecommendResponse,
    RetrievedPoint, ScrollPoints, ScrollResponse, SearchBatchPoints, SearchBatchResponse,
    SearchGroupsResponse, SearchMatrixOffsetsResponse, SearchMatrixPairsResponse,
    SearchMatrixPoints, SearchPointGroups, SearchPoints, SearchResponse, SetPayloadPoints,
    UpdateBatchPoints, UpdateBatchResponse, UpdatePointVectors, UpdateResult, UpdateStatus,
    UpsertPoints, Value,
};
use tokio::sync::Semaphore;
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
    /// The collection named by the request.
    pub collection: String,
    /// The field selector the request carried, when it deleted by filter.
    pub filter: Option<(String, String)>,
}

/// One point update of a recorded `SetPayload` request.
#[derive(Debug, Clone)]
pub struct PayloadUpdate {
    pub point_id: String,
    pub payload: HashMap<String, Value>,
}

/// One recorded payload update request (`set_payload` or `update_batch`).
#[derive(Debug, Clone)]
pub struct SetPayloadCall {
    pub collection: String,
    pub updates: Vec<PayloadUpdate>,
}

#[derive(Default)]
struct PointsState {
    points: HashMap<String, StoredPoint>,
    upserts: Vec<UpsertCall>,
    deletes: Vec<DeleteCall>,
    set_payloads: Vec<SetPayloadCall>,
    fail_deletes: bool,
    fail_set_payloads: usize,
    set_payload_gate: Option<Arc<Semaphore>>,
    set_payload_entered: Arc<AtomicUsize>,
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

    /// Every payload update request the mock answered.
    pub fn set_payload_calls(&self) -> Vec<SetPayloadCall> {
        self.state
            .lock()
            .expect("the payload log must be readable")
            .set_payloads
            .clone()
    }

    /// Hold the next payload update until the returned semaphore is released.
    pub fn pause_set_payload(&self) -> Arc<Semaphore> {
        let gate = Arc::new(Semaphore::new(0));
        self.state
            .lock()
            .expect("the store must be writable")
            .set_payload_gate = Some(gate.clone());
        gate
    }

    /// Stop pausing later payload updates.
    pub fn resume_set_payload(&self) {
        self.state
            .lock()
            .expect("the store must be writable")
            .set_payload_gate = None;
    }

    /// How many payload update handlers have entered (and not yet left) the
    /// pause gate. A test uses this instead of a fixed sleep.
    pub fn set_payload_entered(&self) -> usize {
        self.state
            .lock()
            .expect("the store must be readable")
            .set_payload_entered
            .load(Ordering::SeqCst)
    }

    /// Make the next payload update fail once.
    pub fn fail_set_payload_once(&self) {
        self.state
            .lock()
            .expect("the store must be writable")
            .fail_set_payloads = 1;
    }

    /// The number of configured one-shot payload failures still pending.
    pub fn pending_payload_failures(&self) -> usize {
        self.state
            .lock()
            .expect("the store must be readable")
            .fail_set_payloads
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
        self.insert_preexisting_payload(id, payload, dimensions);
    }

    /// Inserts a pre-existing library point that a file-scoped delete selects
    /// by its `library_file_id` payload.
    pub fn insert_preexisting_library_point(
        &self,
        id: &str,
        file_id: uuid::Uuid,
        document_id: i64,
        record_hash: &str,
        dimensions: usize,
    ) {
        let payload = HashMap::from([
            (
                "library_file_id".to_string(),
                string_value(&file_id.to_string()),
            ),
            ("document_id".to_string(), integer_value(document_id)),
            ("record_hash".to_string(), string_value(record_hash)),
        ]);
        self.insert_preexisting_payload(id, payload, dimensions);
    }

    /// Inserts a point with an explicit payload.
    pub fn insert_preexisting_payload(
        &self,
        id: &str,
        payload: HashMap<String, Value>,
        dimensions: usize,
    ) {
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

fn value_string(value: &Value) -> Option<String> {
    match value.kind.as_ref()? {
        qdrant_client::qdrant::value::Kind::StringValue(value) => Some(value.clone()),
        _ => None,
    }
}

fn value_integer(value: &Value) -> Option<i64> {
    match value.kind.as_ref()? {
        qdrant_client::qdrant::value::Kind::IntegerValue(value) => Some(*value),
        _ => None,
    }
}

/// Read one string field of a struct payload value, for test assertions.
pub fn struct_string_field(value: &Value, field: &str) -> Option<String> {
    match value.kind.as_ref()? {
        qdrant_client::qdrant::value::Kind::StructValue(metadata) => {
            match metadata.fields.get(field)?.kind.as_ref()? {
                qdrant_client::qdrant::value::Kind::StringValue(value) => Some(value.clone()),
                _ => None,
            }
        }
        _ => None,
    }
}

fn selector_point_ids(selector: Option<qdrant_client::qdrant::PointsSelector>) -> Vec<String> {
    match selector.and_then(|selector| selector.points_selector_one_of) {
        Some(PointsSelectorOneOf::Points(list)) => list.ids.iter().map(point_id_string).collect(),
        _ => Vec::new(),
    }
}

fn filter_match(filter: &Filter, payload: &HashMap<String, Value>) -> Option<(String, String)> {
    let mut recorded = None;
    for condition in &filter.must {
        let Some(ConditionOneOf::Field(field)) = condition.condition_one_of.as_ref() else {
            return None;
        };
        let matched = match field.r#match.as_ref()?.match_value.as_ref()? {
            MatchValue::Keyword(value) => {
                recorded.get_or_insert_with(|| (field.key.clone(), value.clone()));
                payload.get(&field.key).and_then(value_string).as_deref() == Some(value.as_str())
            }
            MatchValue::Integer(value) => {
                recorded.get_or_insert_with(|| (field.key.clone(), value.to_string()));
                payload.get(&field.key).and_then(value_integer) == Some(*value)
            }
            _ => return None,
        };
        if !matched {
            return None;
        }
    }
    recorded
}

fn apply_payload_updates(
    state: &mut PointsState,
    ids: Vec<String>,
    payload: &HashMap<String, Value>,
) -> Vec<PayloadUpdate> {
    let mut updates = Vec::with_capacity(ids.len());
    for id in ids {
        if let Some(point) = state.points.get_mut(&id) {
            for (key, value) in payload {
                point.payload.insert(key.clone(), value.clone());
            }
        }
        updates.push(PayloadUpdate {
            point_id: id,
            payload: payload.clone(),
        });
    }
    updates
}

fn apply_payload(
    state: &mut PointsState,
    collection: &str,
    ids: Vec<String>,
    payload: HashMap<String, Value>,
) {
    let updates = apply_payload_updates(state, ids, &payload);
    state.set_payloads.push(SetPayloadCall {
        collection: collection.to_string(),
        updates,
    });
}

fn completed() -> Response<PointsOperationResponse> {
    Response::new(PointsOperationResponse {
        result: Some(UpdateResult {
            operation_id: Some(0),
            status: UpdateStatus::Completed as i32,
        }),
        time: 0.0,
        usage: None,
    })
}

struct PointsMock {
    state: Arc<Mutex<PointsState>>,
}

impl PointsMock {
    /// Enter the optional pause gate and apply the one-shot failure. The state
    /// lock is never held across the wait.
    async fn await_payload_gate(&self) -> Result<(), Status> {
        let gate = {
            let state = self.state.lock().expect("the store must be readable");
            state.set_payload_entered.fetch_add(1, Ordering::SeqCst);
            state.set_payload_gate.clone()
        };
        if let Some(gate) = gate {
            let permit = gate
                .acquire()
                .await
                .map_err(|_| Status::internal("mock payload gate closed"))?;
            drop(permit);
        }
        let mut state = self.state.lock().expect("the store must be writable");
        state.set_payload_entered.fetch_sub(1, Ordering::SeqCst);
        if state.fail_set_payloads > 0 {
            state.fail_set_payloads -= 1;
            // A non-retryable status so the qdrant client's transparent
            // internal/unavailable retry cannot consume and then hide the
            // configured one-shot failure.
            return Err(Status::invalid_argument("mock payload failure"));
        }
        Ok(())
    }
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
        let mut state = self.state.lock().expect("the store must be writable");
        if state.fail_deletes {
            return Err(Status::internal("mock delete failure"));
        }
        let mut filter = None;
        let ids: Vec<String> = match request.points {
            Some(selector) => match selector.points_selector_one_of {
                Some(PointsSelectorOneOf::Points(list)) => {
                    list.ids.iter().map(point_id_string).collect()
                }
                Some(PointsSelectorOneOf::Filter(selector)) => state
                    .points
                    .iter()
                    .filter_map(|(id, point)| {
                        let matched = filter_match(&selector, &point.payload);
                        if matched.is_some() {
                            filter = matched;
                            Some(id.clone())
                        } else {
                            None
                        }
                    })
                    .collect(),
                _ => Vec::new(),
            },
            None => Vec::new(),
        };
        for id in &ids {
            state.points.remove(id);
        }
        state.deletes.push(DeleteCall {
            point_ids: ids,
            collection: request.collection_name,
            filter,
        });
        Ok(completed())
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
        request: Request<SetPayloadPoints>,
    ) -> Result<Response<PointsOperationResponse>, Status> {
        let request = request.into_inner();
        self.await_payload_gate().await?;
        let ids = selector_point_ids(request.points_selector);
        let mut state = self.state.lock().expect("the store must be writable");
        apply_payload(&mut state, &request.collection_name, ids, request.payload);
        Ok(completed())
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
        request: Request<UpdateBatchPoints>,
    ) -> Result<Response<UpdateBatchResponse>, Status> {
        let request = request.into_inner();
        self.await_payload_gate().await?;
        let mut state = self.state.lock().expect("the store must be writable");
        let mut updates = Vec::new();
        for operation in request.operations {
            if let Some(points_update_operation::Operation::SetPayload(set)) = operation.operation {
                let ids = selector_point_ids(set.points_selector);
                updates.extend(apply_payload_updates(&mut state, ids, &set.payload));
            }
        }
        state.set_payloads.push(SetPayloadCall {
            collection: request.collection_name,
            updates,
        });
        Ok(Response::new(UpdateBatchResponse {
            result: vec![UpdateResult {
                operation_id: Some(0),
                status: UpdateStatus::Completed as i32,
            }],
            time: 0.0,
            usage: None,
        }))
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
