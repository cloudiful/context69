use std::sync::Arc;

use anyhow::{Context, Result};
use axum::{
    Router,
    http::{HeaderName, Method, header},
    middleware::from_fn_with_state,
};
use rmcp::{
    ErrorData as McpError, Json, ServerHandler,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{
        ListResourceTemplatesResult, ListResourcesResult, PaginatedRequestParams,
        ReadResourceRequestParams, ReadResourceResult, Resource, ResourceContents,
        ResourceTemplate,
    },
    service::{RequestContext, RoleServer},
    tool, tool_handler, tool_router,
};
use tower_http::cors::{Any, CorsLayer};

use crate::{
    api::{RequestAuth, optional_auth_middleware},
    contracts::{
        DocumentKey, DocumentQueryRequest, DocumentResponse, McpBatchDocumentArgs,
        McpBatchDocumentItem, McpBatchDocumentResponse, McpDocumentArgs, McpDocumentDetailResponse,
        McpDocumentQueryResponse, McpDocumentSummary, McpSearchHit, McpSearchResponse,
        McpSourceListResponse, SearchRequest,
    },
    domain::AccessScope,
    services::app::Context69App,
};

#[derive(Clone)]
pub struct Context69McpServer {
    app: Arc<Context69App>,
    tool_router: ToolRouter<Self>,
}

impl Context69McpServer {
    pub fn new(app: Arc<Context69App>) -> Self {
        Self {
            app,
            tool_router: Self::tool_router(),
        }
    }

    fn user_id_from_context(
        &self,
        context: &RequestContext<RoleServer>,
    ) -> Result<Option<i64>, McpError> {
        let Some(parts) = context.extensions.get::<axum::http::request::Parts>() else {
            return Ok(None);
        };
        let auth = parts
            .extensions
            .get::<RequestAuth>()
            .cloned()
            .unwrap_or(RequestAuth(None));
        if auth.0.is_none() && !self.app.auth.anonymous_mcp_enabled() {
            return Err(McpError::invalid_request(
                "anonymous mcp is disabled".to_string(),
                None,
            ));
        }
        Ok(auth.0.map(|authenticated| authenticated.session.user.id))
    }

    async fn scope_from_context(
        &self,
        context: &RequestContext<RoleServer>,
        group_path: Option<String>,
    ) -> Result<AccessScope, McpError> {
        let user_id = self.user_id_from_context(context)?;
        self.app
            .auth
            .access_scope(user_id, group_path)
            .await
            .map_err(internal_error)
    }
}

