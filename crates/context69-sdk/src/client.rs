use std::{sync::Arc, time::Duration};

use context69_contracts::library::{
    CreateTextRequest, LibraryUploadResponse, UpsertLibraryTextRequest,
};
use context69_contracts::sources::SyncOutcome;
use context69_contracts::{
    ApiErrorResponse, AuthLoginRequest, AuthMeResponse, AuthTokenResponse, DocumentResponse,
    GroupMemberResponse, GroupResponse, HealthResponse, ProjectMemberResponse, ProjectResponse,
    SearchRequest, SearchResponse,
};
use reqwest::{
    Method, RequestBuilder, Response, StatusCode, Url,
    header::{AUTHORIZATION, USER_AGENT},
};
use tokio::sync::RwLock;

use crate::Error;

#[derive(Debug, Clone, Default)]
struct SessionState {
    access_token: Option<String>,
}

#[derive(Clone)]
pub struct Context69Client {
    client: reqwest::Client,
    base_url: Url,
    session: Arc<RwLock<SessionState>>,
}

pub struct Context69ClientBuilder {
    base_url: Option<Url>,
    user_agent: Option<String>,
    timeout: Option<Duration>,
    access_token: Option<String>,
}

impl Context69Client {
    pub fn builder() -> Context69ClientBuilder {
        Context69ClientBuilder {
            base_url: None,
            user_agent: None,
            timeout: None,
            access_token: None,
        }
    }

    pub fn with_access_token(&self, token: impl Into<String>) -> Context69Client {
        Context69Client {
            client: self.client.clone(),
            base_url: self.base_url.clone(),
            session: Arc::new(RwLock::new(SessionState {
                access_token: Some(token.into()),
            })),
        }
    }

    pub async fn login(
        &self,
        login_name: impl Into<String>,
        password: impl Into<String>,
    ) -> Result<AuthTokenResponse, Error> {
        let response = self
            .client
            .post(self.url("/v1/auth/login")?)
            .json(&AuthLoginRequest {
                login_name: login_name.into(),
                password: password.into(),
            })
            .send()
            .await?;
        let payload: AuthTokenResponse = self.read_json_response(response).await?;
        self.set_access_token(payload.access_token.clone()).await;
        Ok(payload)
    }

    pub async fn refresh(&self) -> Result<AuthTokenResponse, Error> {
        let response = self
            .client
            .post(self.url("/v1/auth/refresh")?)
            .send()
            .await?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            let message = parse_api_error_message(&body).unwrap_or_else(|| body.clone());
            return Err(Error::RefreshFailed {
                status: Some(status),
                message,
            });
        }

