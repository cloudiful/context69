//! The in-process Qdrant gRPC mock used by `tests/qdrant_connection_probe.rs`.
//!
//! `qdrant-client` ships the generated Qdrant server stubs alongside its client,
//! so a mock needs no proto build step and no `protoc`: only the `Collections`
//! service has to be implemented. Every method is implemented — including the
//! mutating ones — because the interesting assertion is that the probe reaches
//! only `list`, and a mock that simply lacked those methods could not prove it.
//! The mutating methods record themselves and answer `unimplemented`.
//!
//! The mock binds an ephemeral loopback port and is dropped with the test, so a
//! case can never reach an external service or a real Qdrant deployment.

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

use qdrant_client::qdrant::collections_server::{Collections, CollectionsServer};
use qdrant_client::qdrant::{
    ChangeAliases, CollectionClusterInfoRequest, CollectionClusterInfoResponse,
    CollectionExistsRequest, CollectionExistsResponse, CollectionOperationResponse,
    CreateCollection, CreateShardKeyRequest, CreateShardKeyResponse, DeleteCollection,
    DeleteShardKeyRequest, DeleteShardKeyResponse, GetCollectionInfoRequest,
    GetCollectionInfoResponse, ListAliasesRequest, ListAliasesResponse,
    ListCollectionAliasesRequest, ListCollectionsRequest, ListCollectionsResponse,
    ListShardKeysRequest, ListShardKeysResponse, UpdateCollection,
    UpdateCollectionClusterSetupRequest, UpdateCollectionClusterSetupResponse,
};
use tokio::net::TcpListener;
use tonic::transport::Server;
use tonic::transport::server::TcpIncoming;
use tonic::{Request, Response, Status};

/// A running mock, dropped with the test.
pub struct QdrantMock {
    url: String,
    calls: Arc<Mutex<BTreeSet<&'static str>>>,
    shutdown: Option<tokio::sync::oneshot::Sender<()>>,
    server: tokio::task::JoinHandle<()>,
}

/// How the mock answers `list`.
#[derive(Clone, Copy)]
pub enum ListBehaviour {
    /// Answer with an empty collection list, as a fresh Qdrant would.
    Succeeds,
    /// Refuse the call the way a key-protected Qdrant would.
    Unauthenticated,
    /// Accept the gRPC list request but never produce a response.
    NeverAnswers,
}

/// A loopback TCP listener that accepts connections but never speaks, used to
/// hold a port so a probe runs into a listener that never answers.
///
/// A real `TcpListener` is used rather than a port that is merely closed,
/// because "listens but never answers" is the one shape that exercises the
/// probe's timeout instead of a connection refusal.
pub struct SilentListener {
    url: String,
    accepts: tokio::task::JoinHandle<()>,
}

impl QdrantMock {
    /// Serve on loopback until the test ends.
    pub async fn start(behaviour: ListBehaviour) -> Self {
        let incoming = TcpIncoming::bind("127.0.0.1:0".parse().expect("a valid bind address"))
            .expect("an ephemeral loopback port must be available");
        let address = incoming
            .local_addr()
            .expect("a bound incoming must report its address");
        let calls = Arc::new(Mutex::new(BTreeSet::new()));
        let service = MockCollections {
            behaviour,
            calls: Arc::clone(&calls),
        };
        let (shutdown, shutdown_rx) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let served = Server::builder()
                .add_service(CollectionsServer::new(service))
                .serve_with_incoming_shutdown(incoming, async {
                    let _ = shutdown_rx.await;
                });
            // A mock that failed to serve would make every case hang until the
            // probe timeout instead of failing, so surface it here.
            served.await.expect("the mock gRPC server must serve");
        });

        Self {
            url: format!("http://{address}"),
            calls,
            shutdown: Some(shutdown),
            server,
        }
    }

    /// The URL a probe should be given.
    pub fn url(&self) -> &str {
        &self.url
    }

    /// Every RPC the mock observed, by name.
    pub fn observed_calls(&self) -> BTreeSet<&'static str> {
        self.calls
            .lock()
            .expect("the call log must be readable")
            .clone()
    }

    /// Assert the probe issued `list` and no mutating RPC.
    pub fn assert_only_read_only_calls(&self) {
        let calls = self.observed_calls();
        assert!(
            calls.contains("list"),
            "the probe must ask for the collection list, observed: {calls:?}"
        );
        for mutating in [
            "create",
            "delete",
            "update",
            "update_aliases",
            "update_collection_cluster_setup",
            "create_shard_key",
            "delete_shard_key",
        ] {
            assert!(
                !calls.contains(mutating),
                "the probe must never mutate a collection, but {mutating} was called: {calls:?}"
            );
        }
    }
}

impl Drop for QdrantMock {
    fn drop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        self.server.abort();
    }
}

impl SilentListener {
    /// Bind loopback and accept without ever answering.
    pub async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("an ephemeral loopback port must be available");
        let address = listener
            .local_addr()
            .expect("a bound listener must report its address");
        let accepts = tokio::spawn(async move {
            // Hold the connections open without reading or writing: the probe
            // must give up on its own bounded deadline.
            loop {
                if listener.accept().await.is_err() {
                    break;
                }
            }
        });

