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
use uuid::Uuid;

use crate::{
    api::{AuthKind, RequestAuth, optional_auth_middleware},
    contracts::{
        McpBatchDocumentArgs, McpBatchDocumentItem, McpBatchDocumentResponse, McpCodeSearchRequest,
        McpCodeSearchResponse, McpDocumentArgs, McpDocumentKeyArgs, McpDocumentQueryArgs,
        McpDocumentQueryResponse, McpGetCodeRequest, McpGetCodeResponse, McpSearchRequest,
        McpSearchResponse, McpSourceListArgs, McpSourceListResponse, PersonalAccessTokenScope,
        TokenGroupScope,
    },
    domain::AccessScope,
    services::app::Context69App,
};
use tools::{
    code as code_tools, code_get as code_get_tools, documents as document_tools,
    search as search_tools, sources as source_tools,
};

#[derive(Clone)]
pub struct Context69McpServer {
    app: Arc<Context69App>,
    tool_router: ToolRouter<Self>,
}

#[derive(Clone)]
struct McpAuthContext {
    user_id: Option<i64>,
    kind: Option<AuthKind>,
    telemetry_due: bool,
    group_scope: Option<TokenGroupScope>,
}

impl McpAuthContext {
    fn anonymous() -> Self {
        Self {
            user_id: None,
            kind: None,
            telemetry_due: false,
            group_scope: None,
        }
    }

    fn is_group_restricted(&self) -> bool {
        is_group_restricted(&self.group_scope)
    }
}

fn is_group_restricted(scope: &Option<TokenGroupScope>) -> bool {
    matches!(scope, Some(TokenGroupScope::Groups { .. }))
}

fn token_missing_scope(kind: &Option<AuthKind>, required: PersonalAccessTokenScope) -> bool {
    match kind {
        Some(AuthKind::PersonalAccessToken { scopes, .. }) => !scopes.contains(&required),
        _ => false,
    }
}

fn successful_use_token_id(auth: &McpAuthContext) -> Option<Uuid> {
    if !auth.telemetry_due {
        return None;
    }
    match &auth.kind {
        Some(AuthKind::PersonalAccessToken { token_id, .. }) => Some(*token_id),
        _ => None,
    }
}

impl Context69McpServer {
    pub fn new(app: Arc<Context69App>) -> Self {
        Self {
            app,
            tool_router: Self::tool_router(),
        }
    }

    fn auth_from_context(
        &self,
        context: &RequestContext<RoleServer>,
    ) -> Result<McpAuthContext, McpError> {
        let Some(parts) = context.extensions.get::<axum::http::request::Parts>() else {
            return Ok(McpAuthContext::anonymous());
        };
        let auth = parts
            .extensions
            .get::<RequestAuth>()
            .cloned()
            .unwrap_or(RequestAuth(None));
        let Some(authenticated) = auth.0 else {
            if !self.app.auth.anonymous_mcp_enabled() {
                return Err(McpError::invalid_request(
                    "anonymous mcp is disabled".to_string(),
                    None,
                ));
            }
            return Ok(McpAuthContext::anonymous());
        };
        Ok(McpAuthContext {
            user_id: Some(authenticated.session.user.id),
            kind: Some(authenticated.kind),
            telemetry_due: authenticated.telemetry_due,
            group_scope: authenticated.session.token_group_scope,
        })
    }

    fn require_scope(
        &self,
        auth: &McpAuthContext,
        required: PersonalAccessTokenScope,
    ) -> Result<(), McpError> {
        if token_missing_scope(&auth.kind, required) {
            return Err(McpError::invalid_request(
                format!(
                    "forbidden: personal access token missing {} scope",
                    scope_name(required)
                ),
                None,
            ));
        }
        Ok(())
    }

    /// Source enumeration is global, so a group-restricted token is rejected
    /// exactly like the HTTP source surface instead of filtering after the fact.
    fn require_source_access(&self, auth: &McpAuthContext) -> Result<(), McpError> {
        self.require_scope(auth, PersonalAccessTokenScope::Sources)?;
        if auth.is_group_restricted() {
            return Err(McpError::invalid_request(
                "forbidden: source enumeration is not available to a group-restricted personal access token"
                    .to_string(),
                None,
            ));
        }
        Ok(())
    }