        let payload = response.json::<AuthTokenResponse>().await?;
        self.set_access_token(payload.access_token.clone()).await;
        Ok(payload)
    }

    pub async fn logout(&self) -> Result<(), Error> {
        let response = self
            .client
            .post(self.url("/v1/auth/logout")?)
            .send()
            .await?;
        self.read_empty_response(response).await?;
        self.clear_access_token().await;
        Ok(())
    }

    pub async fn me(&self) -> Result<AuthMeResponse, Error> {
        self.send_json(Method::GET, "/v1/auth/me", None::<&()>)
            .await
    }

    pub async fn healthz(&self) -> Result<HealthResponse, Error> {
        let response = self.client.get(self.url("/healthz")?).send().await?;
        self.read_json_response(response).await
    }

    pub async fn search(&self, request: SearchRequest) -> Result<SearchResponse, Error> {
        self.send_json(Method::POST, "/v1/search", Some(&request))
            .await
    }

    pub async fn get_document(&self, document_id: i64) -> Result<DocumentResponse, Error> {
        let path = format!("/v1/documents/{document_id}");
        self.send_json(Method::GET, &path, None::<&()>).await
    }

    pub async fn list_groups(&self) -> Result<Vec<GroupResponse>, Error> {
        self.send_json(Method::GET, "/v1/groups", None::<&()>).await
    }

    pub async fn get_group(&self, group_key: &str) -> Result<GroupResponse, Error> {
        let path = format!("/v1/groups/{group_key}");
        self.send_json(Method::GET, &path, None::<&()>).await
    }

    pub async fn list_projects(&self, group_key: &str) -> Result<Vec<ProjectResponse>, Error> {
        let path = format!("/v1/groups/{group_key}/projects");
        self.send_json(Method::GET, &path, None::<&()>).await
    }

    pub async fn get_project(
        &self,
        group_key: &str,
        project_key: &str,
    ) -> Result<ProjectResponse, Error> {
        let path = format!("/v1/groups/{group_key}/projects/{project_key}");
        self.send_json(Method::GET, &path, None::<&()>).await
    }

    pub async fn list_group_members(
        &self,
        group_key: &str,
    ) -> Result<Vec<GroupMemberResponse>, Error> {
        let path = format!("/v1/groups/{group_key}/members");
        self.send_json(Method::GET, &path, None::<&()>).await
    }

    pub async fn list_project_members(
        &self,
        group_key: &str,
        project_key: &str,
    ) -> Result<Vec<ProjectMemberResponse>, Error> {
        let path = format!("/v1/groups/{group_key}/projects/{project_key}/members");
        self.send_json(Method::GET, &path, None::<&()>).await
    }

    pub async fn list_sources(&self) -> Result<Vec<context69_contracts::SourceStatus>, Error> {
        let response: Vec<context69_contracts::SourceStatus> = self
            .send_json(Method::GET, "/v1/sources", None::<&()>)
            .await?;
        Ok(response)
    }

    pub async fn sync_source(&self, source_key: &str) -> Result<SyncOutcome, Error> {
        let path = format!("/v1/sources/{source_key}/sync");
        self.send_json(Method::POST, &path, None::<&()>).await
    }

    pub async fn create_project_library_text(
        &self,
        group_key: &str,
        project_key: &str,
        request: &CreateTextRequest,
    ) -> Result<LibraryUploadResponse, Error> {
        let path = format!("/v1/groups/{group_key}/projects/{project_key}/library/texts");
        self.send_json(Method::POST, &path, Some(request)).await
    }

    pub async fn upsert_project_library_text(
        &self,
        group_key: &str,
        project_key: &str,
        request: &UpsertLibraryTextRequest,
    ) -> Result<LibraryUploadResponse, Error> {
        let path = format!("/v1/groups/{group_key}/projects/{project_key}/library/texts");
        self.send_json(Method::PUT, &path, Some(request)).await
    }

    async fn send_json<TReq, TRes>(
        &self,
        method: Method,
        path: &str,
        body: Option<TReq>,
    ) -> Result<TRes, Error>
    where
        TReq: serde::Serialize,
        TRes: serde::de::DeserializeOwned,
    {
        self.ensure_authenticated().await?;
        let request_body = body
            .map(|value| serde_json::to_value(value))
            .transpose()
            .map_err(Error::from)?;

        let response = self
            .send_with_refresh(method.clone(), path, request_body.clone())
            .await?;
        self.read_json_response(response).await
    }

    async fn send_with_refresh(
        &self,
        method: Method,
        path: &str,
        body: Option<serde_json::Value>,
    ) -> Result<Response, Error> {
        let response = self
            .send_request(method.clone(), path, body.clone(), true)
            .await?;

        if response.status() != StatusCode::UNAUTHORIZED {
            return Ok(response);
        }

        let _ = response.bytes().await;

        match self.refresh().await {
            Ok(_) => self.send_request(method, path, body, true).await,
            Err(Error::RefreshFailed { status, message }) => {
                Err(Error::RefreshFailed { status, message })
            }
            Err(other) => Err(Error::RefreshFailed {
                status: None,
                message: other.to_string(),
            }),
        }
    }

    async fn send_request(
        &self,
        method: Method,
        path: &str,
        body: Option<serde_json::Value>,
        include_auth: bool,
    ) -> Result<Response, Error> {
        let url = self.url(path)?;
        let mut request = self.client.request(method, url);
        if include_auth {
            request = self.authorized(request).await?;
        }
        if let Some(body) = body {
            request = request.json(&body);
        }
        Ok(request.send().await?)
    }

    async fn authorized(&self, request: RequestBuilder) -> Result<RequestBuilder, Error> {
        let token = self
            .session
            .read()
            .await
            .access_token
            .clone()
            .ok_or(Error::AuthenticationRequired)?;
        Ok(request.header(AUTHORIZATION, format!("Bearer {token}")))
    }

    async fn ensure_authenticated(&self) -> Result<(), Error> {
        if self.session.read().await.access_token.is_some() {
            Ok(())
        } else {
            Err(Error::AuthenticationRequired)
        }
    }

    fn url(&self, path: &str) -> Result<Url, Error> {
        self.base_url
            .join(path.trim_start_matches('/'))
            .map_err(|source| Error::UrlJoin {
                path: path.to_string(),
                source,
            })
    }

    async fn set_access_token(&self, token: String) {
        self.session.write().await.access_token = Some(token);
    }

    async fn clear_access_token(&self) {
        self.session.write().await.access_token = None;
    }

    async fn read_empty_response(&self, response: Response) -> Result<(), Error> {
        let status = response.status();
        if status.is_success() {
            return Ok(());
        }
        Err(self.build_http_error(response).await)
    }

    async fn read_json_response<T: serde::de::DeserializeOwned>(
        &self,
        response: Response,
    ) -> Result<T, Error> {
        let status = response.status();
        if !status.is_success() {
            return Err(self.build_http_error(response).await);
        }
        Ok(response.json::<T>().await?)
    }

    async fn build_http_error(&self, response: Response) -> Error {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        Error::HttpStatus {
            status,
            api_error: parse_api_error_message(&body),
            body,
        }
    }
}

