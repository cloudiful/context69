mod tools;

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
        ReadResourceRequestParams, ReadResourceResponse, ReadResourceResult, ResourceContents,
        ResourceTemplate,
    },
    service::{RequestContext, RoleServer},
    tool, tool_handler, tool_router,
};
use tower_http::cors::{Any, CorsLayer};

use crate::{
    api::{RequestAuth, optional_auth_middleware},
    contracts::{
        McpBatchDocumentArgs, McpBatchDocumentItem, McpBatchDocumentResponse, McpDocumentArgs,
        McpDocumentKeyArgs, McpDocumentQueryArgs, McpDocumentQueryResponse, McpSearchRequest,
        McpSearchResponse, McpSourceListArgs, McpSourceListResponse,
    },
    domain::AccessScope,
    services::app::Context69App,
};
use tools::{documents as document_tools, search as search_tools, sources as source_tools};

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
        description = "Search indexed documents with vector and hybrid retrieval. Returns compact hits with a next_cursor when more results exist."
    )]
    async fn search_documents(
        &self,
        Parameters(request): Parameters<McpSearchRequest>,
        context: RequestContext<RoleServer>,
    ) -> Result<Json<McpSearchResponse>, McpError> {
        search_tools::checked_request(&request)?;
        let user_id = self.user_id_from_context(&context)?;
        let internal = request.to_search_request();
        let response = self
            .app
            .query
            .search(user_id, internal)
            .await
            .map_err(service_error)?;
        Ok(Json(search_tools::search_response(response)))
    }

    #[tool(
        name = "get_document",
        description = "Fetch one document's bounded metadata and chunk window. Pass next_chunk_cursor to fetch the next chunk page."
    )]
    async fn get_document(
        &self,
        Parameters(args): Parameters<McpDocumentArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<Json<crate::contracts::McpDocumentDetailResponse>, McpError> {
        let start = document_tools::checked_document_args(&args)?;
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
        Ok(Json(document_tools::detail_window(
            &response,
            start,
            args.chunk_limit,
        )?))
    }

    #[tool(
        name = "query_documents",
        description = "List and filter structured documents in one group. Returns bounded summaries with a next_cursor when more results exist."
    )]
    async fn query_documents(
        &self,
        Parameters(request): Parameters<McpDocumentQueryArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<Json<McpDocumentQueryResponse>, McpError> {
        document_tools::checked_query_args(&request)?;
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
            .scope_from_context(&context, Some(request.group_path.clone()))
            .await?;
        let internal = request.query.to_document_query_request();
        let response = self
            .app
            .document_store
            .query(group.id, &internal, &scope)
            .await
            .map_err(service_error)?;
        let documents = response
            .documents
            .iter()
            .map(document_tools::summary)
            .collect();
        Ok(Json(McpDocumentQueryResponse::new(
            documents,
            response.next_cursor,
        )))
    }

    #[tool(
        name = "get_document_by_external_id",
        description = "Fetch a structured document by group, source key and external id. Returns the first bounded chunk window."
    )]
    async fn get_document_by_external_id(
        &self,
        Parameters(request): Parameters<McpDocumentKeyArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<Json<crate::contracts::McpDocumentDetailResponse>, McpError> {
        document_tools::checked_key_args(&request)?;
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
            .scope_from_context(&context, Some(request.group_path.clone()))
            .await?;
        let document = self
            .app
            .document_store
            .get_by_key(group.id, &request.key, request.locale.as_deref(), &scope)
            .await
            .map_err(internal_error)?;
        Ok(Json(document_tools::first_page_detail(&document)?))
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
        document_tools::checked_batch_args(&args)?;
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
            .scope_from_context(&context, Some(args.group_path.clone()))
            .await?;
        let response = self
            .app
            .document_store
            .batch_get(
                group.id,
                &args.request.keys,
                args.request.locale.as_deref(),
                &scope,
            )
            .await
            .map_err(service_error)?;
        let items = response
            .items
            .into_iter()
            .map(|item| {
                let document = item
                    .document
                    .as_ref()
                    .and_then(|document| document_tools::batch_item_detail(document).ok());
                McpBatchDocumentItem {
                    key: item.key,
                    document,
                }
            })
            .collect();
        Ok(Json(McpBatchDocumentResponse {
            items,
            has_more: false,
        }))
    }

    #[tool(
        name = "list_sources",
        description = "List configured sources as safe summaries with cursor pagination. Operational source configuration is never disclosed."
    )]
    async fn list_sources(
        &self,
        Parameters(args): Parameters<McpSourceListArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<Json<McpSourceListResponse>, McpError> {
        let user_id = self.user_id_from_context(&context)?;
        let sources = self
            .visible_sources(user_id)
            .await
            .map_err(internal_error)?;
        let summaries = source_tools::summarize_all(&sources);
        Ok(Json(source_tools::paged_response(&summaries, &args)?))
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

#[tool_handler(router = self.tool_router)]
impl ServerHandler for Context69McpServer {
    async fn list_resources(
        &self,
        request: Option<PaginatedRequestParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, McpError> {
        let start = source_tools::resource_offset(
            request.as_ref().and_then(|params| params.cursor.as_deref()),
        )?;
        let user_id = self.user_id_from_context(&context)?;
        let sources = self
            .visible_sources(user_id)
            .await
            .map_err(internal_error)?;
        let summaries = source_tools::summarize_all(&sources);
        let (resources, next_cursor) = source_tools::resource_page(&summaries, start);
        let mut result = ListResourcesResult::with_all_items(resources);
        result.next_cursor = next_cursor;
        Ok(result)
    }

    async fn list_resource_templates(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourceTemplatesResult, McpError> {
        Ok(ListResourceTemplatesResult::with_all_items(vec![
            ResourceTemplate::new("context69://documents/{document_id}", "context69-document")
                .with_description("Fetch a single indexed document")
                .with_mime_type("application/json"),
        ]))
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResponse, McpError> {
        let uri = request.uri;
        if let Some(source_key) = uri.strip_prefix("context69://sources/") {
            let user_id = self.user_id_from_context(&context)?;
            let sources = self
                .visible_sources(user_id)
                .await
                .map_err(internal_error)?;
            let summaries = source_tools::summarize_all(&sources);
            let summary = summaries
                .into_iter()
                .find(|summary| summary.source_key == source_key)
                .context("source not found")
                .map_err(|error| McpError::resource_not_found(error.to_string(), None))?;
            let content = source_tools::resource_content(&uri, &summary)?;
            return Ok(ReadResourceResult::new(vec![content]).into());
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
            let detail = document_tools::first_page_detail(&document)?;
            let content = serde_json::to_string_pretty(&detail)
                .map_err(|error| internal_error(anyhow::Error::new(error)))?;
            return Ok(
                ReadResourceResult::new(vec![ResourceContents::text(content, uri)
                    .with_mime_type("application/json")])
                    .into(),
            );
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

pub(crate) fn internal_error<E>(error: E) -> McpError
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
