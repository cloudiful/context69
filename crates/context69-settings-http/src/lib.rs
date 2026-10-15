use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use axum::{
    Router,
    extract::{FromRef, State},
    http::StatusCode,
    response::IntoResponse,
    routing::get,
};
use context69_contracts_core::common::ApiErrorResponse;
use context69_contracts_settings::{
    CanonicalUpdateSearchSettingsRequest, DoclingSettingsResponse, RuntimeSettingsResponse,
    SearchSettingsResponse, TestLlmProviderOutcome, TestLlmProviderRequest,
    TestLlmProviderResponse, TestRuntimeEmbeddingRequest, TestRuntimeQdrantRequest,
    TestRuntimeValkeyRequest, UpdateDoclingConnectionSettings, UpdateDoclingSettingsRequest,
    UpdateRuntimeS3Settings, UpdateRuntimeSettingsRequest,
};
use context69_http_support::{CurrentUser, internal_error_response, map_settings_error};
use utoipa::OpenApi;

#[async_trait]
pub trait SettingsApi: Send + Sync {
    async fn get_runtime_settings(&self) -> Result<RuntimeSettingsResponse>;
    async fn update_runtime_settings(
        &self,
        request: &UpdateRuntimeSettingsRequest,
    ) -> Result<RuntimeSettingsResponse>;
    async fn test_s3_connection(&self, request: &UpdateRuntimeS3Settings) -> Result<()>;
    async fn test_valkey_connection(&self, request: &TestRuntimeValkeyRequest) -> Result<()>;
    async fn test_embedding_connection(&self, request: &TestRuntimeEmbeddingRequest) -> Result<()>;
    /// A non-persisting connectivity check against a submitted Qdrant endpoint.
    async fn test_qdrant_connection(&self, request: &TestRuntimeQdrantRequest) -> Result<()>;
    async fn get_docling_settings(&self) -> Result<DoclingSettingsResponse>;
    async fn update_docling_settings(
        &self,
        request: &UpdateDoclingSettingsRequest,
    ) -> Result<DoclingSettingsResponse>;
    /// A non-persisting connectivity check against a submitted Docling endpoint.
    async fn test_docling_connection(
        &self,
        request: &UpdateDoclingConnectionSettings,
    ) -> Result<()>;
    /// A non-persisting structured-call probe against submitted LLM draft values.
    async fn test_llm_provider_connection(
        &self,
        request: &TestLlmProviderRequest,
    ) -> Result<TestLlmProviderResponse>;
    async fn get_search_settings(&self) -> Result<SearchSettingsResponse>;
    async fn update_search_settings(
        &self,
        request: &CanonicalUpdateSearchSettingsRequest,
    ) -> Result<SearchSettingsResponse>;
}

#[derive(Clone)]
pub struct SettingsHttpState {
    pub settings: Arc<dyn SettingsApi>,
}

pub fn router<S>() -> Router<S>
where
    S: Clone + Send + Sync + 'static,
    SettingsHttpState: FromRef<S>,
{
    Router::new()
        .route(
            "/v1/settings/runtime",
            get(get_runtime_settings).put(update_runtime_settings),
        )
        .route(
            "/v1/settings/runtime/s3/test",
            axum::routing::post(test_s3_connection),
        )
        .route(
            "/v1/settings/runtime/valkey/test",
            axum::routing::post(test_valkey_connection),
        )
        .route(
            "/v1/settings/runtime/embedding/test",
            axum::routing::post(test_embedding_connection),
        )
        .route(
            "/v1/settings/runtime/qdrant/test",
            axum::routing::post(test_qdrant_connection),
        )
        .route(
            "/v1/settings/docling",
            get(get_docling_settings).put(update_docling_settings),
        )
        .route(
            "/v1/settings/docling/test",
            axum::routing::post(test_docling_connection),
        )
        .route(
            "/v1/settings/translation/providers/llm/test",
            axum::routing::post(test_llm_provider_connection),
        )
        .route(
            "/v1/settings/search",
            get(get_search_settings).put(update_search_settings),
        )
}