impl Context69ClientBuilder {
    pub fn base_url(mut self, base_url: &str) -> Result<Self, Error> {
        let mut url =
            Url::parse(base_url).map_err(|_| Error::InvalidBaseUrl(base_url.to_string()))?;
        if !url.path().ends_with('/') {
            let next_path = format!("{}/", url.path());
            url.set_path(&next_path);
        }
        self.base_url = Some(url);
        Ok(self)
    }

    pub fn user_agent(mut self, user_agent: impl Into<String>) -> Self {
        self.user_agent = Some(user_agent.into());
        self
    }

    pub fn timeout(mut self, timeout: Duration) -> Result<Self, Error> {
        if timeout.is_zero() {
            return Err(Error::InvalidTimeout(timeout));
        }
        self.timeout = Some(timeout);
        Ok(self)
    }

    pub fn with_access_token(mut self, token: impl Into<String>) -> Self {
        self.access_token = Some(token.into());
        self
    }

    pub fn build(self) -> Result<Context69Client, Error> {
        let base_url = self
            .base_url
            .ok_or_else(|| Error::InvalidBaseUrl("missing base_url".to_string()))?;
        let mut builder = reqwest::Client::builder().cookie_store(true);
        if let Some(user_agent) = self.user_agent {
            builder = builder.default_headers({
                let mut headers = reqwest::header::HeaderMap::new();
                headers.insert(
                    USER_AGENT,
                    user_agent
                        .parse()
                        .map_err(|_| Error::InvalidHeader(user_agent.clone()))?,
                );
                headers
            });
        }
        if let Some(timeout) = self.timeout {
            builder = builder.timeout(timeout);
        }
        let client = builder.build()?;
        Ok(Context69Client {
            client,
            base_url,
            session: Arc::new(RwLock::new(SessionState {
                access_token: self.access_token,
            })),
        })
    }
}

fn parse_api_error_message(body: &str) -> Option<String> {
    serde_json::from_str::<ApiErrorResponse>(body)
        .ok()
        .map(|value| value.error)
}

#[cfg(test)]
mod tests {
    use std::assert_matches;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    use super::*;
    use axum::{
        Json, Router,
        extract::State,
        http::{HeaderMap, HeaderValue, StatusCode, header},
        response::{IntoResponse, Response},
        routing::{get, post},
    };
    use chrono::NaiveDate;
    use context69_contracts::library::{
        CreateTextRequest, LibraryFileSummary, LibraryIngestJobResponse, LibraryIngestStatus,
        LibraryUploadResponse, UpsertLibraryTextRequest,
    };
    use context69_contracts::sources::SyncOutcome;
    use context69_contracts::{
        AuthUserResponse, GroupKind, HealthStatus, MembershipRole, SearchHit, Visibility,
    };
    use serde_json::json;
    use tokio::net::TcpListener;

    #[derive(Clone, Default)]
    struct TestState {
        search_calls: Arc<AtomicUsize>,
        refresh_calls: Arc<AtomicUsize>,
        create_text_calls: Arc<AtomicUsize>,
        upsert_text_calls: Arc<AtomicUsize>,
        sync_calls: Arc<AtomicUsize>,
    }

