use context69_contracts::ApiErrorResponse;
use reqwest::{Method, RequestBuilder, Response, Url, header::AUTHORIZATION};

use super::{Context69Client, PERSONAL_ACCESS_TOKEN_PREFIX};
use crate::Error;

impl Context69Client {
    pub(crate) async fn authorized_request(
        &self,
        method: Method,
        path: &str,
    ) -> Result<RequestBuilder, Error> {
        let token = self
            .session
            .read()
            .await
            .personal_access_token
            .clone()
            .ok_or(Error::AuthenticationRequired)?;
        Ok(self
            .client
            .request(method, self.url(path)?)
            .header(AUTHORIZATION, format!("Bearer {token}")))
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
        let response = request.send().await?;
        if response.status().is_success() {
            return Ok(());
        }
        Err(self.build_http_error(response).await)
    }

    pub(crate) async fn read_json_response<T: serde::de::DeserializeOwned>(
        &self,
        response: Response,
    ) -> Result<T, Error> {
        if !response.status().is_success() {
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

pub(crate) fn encode_path_component(value: &str) -> String {
    url::form_urlencoded::byte_serialize(value.as_bytes()).collect()
}

pub(crate) fn group_path(group_path: &str, suffix: &str) -> String {
    format!(
        "/v1/groups/by-path/{}{}",
        encode_path_component(group_path),
        suffix
    )
}

pub(crate) fn validate_personal_access_token(token: String) -> Result<String, Error> {
    let trimmed = token.trim();
    if trimmed.is_empty() {
        return Err(Error::InvalidPersonalAccessToken(
            "personal access token must not be empty".to_string(),
        ));
    }
    if !trimmed.starts_with(PERSONAL_ACCESS_TOKEN_PREFIX) {
        return Err(Error::InvalidPersonalAccessToken(format!(
            "personal access token must start with {PERSONAL_ACCESS_TOKEN_PREFIX}"
        )));
    }
    Ok(trimmed.to_string())
}

pub(crate) fn parse_api_error_message(body: &str) -> Option<String> {
    serde_json::from_str::<ApiErrorResponse>(body)
        .ok()
        .map(|response| response.message)
}
