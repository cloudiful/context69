use super::Authorized;
use crate::{
    api::RequestAuth,
    contracts::PersonalAccessTokenScope::{self, Search, Sources, Workspace},
};
use rmcp::{
    ErrorData, ServerHandler,
    model::*,
    service::{RequestContext, RoleServer, serve_directly},
    transport::OneshotTransport,
};
use std::{
    collections::BTreeSet,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

#[derive(Clone, Default)]
struct Stub {
    calls: Arc<AtomicUsize>,
    reads: Arc<AtomicUsize>,
    listings: Arc<AtomicUsize>,
}
impl ServerHandler for Stub {
    async fn list_tools(
        &self,
        _: Option<PaginatedRequestParams>,
        _: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        Ok(ListToolsResult {
            tools: [
                "search_documents",
                "list_sources",
                "search_code",
                "future_tool",
            ]
            .into_iter()
            .map(|name| Tool::new(name, name, Arc::new(Default::default())))
            .collect(),
            ..Default::default()
        })
    }
    async fn call_tool(
        &self,
        _: CallToolRequestParams,
        _: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Err(ErrorData::invalid_params("inner call", None))
    }
    async fn list_resources(
        &self,
        request: Option<PaginatedRequestParams>,
        _: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, ErrorData> {
        self.listings.fetch_add(1, Ordering::SeqCst);
        let resources = match request.as_ref().and_then(|params| params.cursor.as_deref()) {
            Some("empty") => vec![],
            Some("sources") => vec![Resource::new("context69://sources/a", "source")],
            _ => vec![
                Resource::new("context69://sources/a", "source"),
                Resource::new("context69://documents/1", "document"),
                Resource::new("context69://other/a", "other"),
            ],
        };
        let mut result = ListResourcesResult::with_all_items(resources);
        result.next_cursor = Some("next".to_string());
        Ok(result)
    }
    async fn read_resource(
        &self,
        _: ReadResourceRequestParams,
        _: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResponse, ErrorData> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        Err(ErrorData::invalid_params("inner read", None))
    }
}

async fn request(
    stub: &Stub,
    method: &str,
    params: serde_json::Value,
    auth: Option<RequestAuth>,
    http: bool,
) -> serde_json::Value {
    use rmcp::model::GetExtensions;
    let mut message: ClientJsonRpcMessage = serde_json::from_value(
        serde_json::json!({"jsonrpc":"2.0", "id":1, "method":method, "params":params}),
    )
    .unwrap();
    if http {
        let (mut parts, _) = axum::http::Request::new(()).into_parts();
        if let Some(auth) = auth {
            parts.extensions.insert(auth);
        }
        if let ClientJsonRpcMessage::Request(request) = &mut message {
            request.request.extensions_mut().insert(parts);
        }
    }
    let (transport, mut receiver) = OneshotTransport::<RoleServer>::new(message);
    let running = serve_directly(Authorized::new(stub.clone()), transport, None);
    let response = receiver.recv().await.unwrap();
    running.waiting().await.unwrap();
    serde_json::to_value(response).unwrap()
}
fn pat(scopes: &[PersonalAccessTokenScope]) -> Option<RequestAuth> {
    Some(RequestAuth::test_authenticated(Some(BTreeSet::from_iter(
        scopes.iter().copied(),
    ))))
}
fn message(value: &serde_json::Value) -> &str {
    value["error"]["message"].as_str().unwrap()
}

#[tokio::test]
async fn resource_pagination_never_discloses_denied_categories() {
    let stub = Stub::default();
    for cursor in ["sources", "empty"] {
        for scopes in [vec![], vec![Search]] {
            let result = request(
                &stub,
                "resources/list",
                serde_json::json!({"cursor":cursor}),
                pat(&scopes),
                true,
            )
            .await;
            assert!(result["result"]["nextCursor"].is_null());
            assert!(result["result"]["resources"].as_array().unwrap().is_empty());
        }
        let result = request(
            &stub,
            "resources/list",
            serde_json::json!({"cursor":cursor}),
            pat(&[Sources]),
            true,
        )
        .await;
        assert_eq!(result["result"]["nextCursor"], "next");
    }
    let mixed = request(
        &stub,
        "resources/list",
        serde_json::json!({}),
        pat(&[Sources, Search]),
        true,
    )
    .await;
    assert_eq!(mixed["result"]["resources"].as_array().unwrap().len(), 2);
    assert!(mixed["result"]["nextCursor"].is_null());
    for (auth, http) in [
        (Some(RequestAuth::test_authenticated(None)), true),
        (Some(RequestAuth(None)), true),
        (None, false),
    ] {
        let result = request(&stub, "resources/list", serde_json::json!({}), auth, http).await;
        assert_eq!(result["result"]["resources"].as_array().unwrap().len(), 3);
        assert_eq!(result["result"]["nextCursor"], "next");
    }
}

#[tokio::test]
async fn tool_and_resource_boundaries_use_request_facts() {
    let stub = Stub::default();
    let tools = request(
        &stub,
        "tools/list",
        serde_json::json!({}),
        pat(&[Search]),
        true,
    )
    .await;
    let names: Vec<_> = tools["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["search_documents"]);
    for (name, scopes) in [
        ("search_documents", vec![]),
        ("search_documents", vec![Sources]),
        ("future_tool", vec![Search]),
    ] {
        let response = request(
            &stub,
            "tools/call",
            serde_json::json!({"name":name}),
            pat(&scopes),
            true,
        )
        .await;
        assert_eq!(
            message(&response),
            "personal access token missing required scope"
        );
    }
    assert_eq!(stub.calls.load(Ordering::SeqCst), 0);
    let response = request(
        &stub,
        "tools/call",
        serde_json::json!({"name":"search_documents"}),
        pat(&[Search]),
        true,
    )
    .await;
    assert_eq!(message(&response), "inner call");
    assert_eq!(stub.calls.load(Ordering::SeqCst), 1);
    let list = request(
        &stub,
        "resources/list",
        serde_json::json!({}),
        pat(&[Search]),
        true,
    )
    .await;
    assert_eq!(
        list["result"]["resources"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["uri"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["context69://documents/1"]
    );
    assert_eq!(stub.listings.load(Ordering::SeqCst), 1);
    let list = request(
        &stub,
        "resources/list",
        serde_json::json!({}),
        pat(&[Sources]),
        true,
    )
    .await;
    assert_eq!(stub.listings.load(Ordering::SeqCst), 2);
    assert_eq!(
        list["result"]["resources"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["uri"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["context69://sources/a"]
    );
    for (uri, scopes) in [
        ("context69://documents/1", vec![Sources]),
        ("context69://sources/a", vec![Search]),
        ("context69://other/a", vec![Search, Sources]),
    ] {
        let response = request(
            &stub,
            "resources/read",
            serde_json::json!({"uri":uri}),
            pat(&scopes),
            true,
        )
        .await;
        assert_eq!(
            message(&response),
            "personal access token missing required scope"
        );
    }
    assert_eq!(stub.reads.load(Ordering::SeqCst), 0);
    for (uri, scope) in [
        ("context69://documents/1", Search),
        ("context69://sources/a", Sources),
    ] {
        let response = request(
            &stub,
            "resources/read",
            serde_json::json!({"uri":uri}),
            pat(&[scope]),
            true,
        )
        .await;
        assert_eq!(message(&response), "inner read");
    }
    assert_eq!(stub.reads.load(Ordering::SeqCst), 2);
    for (auth, http) in [
        (Some(RequestAuth::test_authenticated(None)), true),
        (Some(RequestAuth(None)), true),
        (None, false),
    ] {
        let response = request(
            &stub,
            "tools/call",
            serde_json::json!({"name":"search_code"}),
            auth.clone(),
            http,
        )
        .await;
        assert_eq!(message(&response), "inner call");
        let resources = request(
            &stub,
            "resources/list",
            serde_json::json!({}),
            auth.clone(),
            http,
        )
        .await;
        assert_eq!(
            resources["result"]["resources"].as_array().unwrap().len(),
            3
        );
        let read = request(
            &stub,
            "resources/read",
            serde_json::json!({"uri":"context69://documents/1"}),
            auth,
            http,
        )
        .await;
        assert_eq!(message(&read), "inner read");
    }
    let response = request(
        &stub,
        "tools/call",
        serde_json::json!({"name":"search_code"}),
        None,
        true,
    )
    .await;
    assert_eq!(
        message(&response),
        "personal access token missing required scope"
    );
    let response = request(
        &stub,
        "tools/call",
        serde_json::json!({"name":"search_code"}),
        pat(&[Workspace]),
        true,
    )
    .await;
    assert_eq!(message(&response), "inner call");
}