    async fn spawn_test_server() -> (String, TestState) {
        let state = TestState::default();
        let app = Router::new()
            .route("/healthz", get(health_handler))
            .route("/v1/auth/login", post(login_handler))
            .route("/v1/auth/refresh", post(refresh_handler))
            .route("/v1/search", post(search_handler))
            .route("/v1/groups", get(groups_handler))
            .route(
                "/v1/groups/{group_key}/projects/{project_key}/library/texts",
                post(create_project_library_text_handler).put(upsert_project_library_text_handler),
            )
            .route("/v1/sources/{source_key}/sync", post(sync_source_handler))
            .with_state(state.clone());

        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind listener");
        let addr = listener.local_addr().expect("local addr");
        tokio::spawn(async move {
            axum::serve(listener, app).await.expect("serve app");
        });
        (format!("http://{addr}"), state)
    }

    async fn health_handler() -> Json<HealthResponse> {
        Json(HealthResponse {
            status: HealthStatus::Ok,
            indexed_chunks: Some(7),
            db_ok: None,
            qdrant_ok: None,
        })
    }

    fn json_response<T>(status: StatusCode, value: &T) -> Response
    where
        T: serde::Serialize,
    {
        (status, Json(value)).into_response()
    }

    fn json_response_with_cookie<T>(
        status: StatusCode,
        value: &T,
        set_cookie: &'static str,
    ) -> Response
    where
        T: serde::Serialize,
    {
        let mut response = json_response(status, value);
        response
            .headers_mut()
            .append(header::SET_COOKIE, HeaderValue::from_static(set_cookie));
        response
    }

    async fn login_handler() -> Response {
        json_response_with_cookie(
            StatusCode::OK,
            &token_response("token-initial"),
            "context69_refresh=refresh-ok; HttpOnly; Path=/",
        )
    }

    async fn refresh_handler(State(state): State<TestState>) -> Response {
        state.refresh_calls.fetch_add(1, Ordering::SeqCst);
        json_response_with_cookie(
            StatusCode::OK,
            &token_response("token-refreshed"),
            "context69_refresh=refresh-ok; HttpOnly; Path=/",
        )
    }