#[tool_router(router = tool_router)]
impl Context69McpServer {
    #[tool(
        name = "search_documents",
        description = "Search indexed documents with vector and hybrid retrieval."
    )]
    async fn search_documents(
        &self,
        Parameters(mut request): Parameters<SearchRequest>,
        context: RequestContext<RoleServer>,
    ) -> Result<Json<McpSearchResponse>, McpError> {
        if request.limit == 0 {
            return Err(McpError::invalid_params(
                "limit must be between 1 and 20".to_string(),
                Some(serde_json::json!({"fix": "set limit to a value between 1 and 20"})),
            ));
        }
        let limit = request.limit.min(20);
        let truncated = request.limit > limit;
        request.limit = limit;
        let user_id = self.user_id_from_context(&context)?;
        let response = self
            .app
            .query
            .search(user_id, request)
            .await
            .map_err(service_error)?;
        Ok(Json(McpSearchResponse {
            query: response.query,
            hits: response
                .items
                .into_iter()
                .map(|hit| McpSearchHit {
                    document_id: hit.document_id,
                    external_id: hit.external_id,
                    title: hit.title,
                    summary: hit.summary,
                    source_uri: hit.source_uri,
                    published_at: hit.published_at,
                    score: hit.score,
                    snippet: hit.chunk_text.chars().take(600).collect(),
                })
                .collect(),
            truncated,
            has_more: truncated,
        }))
    }

    #[tool(
        name = "get_document",
        description = "Fetch a document and all of its indexed chunks."
    )]
    async fn get_document(
        &self,
        Parameters(args): Parameters<McpDocumentArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<Json<McpDocumentDetailResponse>, McpError> {
        let scope = self.scope_from_context(&context, None).await?;
        let response = self
            .app
            .query
            .get_document(args.document_id, args.locale.as_deref(), &scope)
            .await
            .map_err(|error| {
                if error.to_string().contains("not found") {
                    McpError::resource_not_found(error.to_string(), None)
                } else {
                    internal_error(error)
                }
            })?;
        Ok(Json(paginate_document(response, &args)?))
    }

    #[tool(
        name = "query_documents",
        description = "List and filter structured documents in one group."
    )]
    async fn query_documents(
        &self,
        Parameters(request): Parameters<McpDocumentQueryArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<Json<McpDocumentQueryResponse>, McpError> {
        let user_id = self
            .user_id_from_context(&context)?
            .ok_or_else(|| McpError::invalid_request("authentication required", None))?;
        let group = self
            .app
            .namespace
            .get_group_for_user(user_id, &request.group_path)
            .await
            .map_err(internal_error)?
            .ok_or_else(|| McpError::resource_not_found("group not found", None))?;
        let scope = self
            .scope_from_context(&context, Some(request.group_path))
            .await?;
        let mut query = request.query;
        if query.limit == 0 {
            return Err(McpError::invalid_params(
                "limit must be between 1 and 20".to_string(),
                Some(serde_json::json!({"fix": "set limit to a value between 1 and 20"})),
            ));
        }
        let limit = query.limit.min(20);
        let truncated = query.limit > limit;
        query.limit = limit;
        let response = self
            .app
            .document_store
            .query(group.id, &query, &scope)
            .await
            .map_err(service_error)?;
        let has_more = truncated || response.next_cursor.is_some();
        Ok(Json(McpDocumentQueryResponse {
            documents: response
                .documents
                .into_iter()
                .map(document_summary)
                .collect(),
            next_cursor: response.next_cursor,
            truncated,
            has_more,
        }))
    }

    #[tool(
        name = "get_document_by_external_id",
        description = "Fetch a structured document by group, source key and external id."
    )]
    async fn get_document_by_external_id(
        &self,
        Parameters(request): Parameters<McpDocumentKeyArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<Json<McpDocumentDetailResponse>, McpError> {
        let user_id = self
            .user_id_from_context(&context)?
            .ok_or_else(|| McpError::invalid_request("authentication required", None))?;
        let group = self
            .app
            .namespace
            .get_group_for_user(user_id, &request.group_path)
            .await
            .map_err(internal_error)?
            .ok_or_else(|| McpError::resource_not_found("group not found", None))?;
        let scope = self
            .scope_from_context(&context, Some(request.group_path))
            .await?;
        let document = self
            .app
            .document_store
            .get_by_key(group.id, &request.key, request.locale.as_deref(), &scope)
            .await
            .map_err(internal_error)?;
        Ok(Json(paginate_document(
            document,
            &McpDocumentArgs {
                document_id: 0,
                locale: request.locale,
                chunk_cursor: None,
                chunk_limit: 20,
            },
        )?))
    }

    #[tool(
        name = "get_documents",
        description = "Fetch a bounded batch of document details. Use after search_documents or query_documents."
    )]
    async fn get_documents(
        &self,
        Parameters(args): Parameters<McpBatchDocumentArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<Json<McpBatchDocumentResponse>, McpError> {
        let user_id = self.user_id_from_context(&context)?.ok_or_else(|| {
            McpError::invalid_request("authentication required".to_string(), None)
        })?;
        let group = self
            .app
            .namespace
            .get_group_for_user(user_id, &args.group_path)
            .await
            .map_err(internal_error)?
            .ok_or_else(|| McpError::resource_not_found("group not found", None))?;
        let scope = self
            .scope_from_context(&context, Some(args.group_path))
            .await?;
        let mut request = args.request;
        if request.keys.is_empty() {
            return Err(McpError::invalid_params(
                "keys must contain at least one document key".to_string(),
                Some(serde_json::json!({"fix": "provide one or more document keys"})),
            ));
        }
        let truncated = request.keys.len() > 20;
        request.keys.truncate(20);
        let response = self
            .app
            .document_store
            .batch_get(group.id, &request.keys, request.locale.as_deref(), &scope)
            .await
            .map_err(service_error)?;
        let items = response
            .items
            .into_iter()
            .map(|item| {
                let document = item.document.and_then(|document| {
                    paginate_document(
                        document,
                        &McpDocumentArgs {
                            document_id: 0,
                            locale: request.locale.clone(),
                            chunk_cursor: None,
                            chunk_limit: 5,
                        },
                    )
                    .ok()
                });
                McpBatchDocumentItem {
                    key: item.key,
                    document,
                }
            })
            .collect();
        Ok(Json(McpBatchDocumentResponse {
            items,
            truncated,
            has_more: truncated,
        }))
    }

    #[tool(
        name = "list_sources",
        description = "List configured source connectors and checkpoint status."
    )]
    async fn list_sources(
        &self,
        context: RequestContext<RoleServer>,
    ) -> Result<Json<McpSourceListResponse>, McpError> {
        let user_id = self.user_id_from_context(&context)?;
        let sources = self
            .visible_sources(user_id)
            .await
            .map_err(internal_error)?;
        let truncated = sources.len() > 100;
        let mut sources = sources;
        sources.truncate(100);
        Ok(Json(McpSourceListResponse {
            sources,
            truncated,
            has_more: truncated,
        }))
    }

    async fn visible_sources(
        &self,
        user_id: Option<i64>,
    ) -> Result<Vec<crate::contracts::SourceStatus>> {
        let mut sources = self.app.sync.list_sources().await?;
        if user_id.is_none() {
            sources.retain(|source| source.visibility == crate::contracts::Visibility::Public);
        }
        Ok(sources)
    }
}