#[derive(OpenApi)]
#[openapi(
    paths(
        get_runtime_settings,
        update_runtime_settings,
        test_s3_connection,
        test_valkey_connection,
        test_embedding_connection,
        test_qdrant_connection,
        get_docling_settings,
        update_docling_settings,
        test_docling_connection,
        test_llm_provider_connection,
        get_search_settings,
        update_search_settings
    ),
    components(
        schemas(
            ApiErrorResponse,
            RuntimeSettingsResponse,
            UpdateRuntimeSettingsRequest,
            UpdateRuntimeS3Settings,
            TestRuntimeValkeyRequest,
            TestRuntimeEmbeddingRequest,
            TestRuntimeQdrantRequest,
            DoclingSettingsResponse,
            UpdateDoclingSettingsRequest,
            UpdateDoclingConnectionSettings,
            TestLlmProviderRequest,
            TestLlmProviderResponse,
            TestLlmProviderOutcome,
            SearchSettingsResponse,
            CanonicalUpdateSearchSettingsRequest
        )
    ),
    tags((name = "settings", description = "Runtime settings transport"))
)]
struct SettingsApiDoc;

pub fn openapi_document() -> utoipa::openapi::OpenApi {
    SettingsApiDoc::openapi()
}

#[utoipa::path(get, path = "/v1/settings/runtime", responses((status = 200, body = RuntimeSettingsResponse), (status = 500, body = ApiErrorResponse)))]
async fn get_runtime_settings(State(state): State<SettingsHttpState>) -> impl IntoResponse {
    match state.settings.get_runtime_settings().await {
        Ok(settings) => (StatusCode::OK, axum::Json(settings)).into_response(),
        Err(error) => settings_management_error_response(error),
    }
}

#[utoipa::path(put, path = "/v1/settings/runtime", request_body = UpdateRuntimeSettingsRequest, responses((status = 200, body = RuntimeSettingsResponse), (status = 400, body = ApiErrorResponse), (status = 500, body = ApiErrorResponse)))]
async fn update_runtime_settings(
    State(state): State<SettingsHttpState>,
    axum::Json(request): axum::Json<UpdateRuntimeSettingsRequest>,
) -> impl IntoResponse {
    match state.settings.update_runtime_settings(&request).await {
        Ok(settings) => (StatusCode::OK, axum::Json(settings)).into_response(),
        Err(error) => settings_management_error_response(error),
    }
}

#[utoipa::path(post, path = "/v1/settings/runtime/s3/test", request_body = UpdateRuntimeS3Settings, responses((status = 204), (status = 400, body = ApiErrorResponse), (status = 500, body = ApiErrorResponse)))]
async fn test_s3_connection(
    State(state): State<SettingsHttpState>,
    axum::Json(request): axum::Json<UpdateRuntimeS3Settings>,
) -> impl IntoResponse {
    match state.settings.test_s3_connection(&request).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => settings_management_error_response(error),
    }
}

#[utoipa::path(post, path = "/v1/settings/runtime/valkey/test", request_body = TestRuntimeValkeyRequest, responses((status = 204), (status = 400, body = ApiErrorResponse), (status = 500, body = ApiErrorResponse)))]
async fn test_valkey_connection(
    State(state): State<SettingsHttpState>,
    axum::Json(request): axum::Json<TestRuntimeValkeyRequest>,
) -> impl IntoResponse {
    match state.settings.test_valkey_connection(&request).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => settings_management_error_response(error),
    }
}

#[utoipa::path(post, path = "/v1/settings/runtime/embedding/test", request_body = TestRuntimeEmbeddingRequest, responses((status = 204), (status = 400, body = ApiErrorResponse), (status = 500, body = ApiErrorResponse)))]
async fn test_embedding_connection(
    State(state): State<SettingsHttpState>,
    axum::Json(request): axum::Json<TestRuntimeEmbeddingRequest>,
) -> impl IntoResponse {
    match state.settings.test_embedding_connection(&request).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => settings_management_error_response(error),
    }
}

/// A non-persisting connectivity check against a submitted Qdrant endpoint.
///
/// It reports reachability alone, so a reachable endpoint answers 204 and the
/// response body never discloses anything about the remote — not the collection
/// list, not the submitted URL.
///
/// Admin-only like the translation settings admin routes: the settings scope
/// admits any authenticated identity, so the handler rejects non-admins before
/// any outbound request is built.
#[utoipa::path(post, path = "/v1/settings/runtime/qdrant/test", request_body = TestRuntimeQdrantRequest, responses((status = 204), (status = 400, body = ApiErrorResponse), (status = 403, body = ApiErrorResponse), (status = 500, body = ApiErrorResponse)))]
async fn test_qdrant_connection(
    State(state): State<SettingsHttpState>,
    CurrentUser(user): CurrentUser,
    axum::Json(request): axum::Json<TestRuntimeQdrantRequest>,
) -> impl IntoResponse {
    if !user.is_admin {
        return qdrant_admin_required();
    }
    match state.settings.test_qdrant_connection(&request).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => settings_management_error_response(error),
    }
}

