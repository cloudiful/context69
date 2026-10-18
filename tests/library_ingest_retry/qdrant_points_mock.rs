//! Minimal in-process Qdrant Points service for the dependency-wait and
//! source-deletion fixtures.
//!
//! Only `delete` answers successfully: the fixtures publish a pair whose
//! collection is the no-network `test-noop` collection, so upserts short-circuit
//! in the index while a real delete is still issued and recorded. Every other
//! RPC is `unimplemented` and would fail the test.
//!
//! The server binds an ephemeral loopback port and is dropped with the test, so
//! no case can reach an external service or a real Qdrant deployment.

use qdrant_client::qdrant::points_server::{Points, PointsServer};
use qdrant_client::qdrant::{
    ClearPayloadPoints, CountPoints, CountResponse, CreateFieldIndexCollection,
    CreateVectorNameRequest, DeleteFieldIndexCollection, DeletePayloadPoints, DeletePointVectors,
    DeletePoints, DeleteVectorNameRequest, DiscoverBatchPoints, DiscoverBatchResponse,
    DiscoverPoints, DiscoverResponse, FacetCounts, FacetResponse, GetPoints, GetResponse,
    PointsOperationResponse, QueryBatchPoints, QueryBatchResponse, QueryGroupsResponse,
    QueryPointGroups, QueryPoints, QueryResponse, RecommendBatchPoints, RecommendBatchResponse,
    RecommendGroupsResponse, RecommendPointGroups, RecommendPoints, RecommendResponse,
    ScrollPoints, ScrollResponse, SearchBatchPoints, SearchBatchResponse, SearchGroupsResponse,
    SearchMatrixOffsetsResponse, SearchMatrixPairsResponse, SearchMatrixPoints, SearchPointGroups,
    SearchPoints, SearchResponse, SetPayloadPoints, UpdateBatchPoints, UpdateBatchResponse,
    UpdatePointVectors, UpdateResult, UpdateStatus, UpsertPoints,
};
use std::sync::{Arc, Mutex};

use qdrant_client::qdrant::point_id::PointIdOptions;
use qdrant_client::qdrant::points_selector::PointsSelectorOneOf;
use tonic::transport::Server;
use tonic::transport::server::TcpIncoming;
use tonic::{Request, Response, Status};

/// A running Points mock, dropped with the test.
pub struct QdrantPointsMock {
    url: String,
    deletes: Arc<Mutex<Vec<DeleteCall>>>,
    shutdown: Option<tokio::sync::oneshot::Sender<()>>,
    server: tokio::task::JoinHandle<()>,
}

/// One recorded `delete` request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeleteCall {
    /// The point ids the request named, in request order.
    pub point_ids: Vec<String>,
    /// Whether the request selected points by filter instead of by id.
    pub filter: bool,
}

impl QdrantPointsMock {
    /// Serve on loopback until the test ends.
    pub async fn start() -> Self {
        let incoming = TcpIncoming::bind("127.0.0.1:0".parse().expect("a valid bind address"))
            .expect("an ephemeral loopback port must be available");
        let address = incoming
            .local_addr()
            .expect("a bound incoming must report its address");
        let deletes = Arc::new(Mutex::new(Vec::new()));
        let (shutdown, shutdown_rx) = tokio::sync::oneshot::channel();
        let service = PointsMock {
            deletes: Arc::clone(&deletes),
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
            deletes,
            shutdown: Some(shutdown),
            server,
        }
    }

    /// The URL the fixture hands to `QdrantIndex`.
    pub fn url(&self) -> &str {
        &self.url
    }

    /// Every `delete` request the mock answered.
    pub fn delete_calls(&self) -> Vec<DeleteCall> {
        self.deletes
            .lock()
            .expect("the delete log must be readable")
            .clone()
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

struct PointsMock {
    deletes: Arc<Mutex<Vec<DeleteCall>>>,
}

#[tonic::async_trait]
impl Points for PointsMock {
    async fn upsert(
        &self,
        _request: Request<UpsertPoints>,
    ) -> Result<Response<PointsOperationResponse>, Status> {
        Err(Status::unimplemented("upsert"))
    }
    async fn delete(
        &self,
        request: Request<DeletePoints>,
    ) -> Result<Response<PointsOperationResponse>, Status> {
        let call = match request.into_inner().points {
            Some(selector) => match selector.points_selector_one_of {
                Some(PointsSelectorOneOf::Points(list)) => DeleteCall {
                    point_ids: list
                        .ids
                        .into_iter()
                        .map(|id| match id.point_id_options {
                            Some(PointIdOptions::Uuid(value)) => value,
                            Some(PointIdOptions::Num(value)) => value.to_string(),
                            None => String::new(),
                        })
                        .collect(),
                    filter: false,
                },
                Some(PointsSelectorOneOf::Filter(_)) | None => DeleteCall {
                    point_ids: Vec::new(),
                    filter: true,
                },
            },
            None => DeleteCall {
                point_ids: Vec::new(),
                filter: true,
            },
        };
        self.deletes
            .lock()
            .expect("the delete log must be writable")
            .push(call);
        Ok(Response::new(PointsOperationResponse {
            result: Some(UpdateResult {
                operation_id: Some(0),
                status: UpdateStatus::Completed as i32,
            }),
            time: 0.0,
            usage: None,
        }))
    }
    async fn get(&self, _request: Request<GetPoints>) -> Result<Response<GetResponse>, Status> {
        Err(Status::unimplemented("get"))
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