fn document_summary(document: DocumentResponse) -> McpDocumentSummary {
    McpDocumentSummary {
        document_id: document.document_id,
        external_id: document.external_id,
        title: document.title,
        summary: document.summary,
        source_uri: document.source_uri,
        published_at: document.published_at,
        updated_at: document.updated_at,
    }
}

fn paginate_document(
    mut document: DocumentResponse,
    args: &McpDocumentArgs,
) -> Result<McpDocumentDetailResponse, McpError> {
    if args.chunk_limit == 0 || args.chunk_limit > 50 {
        return Err(McpError::invalid_params(
            "chunk_limit must be between 1 and 50".to_string(),
            Some(serde_json::json!({"fix": "set chunk_limit to a value between 1 and 50"})),
        ));
    }
    let start = args
        .chunk_cursor
        .as_deref()
        .unwrap_or("0")
        .parse::<usize>()
        .map_err(|_| {
            McpError::invalid_params(
                "chunk_cursor must be a non-negative integer".to_string(),
                Some(serde_json::json!({"fix": "use the next_chunk_cursor returned by get_document"})),
            )
        })?;
    let end = start
        .saturating_add(args.chunk_limit)
        .min(document.chunks.len());
    let has_more = end < document.chunks.len();
    let next_chunk_cursor = has_more.then(|| end.to_string());
    document.chunks = document
        .chunks
        .into_iter()
        .skip(start)
        .take(args.chunk_limit)
        .collect();
    for chunk in &mut document.chunks {
        chunk.text = chunk.text.chars().take(4_000).collect();
    }
    Ok(McpDocumentDetailResponse {
        document,
        next_chunk_cursor,
        has_more,
        truncated: has_more,
    })
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct McpDocumentQueryArgs {
    group_path: String,
    query: DocumentQueryRequest,
}
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct McpDocumentKeyArgs {
    group_path: String,
    key: DocumentKey,
    #[serde(default)]
    locale: Option<String>,
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for Context69McpServer {
    async fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, McpError> {
        let user_id = self.user_id_from_context(&context)?;
        let mut sources = self
            .visible_sources(user_id)
            .await
            .map_err(internal_error)?;
        let truncated = sources.len() > 100;
        sources.truncate(100);
        let resources = sources
            .into_iter()
            .map(|source| {
                Resource::new(
                    format!("context69://sources/{}", source.source_key),
                    source.source_key,
                )
                .with_description("Configured source checkpoint status")
                .with_mime_type("application/json")
            })
            .collect::<Vec<_>>();
        Ok(ListResourcesResult {
            meta: None,
            resources,
            next_cursor: truncated.then(|| "100".to_string()),
        })
    }

    async fn list_resource_templates(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourceTemplatesResult, McpError> {
        Ok(ListResourceTemplatesResult {
            meta: None,
            resource_templates: vec![
                ResourceTemplate::new("context69://documents/{document_id}", "context69-document")
                    .with_description("Fetch a single indexed document")
                    .with_mime_type("application/json"),
            ],
            next_cursor: None,
        })
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResult, McpError> {
        let uri = request.uri;
        if let Some(source_key) = uri.strip_prefix("context69://sources/") {
            let user_id = self.user_id_from_context(&context)?;
            let sources = self
                .visible_sources(user_id)
                .await
                .map_err(internal_error)?;
            let source = sources
                .into_iter()
                .find(|source| source.source_key == source_key)
                .context("source not found")
                .map_err(|error| McpError::resource_not_found(error.to_string(), None))?;
            let content = serde_json::to_string_pretty(&source)
                .map_err(|error| internal_error(anyhow::Error::new(error)))?;
            return Ok(ReadResourceResult::new(vec![
                ResourceContents::text(content, uri).with_mime_type("application/json"),
            ]));
        }

        if let Some(document_id) = uri.strip_prefix("context69://documents/") {
            let document_id = document_id.parse::<i64>().map_err(|error| {
                McpError::invalid_params(format!("invalid document id: {error}"), None)
            })?;
            let scope = self.scope_from_context(&context, None).await?;
            let document = self
                .app
                .query
                .get_document(document_id, None, &scope)
                .await
                .map_err(|error| {
                    if error.to_string().contains("not found") {
                        McpError::resource_not_found(error.to_string(), None)
                    } else {
                        internal_error(error)
                    }
                })?;
            let detail = paginate_document(
                document,
                &McpDocumentArgs {
                    document_id,
                    locale: None,
                    chunk_cursor: None,
                    chunk_limit: 20,
                },
            )?;
            let content = serde_json::to_string_pretty(&detail)
                .map_err(|error| internal_error(anyhow::Error::new(error)))?;
            return Ok(ReadResourceResult::new(vec![
                ResourceContents::text(content, uri).with_mime_type("application/json"),
            ]));
        }

        Err(McpError::resource_not_found(
            format!("unknown resource uri: {uri}"),
            None,
        ))
    }
}

pub async fn run_stdio(app: Arc<Context69App>) -> Result<()> {
    let server = Context69McpServer::new(app);
    let running = server::mcp::serve_stdio(server).await?;
    running.waiting().await?;
    Ok(())
}

pub async fn run_http(app: Arc<Context69App>) -> Result<()> {
    let bind_addr = app.config.mcp.bind_addr.clone();
    let router = router(app)?;
    let server_config = server::ServerConfig::new()
        .with_listen_addr(bind_addr)
        .build()?;
    let bound = server::axum::Server::new(server_config, router).bind()?;
    tracing::info!(addrs = ?bound.addrs(), "mcp http listening");
    bound.run().await?;
    Ok(())
}

pub fn router(app: Arc<Context69App>) -> Result<Router> {
    let api_state = crate::api::build_api_state(app.clone());
    let router = server::mcp::router(streamable_http_config(), move || {
        Context69McpServer::new(app.clone())
    })?;
    Ok(router
        .layer(from_fn_with_state(api_state, optional_auth_middleware))
        .layer(cors_layer()))
}

fn streamable_http_config() -> server::mcp::ServerConfig {
    server::mcp::ServerConfig::new()
        .disable_allowed_hosts()
        .disable_allowed_origins()
}

fn cors_layer() -> CorsLayer {
    CorsLayer::new()
        .allow_origin(Any)
        .allow_methods([Method::GET, Method::POST, Method::OPTIONS])
        .allow_headers([
            header::ACCEPT,
            header::AUTHORIZATION,
            header::CONTENT_TYPE,
            HeaderName::from_static("last-event-id"),
            HeaderName::from_static("mcp-protocol-version"),
            HeaderName::from_static("mcp-session-id"),
        ])
        .expose_headers([
            HeaderName::from_static("mcp-protocol-version"),
            HeaderName::from_static("mcp-session-id"),
        ])
}

fn internal_error<E>(error: E) -> McpError
where
    E: Into<anyhow::Error>,
{
    let message = error.into().to_string();
    let retryable = message.contains("timeout")
        || message.contains("temporarily")
        || message.contains("connection")
        || message.contains("unavailable");
    McpError::internal_error(
        message,
        Some(serde_json::json!({
            "retryable": retryable,
            "fix": if retryable { "retry the request" } else { "check the request and service logs" }
        })),
    )
}

fn service_error<E>(error: E) -> McpError
where
    E: Into<anyhow::Error>,
{
    let message = error.into().to_string();
    let lower = message.to_ascii_lowercase();
    if [
        "invalid",
        "filter",
        "metadata index",
        "sort",
        "cursor",
        "limit",
    ]
    .iter()
    .any(|marker| lower.contains(marker))
    {
        return McpError::invalid_params(
            message,
            Some(serde_json::json!({"fix": "check the filter, sort, cursor, and limit values"})),
        );
    }
    internal_error(anyhow::anyhow!(message))
}