fn qdrant_admin_required() -> axum::response::Response {
    (
        StatusCode::FORBIDDEN,
        axum::Json(ApiErrorResponse::new(
            "forbidden",
            "administrator access required".to_string(),
        )),
    )
        .into_response()
}

#[utoipa::path(get, path = "/v1/settings/docling", responses((status = 200, body = DoclingSettingsResponse), (status = 500, body = ApiErrorResponse)))]
async fn get_docling_settings(State(state): State<SettingsHttpState>) -> impl IntoResponse {
    match state.settings.get_docling_settings().await {
        Ok(settings) => (StatusCode::OK, axum::Json(settings)).into_response(),
        Err(error) => internal_error_response(error),
    }
}

#[utoipa::path(put, path = "/v1/settings/docling", request_body = UpdateDoclingSettingsRequest, responses((status = 200, body = DoclingSettingsResponse), (status = 400, body = ApiErrorResponse), (status = 500, body = ApiErrorResponse)))]
async fn update_docling_settings(
    State(state): State<SettingsHttpState>,
    axum::Json(request): axum::Json<UpdateDoclingSettingsRequest>,
) -> impl IntoResponse {
    match state.settings.update_docling_settings(&request).await {
        Ok(settings) => (StatusCode::OK, axum::Json(settings)).into_response(),
        Err(error) => settings_management_error_response(error),
    }
}

/// A non-persisting connectivity check against the submitted Docling endpoint.
///
/// It reuses the connection settings block as its body, so the checked values
/// are exactly the ones a save would store, and it never returns them: a
/// reachable endpoint answers 204 and nothing else.
#[utoipa::path(post, path = "/v1/settings/docling/test", request_body = UpdateDoclingConnectionSettings, responses((status = 204), (status = 400, body = ApiErrorResponse), (status = 500, body = ApiErrorResponse)))]
async fn test_docling_connection(
    State(state): State<SettingsHttpState>,
    axum::Json(request): axum::Json<UpdateDoclingConnectionSettings>,
) -> impl IntoResponse {
    match state.settings.test_docling_connection(&request).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => settings_management_error_response(error),
    }
}

/// A non-persisting structured-call probe against submitted LLM draft values.
///
/// It exercises the exact unsaved endpoint/model/protocol/key with one
/// forced-tool extraction call and answers 200 with a typed outcome, so a
/// route misconfiguration surfaces before save. Probe failures are typed
/// outcome data, never upstream bodies or keys.
///
/// Admin-only like the Qdrant probe: the settings scope admits any
/// authenticated identity, so the handler rejects non-admins before any
/// outbound request is built.
#[utoipa::path(post, path = "/v1/settings/translation/providers/llm/test", request_body = TestLlmProviderRequest, responses((status = 200, body = TestLlmProviderResponse), (status = 400, body = ApiErrorResponse), (status = 403, body = ApiErrorResponse), (status = 500, body = ApiErrorResponse)))]
async fn test_llm_provider_connection(
    State(state): State<SettingsHttpState>,
    CurrentUser(user): CurrentUser,
    axum::Json(request): axum::Json<TestLlmProviderRequest>,
) -> impl IntoResponse {
    if !user.is_admin {
        return qdrant_admin_required();
    }
    match state.settings.test_llm_provider_connection(&request).await {
        Ok(response) => (StatusCode::OK, axum::Json(response)).into_response(),
        Err(error) => settings_management_error_response(error),
    }
}

#[utoipa::path(get, path = "/v1/settings/search", responses((status = 200, body = SearchSettingsResponse), (status = 500, body = ApiErrorResponse)))]
async fn get_search_settings(State(state): State<SettingsHttpState>) -> impl IntoResponse {
    match state.settings.get_search_settings().await {
        Ok(settings) => (StatusCode::OK, axum::Json(settings)).into_response(),
        Err(error) => internal_error_response(error),
    }
}

#[utoipa::path(put, path = "/v1/settings/search", request_body = CanonicalUpdateSearchSettingsRequest, responses((status = 200, body = SearchSettingsResponse), (status = 400, body = ApiErrorResponse), (status = 500, body = ApiErrorResponse)))]
async fn update_search_settings(
    State(state): State<SettingsHttpState>,
    axum::Json(request): axum::Json<CanonicalUpdateSearchSettingsRequest>,
) -> impl IntoResponse {
    match state.settings.update_search_settings(&request).await {
        Ok(settings) => (StatusCode::OK, axum::Json(settings)).into_response(),
        Err(error) => settings_management_error_response(error),
    }
}