        Self {
            url: format!("http://{address}"),
            accepts,
        }
    }

    /// The URL a probe should be given.
    pub fn url(&self) -> &str {
        &self.url
    }
}

impl Drop for SilentListener {
    fn drop(&mut self) {
        self.accepts.abort();
    }
}

struct MockCollections {
    behaviour: ListBehaviour,
    calls: Arc<Mutex<BTreeSet<&'static str>>>,
}

impl MockCollections {
    fn record(&self, call: &'static str) {
        self.calls
            .lock()
            .expect("the call log must be writable")
            .insert(call);
    }
}

/// Why every method but `list` is written out rather than generated.
///
/// The trait is `#[async_trait]`, and an attribute macro cannot see methods
/// produced by a `macro_rules!` expansion, so the bodies have to be spelled out
/// for `#[tonic::async_trait]` to transform them into the boxed-future shape
/// the trait declares.
const NEVER_CALLED: &str = "the Qdrant connection test must never mutate a collection";

#[tonic::async_trait]
impl Collections for MockCollections {
    async fn list(
        &self,
        _request: Request<ListCollectionsRequest>,
    ) -> Result<Response<ListCollectionsResponse>, Status> {
        self.record("list");
        match self.behaviour {
            ListBehaviour::Succeeds => Ok(Response::new(ListCollectionsResponse {
                collections: Vec::new(),
                time: 0.0,
            })),
            ListBehaviour::Unauthenticated => Err(Status::unauthenticated(
                "the mock refuses anonymous calls, as a key-protected Qdrant would",
            )),
            ListBehaviour::NeverAnswers => std::future::pending().await,
        }
    }

    // The reads below are not mutating, but the probe is defined to issue
    // exactly one call, so they record themselves and refuse as well.
    async fn get(
        &self,
        _request: Request<GetCollectionInfoRequest>,
    ) -> Result<Response<GetCollectionInfoResponse>, Status> {
        self.record("get");
        Err(Status::unimplemented(NEVER_CALLED))
    }

    async fn collection_exists(
        &self,
        _request: Request<CollectionExistsRequest>,
    ) -> Result<Response<CollectionExistsResponse>, Status> {
        self.record("collection_exists");
        Err(Status::unimplemented(NEVER_CALLED))
    }

    async fn collection_cluster_info(
        &self,
        _request: Request<CollectionClusterInfoRequest>,
    ) -> Result<Response<CollectionClusterInfoResponse>, Status> {
        self.record("collection_cluster_info");
        Err(Status::unimplemented(NEVER_CALLED))
    }

    async fn list_aliases(
        &self,
        _request: Request<ListAliasesRequest>,
    ) -> Result<Response<ListAliasesResponse>, Status> {
        self.record("list_aliases");
        Err(Status::unimplemented(NEVER_CALLED))
    }

    async fn list_collection_aliases(
        &self,
        _request: Request<ListCollectionAliasesRequest>,
    ) -> Result<Response<ListAliasesResponse>, Status> {
        self.record("list_collection_aliases");
        Err(Status::unimplemented(NEVER_CALLED))
    }

    async fn list_shard_keys(
        &self,
        _request: Request<ListShardKeysRequest>,
    ) -> Result<Response<ListShardKeysResponse>, Status> {
        self.record("list_shard_keys");
        Err(Status::unimplemented(NEVER_CALLED))
    }

    async fn create(
        &self,
        _request: Request<CreateCollection>,
    ) -> Result<Response<CollectionOperationResponse>, Status> {
        self.record("create");
        Err(Status::unimplemented(NEVER_CALLED))
    }

    async fn update(
        &self,
        _request: Request<UpdateCollection>,
    ) -> Result<Response<CollectionOperationResponse>, Status> {
        self.record("update");
        Err(Status::unimplemented(NEVER_CALLED))
    }

    async fn delete(
        &self,
        _request: Request<DeleteCollection>,
    ) -> Result<Response<CollectionOperationResponse>, Status> {
        self.record("delete");
        Err(Status::unimplemented(NEVER_CALLED))
    }

    async fn update_aliases(
        &self,
        _request: Request<ChangeAliases>,
    ) -> Result<Response<CollectionOperationResponse>, Status> {
        self.record("update_aliases");
        Err(Status::unimplemented(NEVER_CALLED))
    }

    async fn update_collection_cluster_setup(
        &self,
        _request: Request<UpdateCollectionClusterSetupRequest>,
    ) -> Result<Response<UpdateCollectionClusterSetupResponse>, Status> {
        self.record("update_collection_cluster_setup");
        Err(Status::unimplemented(NEVER_CALLED))
    }

    async fn create_shard_key(
        &self,
        _request: Request<CreateShardKeyRequest>,
    ) -> Result<Response<CreateShardKeyResponse>, Status> {
        self.record("create_shard_key");
        Err(Status::unimplemented(NEVER_CALLED))
    }

    async fn delete_shard_key(
        &self,
        _request: Request<DeleteShardKeyRequest>,
    ) -> Result<Response<DeleteShardKeyResponse>, Status> {
        self.record("delete_shard_key");
        Err(Status::unimplemented(NEVER_CALLED))
    }
}
