mod library;
mod search;
mod settings;
mod sources;
mod workspace;

use std::{sync::Arc, time::Duration};

use context69_contracts::{ApiErrorResponse, AuthMeResponse, HealthResponse};
use reqwest::{
    Method, RequestBuilder, Response, Url,
    header::{AUTHORIZATION, USER_AGENT},
    multipart::{Form, Part},
};
use tokio::sync::RwLock;
use uuid::Uuid;

use crate::Error;
pub use library::LibraryApi;
pub use search::SearchApi;
pub use settings::SettingsApi;
pub use sources::SourcesApi;
pub use workspace::WorkspaceApi;

pub(crate) const PERSONAL_ACCESS_TOKEN_PREFIX: &str = "ctx_pat_";

#[derive(Debug, Clone, Default)]
struct SessionState {
    personal_access_token: Option<String>,
}

#[derive(Clone)]
pub struct Context69Client {
    client: reqwest::Client,
    base_url: Url,
    session: Arc<RwLock<SessionState>>,
}

#[derive(Debug)]
pub struct Context69ClientBuilder {
    base_url: Option<Url>,
    user_agent: Option<String>,
    timeout: Option<Duration>,
    personal_access_token: Option<String>,
}

impl Context69Client {
    pub fn builder() -> Context69ClientBuilder {
        Context69ClientBuilder {
            base_url: None,
            user_agent: None,
            timeout: None,
            personal_access_token: None,
        }
    }

    pub fn with_personal_access_token(
        &self,
        token: impl Into<String>,
    ) -> Result<Context69Client, Error> {
        Ok(Context69Client {
            client: self.client.clone(),
            base_url: self.base_url.clone(),
            session: Arc::new(RwLock::new(SessionState {
                personal_access_token: Some(validate_personal_access_token(token.into())?),
            })),
        })
    }

    pub fn workspace(&self) -> WorkspaceApi<'_> {
        WorkspaceApi::new(self)
    }

    pub fn sources(&self) -> SourcesApi<'_> {
        SourcesApi::new(self)
    }

    pub fn library(&self) -> LibraryApi<'_> {
        LibraryApi::new(self)
    }

    pub fn settings(&self) -> SettingsApi<'_> {
        SettingsApi::new(self)
    }

    pub fn search(&self) -> SearchApi<'_> {
        SearchApi::new(self)
    }

    pub async fn me(&self) -> Result<AuthMeResponse, Error> {
        self.execute_json(self.authorized_request(Method::GET, "/v1/auth/me").await?)
            .await
    }

    pub async fn healthz(&self) -> Result<HealthResponse, Error> {
        let response = self.client.get(self.url("/healthz")?).send().await?;
        self.read_json_response(response).await
    }

    pub(crate) async fn authorized_request(
        &self,
        method: Method,
        path: &str,
    ) -> Result<RequestBuilder, Error> {
        let personal_access_token = self
            .session
            .read()
            .await
            .personal_access_token
            .clone()
            .ok_or(Error::AuthenticationRequired)?;
        Ok(self
            .client
            .request(method, self.url(path)?)
            .header(AUTHORIZATION, format!("Bearer {personal_access_token}")))
    }

    pub(crate) async fn authorized_url_request(
        &self,
        method: Method,
        url: Url,
    ) -> Result<RequestBuilder, Error> {
        let personal_access_token = self
            .session
            .read()
            .await
            .personal_access_token
            .clone()
            .ok_or(Error::AuthenticationRequired)?;
        Ok(self
            .client
            .request(method, url)
            .header(AUTHORIZATION, format!("Bearer {personal_access_token}")))
    }

    pub(crate) fn url(&self, path: &str) -> Result<Url, Error> {
        self.base_url
            .join(path.trim_start_matches('/'))
            .map_err(|source| Error::UrlJoin {
                path: path.to_string(),
                source,
            })
    }

    pub(crate) async fn execute_json<T: serde::de::DeserializeOwned>(
        &self,
        request: RequestBuilder,
    ) -> Result<T, Error> {
        self.read_json_response(request.send().await?).await
    }

    pub(crate) async fn execute_empty(&self, request: RequestBuilder) -> Result<(), Error> {
        self.read_empty_response(request.send().await?).await
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

    pub fn with_personal_access_token(mut self, token: impl Into<String>) -> Result<Self, Error> {
        self.personal_access_token = Some(validate_personal_access_token(token.into())?);
        Ok(self)
    }

    pub fn build(self) -> Result<Context69Client, Error> {
        let base_url = self
            .base_url
            .ok_or_else(|| Error::InvalidBaseUrl("missing base_url".to_string()))?;
        let mut builder = reqwest::Client::builder();
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
                personal_access_token: self.personal_access_token,
            })),
        })
    }
}

pub(crate) fn file_upload_form(folder_id: Option<Uuid>, files: Vec<Part>) -> Form {
    let mut form = Form::new();
    if let Some(folder_id) = folder_id {
        form = form.text("folder_id", folder_id.to_string());
    }
    for file in files {
        form = form.part("files", file);
    }
    form
}

pub(crate) fn encode_path_component(value: &str) -> String {
    url::form_urlencoded::byte_serialize(value.as_bytes()).collect()
}

pub(crate) fn validate_personal_access_token(token: String) -> Result<String, Error> {
    let trimmed = token.trim();
    if trimmed.is_empty() {
        return Err(Error::InvalidPersonalAccessToken(
            "personal access token must not be empty".to_string(),
        ));
    }
    if !trimmed.starts_with(PERSONAL_ACCESS_TOKEN_PREFIX) {
        return Err(Error::InvalidPersonalAccessToken(
            "expected personal access token with ctx_pat_ prefix".to_string(),
        ));
    }
    Ok(trimmed.to_string())
}

pub(crate) fn parse_api_error_message(body: &str) -> Option<String> {
    serde_json::from_str::<ApiErrorResponse>(body)
        .ok()
        .map(|value| value.error)
}

#[cfg(test)]
mod tests;