    async fn search_handler(
        State(state): State<TestState>,
        headers: HeaderMap,
        Json(request): Json<SearchRequest>,
    ) -> Response {
        let call = state.search_calls.fetch_add(1, Ordering::SeqCst);
        let bearer = headers
            .get(AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default();

        if request.query == "bad request" {
            return json_response(
                StatusCode::BAD_REQUEST,
                &ApiErrorResponse {
                    error: "invalid query".to_string(),
                },
            );
        }

        if call == 0 && bearer == "Bearer token-initial" {
            return json_response(
                StatusCode::UNAUTHORIZED,
                &ApiErrorResponse {
                    error: "expired".to_string(),
                },
            );
        }

        if bearer != "Bearer token-refreshed" {
            return json_response(
                StatusCode::UNAUTHORIZED,
                &ApiErrorResponse {
                    error: "missing bearer token".to_string(),
                },
            );
        }

        json_response(
            StatusCode::OK,
            &SearchResponse {
                query: request.query,
                hits: vec![SearchHit {
                    chunk_id: uuid::Uuid::nil(),
                    document_id: 42,
                    group_key: "team".to_string(),
                    project_key: "docs".to_string(),
                    visibility: Visibility::Private,
                    source_key: "source".to_string(),
                    external_id: "ext-1".to_string(),
                    title: "Document".to_string(),
                    summary: Some("Summary".to_string()),
                    source_uri: "https://example.test/doc".to_string(),
                    published_at: None,
                    chunk_index: 0,
                    chunk_text: "hello".to_string(),
                    score: 0.9,
                    vector_score: Some(0.9),
                    keyword_score: None,
                    rerank_score: None,
                    match_reason: None,
                    metadata_json: json!({}),
                    library_file_id: None,
                    library_section_label: None,
                    library_path: None,
                    is_library_file: false,
                }],
            },
        )
    }

    async fn groups_handler(headers: HeaderMap) -> Response {
        let bearer = headers
            .get(AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default();
        if bearer != "Bearer token-refreshed" {
            return json_response(
                StatusCode::UNAUTHORIZED,
                &ApiErrorResponse {
                    error: "missing bearer token".to_string(),
                },
            );
        }

        json_response(
            StatusCode::OK,
            &vec![GroupResponse {
                group_id: 1,
                group_key: "team".to_string(),
                parent_group_key: None,
                name: "Team".to_string(),
                visibility: Visibility::Private,
                kind: GroupKind::Shared,
                current_role: Some(MembershipRole::Owner),
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
            }],
        )
    }

    fn require_authorized(headers: &HeaderMap) -> Result<(), StatusCode> {
        let bearer = headers
            .get(AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default();
        if bearer == "Bearer token-refreshed" || bearer == "Bearer manual-token" {
            Ok(())
        } else {
            Err(StatusCode::UNAUTHORIZED)
        }
    }

    async fn create_project_library_text_handler(
        State(state): State<TestState>,
        headers: HeaderMap,
        Json(request): Json<CreateTextRequest>,
    ) -> Response {
        state.create_text_calls.fetch_add(1, Ordering::SeqCst);
        if require_authorized(&headers).is_err() {
            return json_response(
                StatusCode::UNAUTHORIZED,
                &ApiErrorResponse {
                    error: "missing bearer token".to_string(),
                },
            );
        }
        assert_eq!(request.title, "Hello");
        json_response(
            StatusCode::OK,
            &sample_library_upload("created-file", "created-job"),
        )
    }

    async fn upsert_project_library_text_handler(
        State(state): State<TestState>,
        headers: HeaderMap,
        Json(request): Json<UpsertLibraryTextRequest>,
    ) -> Response {
        state.upsert_text_calls.fetch_add(1, Ordering::SeqCst);
        if require_authorized(&headers).is_err() {
            return json_response(
                StatusCode::UNAUTHORIZED,
                &ApiErrorResponse {
                    error: "missing bearer token".to_string(),
                },
            );
        }
        assert_eq!(request.external_id, "ext-1");
        assert_eq!(
            request.published_at,
            Some(NaiveDate::from_ymd_opt(2026, 6, 10).expect("date"))
        );
        assert_eq!(request.metadata_json, json!({"topic":"gov"}));
        json_response(
            StatusCode::OK,
            &sample_library_upload("upserted-file", "upserted-job"),
        )
    }

    async fn sync_source_handler(State(state): State<TestState>, headers: HeaderMap) -> Response {
        state.sync_calls.fetch_add(1, Ordering::SeqCst);
        if require_authorized(&headers).is_err() {
            return json_response(
                StatusCode::UNAUTHORIZED,
                &ApiErrorResponse {
                    error: "missing bearer token".to_string(),
                },
            );
        }
        json_response(
            StatusCode::OK,
            &SyncOutcome {
                records_seen: 10,
                records_changed: 4,
                chunks_upserted: 12,
            },
        )
    }

    fn token_response(access_token: &str) -> AuthTokenResponse {
        AuthTokenResponse {
            access_token: access_token.to_string(),
            token_type: "Bearer".to_string(),
            expires_in_secs: 3600,
            user: AuthUserResponse {
                user_id: 1,
                login_name: "admin".to_string(),
                display_name: "Administrator".to_string(),
                is_admin: true,
                disabled_at: None,
                personal_group_key: "admin".to_string(),
                personal_group_role: Some(MembershipRole::Owner),
            },
        }
    }

    fn sample_library_upload(file_id: &str, job_id: &str) -> LibraryUploadResponse {
        LibraryUploadResponse {
            files: vec![LibraryFileSummary {
                file_id: uuid::Uuid::parse_str("11111111-1111-1111-1111-111111111111")
                    .expect("uuid"),
                group_key: "team".to_string(),
                project_key: "docs".to_string(),
                visibility: Visibility::Private,
                folder_id: None,
                filename: file_id.to_string(),
                media_type: "text/plain".to_string(),
                size_bytes: 5,
                ingest_status: LibraryIngestStatus::Succeeded,
                error_message: None,
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
                ingested_at: Some(chrono::Utc::now()),
            }],
            jobs: vec![LibraryIngestJobResponse {
                job_id: uuid::Uuid::parse_str("22222222-2222-2222-2222-222222222222")
                    .expect("uuid"),
                group_key: "team".to_string(),
                project_key: "docs".to_string(),
                visibility: Visibility::Private,
                file_id: uuid::Uuid::parse_str("11111111-1111-1111-1111-111111111111")
                    .expect("uuid"),
                status: LibraryIngestStatus::Succeeded,
                docling_task_id: Some(job_id.to_string()),
                error_message: None,
                created_at: chrono::Utc::now(),
                started_at: Some(chrono::Utc::now()),
                finished_at: Some(chrono::Utc::now()),
                updated_at: chrono::Utc::now(),
            }],
        }
    }

    #[test]
    fn builder_normalizes_base_url_with_trailing_slash() {
        let client = Context69Client::builder()
            .base_url("http://localhost:8096")
            .expect("base url")
            .build()
            .expect("client");

        assert_eq!(
            client.url("/healthz").expect("url").as_str(),
            "http://localhost:8096/healthz"
        );
    }

    #[test]
    fn parse_api_error_body() {
        let body = r#"{"error":"missing bearer token"}"#;
        assert_eq!(
            parse_api_error_message(body),
            Some("missing bearer token".to_string())
        );
    }

    #[tokio::test]
    async fn protected_api_requires_login() {
        let (base_url, _) = spawn_test_server().await;
        let client = Context69Client::builder()
            .base_url(&base_url)
            .expect("base url")
            .build()
            .expect("client");

        let error = client
            .list_groups()
            .await
            .expect_err("should require authentication");
        assert_matches!(error, Error::AuthenticationRequired);
    }

    #[tokio::test]
    async fn search_refreshes_once_and_retries() {
        let (base_url, state) = spawn_test_server().await;
        let client = Context69Client::builder()
            .base_url(&base_url)
            .expect("base url")
            .build()
            .expect("client");

        client.login("admin", "secret").await.expect("login");
        let response = client
            .search(SearchRequest {
                query: "policy".to_string(),
                limit: 8,
                source_key: None,
                group_key: None,
                project_key: None,
                published_after: None,
                published_before: None,
            })
            .await
            .expect("search response");

        assert_eq!(response.hits.len(), 1);
        assert_eq!(state.refresh_calls.load(Ordering::SeqCst), 1);
        assert_eq!(state.search_calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn surfaces_api_error_message() {
        let (base_url, _) = spawn_test_server().await;
        let client = Context69Client::builder()
            .base_url(&base_url)
            .expect("base url")
            .build()
            .expect("client");

        client.login("admin", "secret").await.expect("login");
        let error = client
            .search(SearchRequest {
                query: "bad request".to_string(),
                limit: 8,
                source_key: None,
                group_key: None,
                project_key: None,
                published_after: None,
                published_before: None,
            })
            .await
            .expect_err("should fail");

        match error {
            Error::HttpStatus {
                status, api_error, ..
            } => {
                assert_eq!(status, StatusCode::BAD_REQUEST);
                assert_eq!(api_error.as_deref(), Some("invalid query"));
            }
            other => panic!("unexpected error: {other}"),
        }
    }

    #[tokio::test]
    async fn builder_access_token_can_call_protected_api() {
        let (base_url, state) = spawn_test_server().await;
        let client = Context69Client::builder()
            .base_url(&base_url)
            .expect("base url")
            .with_access_token("manual-token")
            .build()
            .expect("client");

        let outcome = client
            .sync_source("gov_documents")
            .await
            .expect("sync response");
        assert_eq!(outcome.records_changed, 4);
        assert_eq!(state.sync_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn derived_client_access_token_overrides_session() {
        let (base_url, state) = spawn_test_server().await;
        let client = Context69Client::builder()
            .base_url(&base_url)
            .expect("base url")
            .build()
            .expect("client");
        let derived = client.with_access_token("manual-token");

        let response = derived
            .create_project_library_text(
                "team",
                "docs",
                &CreateTextRequest {
                    folder_id: None,
                    title: "Hello".to_string(),
                    content: "world".to_string(),
                    source_uri: None,
                    summary: None,
                },
            )
            .await
            .expect("create response");

        assert_eq!(response.files[0].filename, "created-file");
        assert_eq!(state.create_text_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn upsert_project_library_text_sends_request_body() {
        let (base_url, state) = spawn_test_server().await;
        let client = Context69Client::builder()
            .base_url(&base_url)
            .expect("base url")
            .with_access_token("manual-token")
            .build()
            .expect("client");

        let response = client
            .upsert_project_library_text(
                "team",
                "docs",
                &UpsertLibraryTextRequest {
                    external_id: "ext-1".to_string(),
                    folder_id: None,
                    title: "Hello".to_string(),
                    content: "world".to_string(),
                    source_uri: None,
                    summary: None,
                    published_at: Some(NaiveDate::from_ymd_opt(2026, 6, 10).expect("published_at")),
                    metadata_json: json!({"topic":"gov"}),
                },
            )
            .await
            .expect("upsert response");

        assert_eq!(response.files[0].filename, "upserted-file");
        assert_eq!(state.upsert_text_calls.load(Ordering::SeqCst), 1);
    }
}