fn settings_management_error_response(error: anyhow::Error) -> axum::response::Response {
    map_settings_error(error)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_error_mapping_uses_typed_codes() {
        use axum::http::StatusCode;
        let conflict = settings_management_error_response(
            context69_http_support::DomainError::conflict("vector rebuild already running").into(),
        );
        assert_eq!(conflict.status(), StatusCode::CONFLICT);
        let bad_request = settings_management_error_response(
            context69_http_support::DomainError::invalid_argument(
                "search.candidate_limit must be greater than 0",
            )
            .into(),
        );
        assert_eq!(bad_request.status(), StatusCode::BAD_REQUEST);
        let unknown = settings_management_error_response(anyhow::anyhow!("plain internal boom"));
        assert_eq!(unknown.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    /// The probe is a runtime-scoped admin route, so it must be published in the
    /// same document as the other runtime probes.
    ///
    /// The generated frontend client is typed from this document, so a missing
    /// path or an unregistered request schema would silently leave the settings
    /// page without a typed call to make.
    #[test]
    fn the_qdrant_probe_is_published_in_the_settings_openapi_document() {
        let document = openapi_document();
        let path = document
            .paths
            .paths
            .get("/v1/settings/runtime/qdrant/test")
            .expect("the probe route must be published");
        let operation = path.post.as_ref().expect("the probe must be a POST");

        for status in ["204", "400", "403", "500"] {
            assert!(
                operation.responses.responses.contains_key(status),
                "the probe must document its {status} response, got: {:?}",
                operation.responses.responses.keys().collect::<Vec<_>>()
            );
        }

        // The request body must reference the generated request type so the
        // generated frontend client types the call from the named schema.
        let body = operation
            .request_body
            .as_ref()
            .and_then(|request| request.content.get("application/json"))
            .and_then(|media| media.schema.as_ref())
            .expect("the probe must publish a request body");
        let reference = match body {
            utoipa::openapi::RefOr::Ref(reference) => reference.ref_location.as_str(),
            utoipa::openapi::RefOr::T(_) => {
                panic!("the probe request body must reference a named schema")
            }
        };
        assert!(
            reference.ends_with("TestRuntimeQdrantRequest"),
            "the request body must reference the generated request type, got: {reference}"
        );
        assert!(
            document
                .components
                .as_ref()
                .expect("the document must carry components")
                .schemas
                .contains_key("TestRuntimeQdrantRequest"),
            "the request type must be registered in the component schemas"
        );
    }

    /// The LLM probe is a settings-scoped admin route, so it must be published
    /// in the same document as the other probes.
    ///
    /// The generated frontend client is typed from this document, so a missing
    /// path or an unregistered request/response schema would silently leave
    /// the translation page without a typed call to make.
    #[test]
    fn the_llm_probe_is_published_in_the_settings_openapi_document() {
        let document = openapi_document();
        let path = document
            .paths
            .paths
            .get("/v1/settings/translation/providers/llm/test")
            .expect("the probe route must be published");
        let operation = path.post.as_ref().expect("the probe must be a POST");

        for status in ["200", "400", "403", "500"] {
            assert!(
                operation.responses.responses.contains_key(status),
                "the probe must document its {status} response, got: {:?}",
                operation.responses.responses.keys().collect::<Vec<_>>()
            );
        }

        let body = operation
            .request_body
            .as_ref()
            .and_then(|request| request.content.get("application/json"))
            .and_then(|media| media.schema.as_ref())
            .expect("the probe must publish a request body");
        let reference = match body {
            utoipa::openapi::RefOr::Ref(reference) => reference.ref_location.as_str(),
            utoipa::openapi::RefOr::T(_) => {
                panic!("the probe request body must reference a named schema")
            }
        };
        assert!(
            reference.ends_with("TestLlmProviderRequest"),
            "the request body must reference the generated request type, got: {reference}"
        );
        let components = document
            .components
            .as_ref()
            .expect("the document must carry components");
        for schema in [
            "TestLlmProviderRequest",
            "TestLlmProviderResponse",
            "TestLlmProviderOutcome",
        ] {
            assert!(
                components.schemas.contains_key(schema),
                "the {schema} type must be registered in the component schemas"
            );
        }
    }
}