    fn observe_success(&self, auth: &McpAuthContext) {
        if let Some(token_id) = successful_use_token_id(auth) {
            self.app.personal_access_tokens.record_last_used(token_id);
        }
    }

    async fn scope_from_context(
        &self,
        auth: &McpAuthContext,
        group_path: Option<String>,
    ) -> Result<AccessScope, McpError> {
        self.app
            .auth
            .access_scope(auth.user_id, group_path, auth.group_scope.as_ref())
            .await
            .map_err(internal_error)
    }
}

fn scope_name(scope: PersonalAccessTokenScope) -> &'static str {
    match scope {
        PersonalAccessTokenScope::Search => "search",
        PersonalAccessTokenScope::Workspace => "workspace",
        PersonalAccessTokenScope::Library => "library",
        PersonalAccessTokenScope::Sources => "sources",
        PersonalAccessTokenScope::Settings => "settings",
        PersonalAccessTokenScope::Admin => "admin",
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
        let auth = self.auth_from_context(&context)?;
        self.require_scope(&auth, PersonalAccessTokenScope::Search)?;
        let internal = request.to_search_request();
        let response = self
            .app
            .query
            .search(auth.user_id, internal, auth.group_scope.as_ref())
            .await
            .map_err(service_error)?;
        let response = search_tools::search_response(response);
        self.observe_success(&auth);
        Ok(Json(response))
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
        let auth = self.auth_from_context(&context)?;
        self.require_scope(&auth, PersonalAccessTokenScope::Search)?;
        let scope = self.scope_from_context(&auth, None).await?;
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
        let detail = document_tools::detail_window(&response, start, args.chunk_limit)?;
        self.observe_success(&auth);
        Ok(Json(detail))
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
        let auth = self.auth_from_context(&context)?;
        self.require_scope(&auth, PersonalAccessTokenScope::Search)?;
        let user_id = auth
            .user_id
            .ok_or_else(|| McpError::invalid_request("authentication required", None))?;
        let group = self
            .app
            .namespace
            .get_group_for_user(user_id, &request.group_path, None)
            .await
            .map_err(internal_error)?
            .ok_or_else(|| McpError::resource_not_found("group not found", None))?;
        crate::authorization::require_group_allowed(auth.group_scope.as_ref(), group.id)
            .map_err(internal_error)?;
        let scope = self
            .scope_from_context(&auth, Some(request.group_path.clone()))
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
        let response = McpDocumentQueryResponse::new(documents, response.next_cursor);
        self.observe_success(&auth);
        Ok(Json(response))
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
        let auth = self.auth_from_context(&context)?;
        self.require_scope(&auth, PersonalAccessTokenScope::Search)?;
        let user_id = auth
            .user_id
            .ok_or_else(|| McpError::invalid_request("authentication required", None))?;
        let group = self
            .app
            .namespace
            .get_group_for_user(user_id, &request.group_path, None)
            .await
            .map_err(internal_error)?
            .ok_or_else(|| McpError::resource_not_found("group not found", None))?;
        crate::authorization::require_group_allowed(auth.group_scope.as_ref(), group.id)
            .map_err(internal_error)?;
        let scope = self
            .scope_from_context(&auth, Some(request.group_path.clone()))
            .await?;
        let document = self
            .app
            .document_store
            .get_by_key(group.id, &request.key, request.locale.as_deref(), &scope)
            .await
            .map_err(internal_error)?;
        let detail = document_tools::first_page_detail(&document)?;
        self.observe_success(&auth);
        Ok(Json(detail))
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
        let auth = self.auth_from_context(&context)?;
        self.require_scope(&auth, PersonalAccessTokenScope::Search)?;
        let user_id = auth.user_id.ok_or_else(|| {
            McpError::invalid_request("authentication required".to_string(), None)
        })?;
        let group = self
            .app
            .namespace
            .get_group_for_user(user_id, &args.group_path, None)
            .await
            .map_err(internal_error)?
            .ok_or_else(|| McpError::resource_not_found("group not found", None))?;
        crate::authorization::require_group_allowed(auth.group_scope.as_ref(), group.id)
            .map_err(internal_error)?;
        let scope = self
            .scope_from_context(&auth, Some(args.group_path.clone()))
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
        self.observe_success(&auth);
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
        let auth = self.auth_from_context(&context)?;
        self.require_source_access(&auth)?;
        let sources = self
            .visible_sources(auth.user_id)
            .await
            .map_err(internal_error)?;
        let summaries = source_tools::summarize_all(&sources);
        let response = source_tools::paged_response(&summaries, &args)?;
        self.observe_success(&auth);
        Ok(Json(response))
    }

    #[tool(
        name = "search_code",
        description = "Search one Git repository's active index generation with the repository's configured profile (lexical, hybrid, or full_semantic). Accepts group_path, repository_key, a bounded query, optional path_prefix/language filters, and limit; returns code hits with commit/ref/path/line provenance, requested/effective profile and explicit vector_unavailable fallback reason, generation coverage, and a truthful truncated flag."
    )]
    async fn search_code(
        &self,
        Parameters(request): Parameters<McpCodeSearchRequest>,
        context: RequestContext<RoleServer>,
    ) -> Result<Json<McpCodeSearchResponse>, McpError> {
        code_tools::checked_search_args(&request)?;
        let auth = self.auth_from_context(&context)?;
        self.require_scope(&auth, PersonalAccessTokenScope::Search)?;
        let user_id = auth
            .user_id
            .ok_or_else(|| McpError::invalid_request("authentication required", None))?;
        let group = self
            .app
            .namespace
            .get_group_for_user(user_id, &request.group_path, None)
            .await
            .map_err(internal_error)?
            .ok_or_else(|| McpError::resource_not_found("group not found", None))?;
        crate::authorization::require_group_allowed(auth.group_scope.as_ref(), group.id)
            .map_err(internal_error)?;
        let scope = self
            .scope_from_context(&auth, Some(request.group_path.clone()))
            .await?;
        let response =
            code_tools::search(&self.app.db, &self.app.settings, &group, &scope, &request).await?;
        self.observe_success(&auth);
        Ok(Json(response))
    }

    #[tool(
        name = "get_code",
        description = "Read one bounded, commit-pinned line window of a file in one Git repository's active index generation. Accepts group_path, repository_key, a safe repository-relative path, inclusive start_line/end_line, and an optional chunk_limit; returns the verbatim stored UTF-8 text with generation provenance, coverage, the exact byte count, and a truthful truncated flag."
    )]
    async fn get_code(
        &self,
        Parameters(request): Parameters<McpGetCodeRequest>,
        context: RequestContext<RoleServer>,
    ) -> Result<Json<McpGetCodeResponse>, McpError> {
        code_get_tools::checked_get_args(&request)?;
        let auth = self.auth_from_context(&context)?;
        self.require_scope(&auth, PersonalAccessTokenScope::Search)?;
        let user_id = auth
            .user_id
            .ok_or_else(|| McpError::invalid_request("authentication required", None))?;
        let group = self
            .app
            .namespace
            .get_group_for_user(user_id, &request.group_path, None)
            .await
            .map_err(internal_error)?
            .ok_or_else(|| McpError::resource_not_found("group not found", None))?;
        crate::authorization::require_group_allowed(auth.group_scope.as_ref(), group.id)
            .map_err(internal_error)?;
        let scope = self
            .scope_from_context(&auth, Some(request.group_path.clone()))
            .await?;
        let response = code_get_tools::get(&self.app.db, group.id, &scope, &request).await?;
        self.observe_success(&auth);
        Ok(Json(response))
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
        let auth = self.auth_from_context(&context)?;
        self.require_source_access(&auth)?;
        let sources = self
            .visible_sources(auth.user_id)
            .await
            .map_err(internal_error)?;
        let summaries = source_tools::summarize_all(&sources);
        let (resources, next_cursor) = source_tools::resource_page(&summaries, start);
        let mut result = ListResourcesResult::with_all_items(resources);
        result.next_cursor = next_cursor;
        self.observe_success(&auth);
        Ok(result)
    }

    async fn list_resource_templates(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourceTemplatesResult, McpError> {
        // A static template reveals no data and grants no execution right, so
        // it neither needs a functional scope nor counts as successful use.
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
        let auth = self.auth_from_context(&context)?;
        if let Some(source_key) = uri.strip_prefix("context69://sources/") {
            self.require_source_access(&auth)?;
            let sources = self
                .visible_sources(auth.user_id)
                .await
                .map_err(internal_error)?;
            let summaries = source_tools::summarize_all(&sources);
            let summary = summaries
                .into_iter()
                .find(|summary| summary.source_key == source_key)
                .context("source not found")
                .map_err(|error| McpError::resource_not_found(error.to_string(), None))?;
            let content = source_tools::resource_content(&uri, &summary)?;
            let result = ReadResourceResult::new(vec![content]).into();
            self.observe_success(&auth);
            return Ok(result);
        }

        if let Some(document_id) = uri.strip_prefix("context69://documents/") {
            self.require_scope(&auth, PersonalAccessTokenScope::Search)?;
            let document_id = document_id.parse::<i64>().map_err(|error| {
                McpError::invalid_params(format!("invalid document id: {error}"), None)
            })?;
            let scope = self.scope_from_context(&auth, None).await?;
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
            let result = ReadResourceResult::new(vec![
                ResourceContents::text(content, uri).with_mime_type("application/json"),
            ])
            .into();
            self.observe_success(&auth);
            return Ok(result);
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

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::{
        AuthKind, McpAuthContext, is_group_restricted, successful_use_token_id, token_missing_scope,
    };
    use crate::contracts::{PersonalAccessTokenScope, TokenGroupScope};
    use uuid::Uuid;

    fn pat(scopes: &[PersonalAccessTokenScope], telemetry_due: bool) -> McpAuthContext {
        McpAuthContext {
            user_id: Some(7),
            kind: Some(AuthKind::PersonalAccessToken {
                token_id: Uuid::nil(),
                scopes: BTreeSet::from_iter(scopes.iter().copied()),
            }),
            telemetry_due,
            group_scope: None,
        }
    }

    #[test]
    fn mcp_requires_the_functional_scope_for_a_personal_access_token() {
        let search = pat(&[PersonalAccessTokenScope::Search], true);
        assert!(!token_missing_scope(
            &search.kind,
            PersonalAccessTokenScope::Search
        ));
        assert!(token_missing_scope(
            &search.kind,
            PersonalAccessTokenScope::Sources
        ));

        let browser = McpAuthContext {
            kind: Some(AuthKind::BrowserSession),
            ..search
        };
        assert!(!token_missing_scope(
            &browser.kind,
            PersonalAccessTokenScope::Sources
        ));
        assert!(!token_missing_scope(
            &McpAuthContext::anonymous().kind,
            PersonalAccessTokenScope::Search
        ));
    }

    #[test]
    fn restricted_tokens_cannot_use_global_source_enumeration() {
        assert!(!is_group_restricted(&None));
        assert!(!is_group_restricted(&Some(TokenGroupScope::All)));
        assert!(is_group_restricted(&Some(TokenGroupScope::Groups {
            group_ids: vec![4],
        })));
    }

    #[test]
    fn mcp_observes_use_only_for_a_due_personal_access_token() {
        let token_id = Uuid::new_v4();
        let mut auth = pat(&[PersonalAccessTokenScope::Search], true);
        auth.kind = Some(AuthKind::PersonalAccessToken {
            token_id,
            scopes: BTreeSet::from([PersonalAccessTokenScope::Search]),
        });
        assert_eq!(successful_use_token_id(&auth), Some(token_id));

        auth.telemetry_due = false;
        assert_eq!(successful_use_token_id(&auth), None);

        auth.telemetry_due = true;
        auth.kind = Some(AuthKind::BrowserSession);
        assert_eq!(successful_use_token_id(&auth), None);
        assert_eq!(successful_use_token_id(&McpAuthContext::anonymous()), None);
    }
}
