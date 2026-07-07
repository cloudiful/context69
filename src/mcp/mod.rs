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
    api::{ApiState, RequestAuth, optional_auth_middleware},
    contracts::{
        DocumentResponse, ListSourcesResponse, McpDocumentArgs, SearchRequest, SearchResponse,
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
        Ok(auth.0.map(|session| session.user.id))
    }

    async fn scope_from_context(
        &self,
        context: &RequestContext<RoleServer>,
        group_key: Option<String>,
        project_key: Option<String>,
    ) -> Result<AccessScope, McpError> {
        let user_id = self.user_id_from_context(context)?;
        self.app
            .auth
            .access_scope(user_id, group_key, project_key)
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
        Parameters(request): Parameters<SearchRequest>,
        context: RequestContext<RoleServer>,
    ) -> Result<Json<SearchResponse>, McpError> {
        let user_id = self.user_id_from_context(&context)?;
        let response = self
            .app
            .query
            .search(user_id, request)
            .await
            .map_err(internal_error)?;
        Ok(Json(response))
    }

    #[tool(
        name = "search",
        description = "Deprecated alias for search_documents."
    )]
    async fn search(
        &self,
        Parameters(request): Parameters<SearchRequest>,
        context: RequestContext<RoleServer>,
    ) -> Result<Json<SearchResponse>, McpError> {
        self.search_documents(Parameters(request), context).await
    }

    #[tool(
        name = "get_document",
        description = "Fetch a document and all of its indexed chunks."
    )]
    async fn get_document(
        &self,
        Parameters(args): Parameters<McpDocumentArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<Json<DocumentResponse>, McpError> {
        let scope = self.scope_from_context(&context, None, None).await?;
        let response = self
            .app
            .query
            .get_document(args.document_id, &scope)
            .await
            .map_err(|error| {
                if error.to_string().contains("not found") {
                    McpError::resource_not_found(error.to_string(), None)
                } else {
                    internal_error(error)
                }
            })?;
        Ok(Json(response))
    }

    #[tool(
        name = "list_sources",
        description = "List configured source connectors and checkpoint status."
    )]
    async fn list_sources(
        &self,
        context: RequestContext<RoleServer>,
    ) -> Result<Json<ListSourcesResponse>, McpError> {
        let user_id = self.user_id_from_context(&context)?;
        let sources = self
            .visible_sources(user_id)
            .await
            .map_err(internal_error)?;
        Ok(Json(ListSourcesResponse { sources }))
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
        _request: Option<PaginatedRequestParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, McpError> {
        let user_id = self.user_id_from_context(&context)?;
        let sources = self
            .visible_sources(user_id)
            .await
            .map_err(internal_error)?;
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
            next_cursor: None,
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
                ResourceTemplate::new(
                    "context69://documents/{document_id}",
                    "context69-document",
                )
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
            let scope = self.scope_from_context(&context, None, None).await?;
            let document = self
                .app
                .query
                .get_document(document_id, &scope)
                .await
                .map_err(|error| {
                    if error.to_string().contains("not found") {
                        McpError::resource_not_found(error.to_string(), None)
                    } else {
                        internal_error(error)
                    }
                })?;
            let content = serde_json::to_string_pretty(&document)
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
    let api_state = ApiState { app: app.clone() };
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
    McpError::internal_error(error.into().to_string(), None)
}
