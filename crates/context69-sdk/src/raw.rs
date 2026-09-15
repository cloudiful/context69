//! OpenAPI-aligned raw transport (Redmine 362 Task 4b, sdk-raw).
//!
//! The high-level facade stays ergonomic and partial; this module is the
//! complete low-level surface. Every HTTP operation in the current OpenAPI
//! appears exactly once in [`OPERATIONS`] (see `raw_operations.rs`, generated
//! mechanically and sorted by `operation_id`). Callers build a [`RawRequest`]
//! with explicit path/query/body helpers and run it through [`RawClient`],
//! which encodes paths with the shared transport helper, applies bearer auth
//! per-operation, and decodes JSON or error envelopes without duplicating
//! service logic.
//!
//! Honest limitation: with the current dependencies (`reqwest` without the
//! `multipart` feature) the two `multipart/form-data` uploads
//! (`upload_library_files`, `upload_group_library_files`) and the SSE streams
//! (`search_stream`, `stream_tasks`) have no typed builder/parser. They are
//! covered in the registry with `BodyKind::Multipart`/empty response, and
//! callers pass pre-encoded bytes via [`RawRequest::raw_body`] or read SSE
//! bytes from [`RawResponse`]. Per-operation typed request/response structs
//! are intentionally not generated; bodies use shared contract types through
//! [`RawRequest::json_body`] and [`RawResponse::decode`].

pub use crate::raw_operations::{BodyKind, OPERATIONS, Operation};
use crate::{Context69Client, Error};

/// Find an operation by its OpenAPI `operation_id`.
pub fn find_operation(id: &str) -> Option<&'static Operation> {
    OPERATIONS.iter().find(|op| op.id == id)
}

/// All operation ids in registry order (sorted for determinism).
pub fn operation_ids() -> Vec<&'static str> {
    OPERATIONS.iter().map(|op| op.id).collect()
}

/// Stable `Idempotency-Key` helper shared with the facade convention.
pub fn stable_idempotency_key(path: &str, body_bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};

    let mut hasher = Sha256::new();
    hasher.update(path.as_bytes());
    hasher.update([0]);
    hasher.update(body_bytes);
    format!(
        "ctx69-sdk-{}",
        hasher
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    )
}

/// One OpenAPI operation invocation without handwritten service logic.
#[derive(Debug, Clone)]
pub struct RawRequest {
    operation: &'static Operation,
    path_params: Vec<(String, String)>,
    query_params: Vec<(String, String)>,
    json_body: Option<serde_json::Value>,
    raw_body: Option<(String, Vec<u8>)>,
    idempotency_key: Option<String>,
}

impl RawRequest {
    /// Build a request for `operation_id`, or fail when unknown.
    pub fn new(operation_id: &str) -> Result<Self, Error> {
        let operation = find_operation(operation_id).ok_or_else(|| {
            Error::InvalidResponse(format!("unknown operation_id: {operation_id}"))
        })?;
        Ok(Self {
            operation,
            path_params: Vec::new(),
            query_params: Vec::new(),
            json_body: None,
            raw_body: None,
            idempotency_key: None,
        })
    }

    /// Borrow the resolved operation metadata.
    pub fn operation(&self) -> &'static Operation {
        self.operation
    }

    /// Set one `{name}` path placeholder. Values are percent-encoded with the
    /// shared transport helper when the path is rendered.
    pub fn path_param(mut self, name: &str, value: impl ToString) -> Self {
        match self.path_params.iter_mut().find(|(key, _)| key == name) {
            Some(slot) => slot.1 = value.to_string(),
            None => self.path_params.push((name.to_string(), value.to_string())),
        }
        self
    }

    /// Push one query pair. Callers add each pair explicitly; nothing is hidden.
    pub fn query(mut self, name: &str, value: impl ToString) -> Self {
        self.query_params
            .push((name.to_string(), value.to_string()));
        self
    }

    /// Push one query pair only when `value` is `Some`.
    pub fn query_opt<T: ToString>(mut self, name: &str, value: Option<T>) -> Self {
        if let Some(value) = value {
            self.query_params
                .push((name.to_string(), value.to_string()));
        }
        self
    }

    /// Set a JSON body from any shared contract type.
    pub fn json_body<T: serde::Serialize>(mut self, body: &T) -> Result<Self, Error> {
        self.json_body = Some(serde_json::to_value(body)?);
        self.raw_body = None;
        Ok(self)
    }

    /// Set a pre-built JSON body value.
    pub fn json_value(mut self, value: serde_json::Value) -> Self {
        self.json_body = Some(value);
        self.raw_body = None;
        self
    }

    /// Set pre-encoded bytes (multipart bodies, SSE payloads, or tests).
    pub fn raw_body(mut self, content_type: &str, bytes: Vec<u8>) -> Self {
        self.raw_body = Some((content_type.to_string(), bytes));
        self.json_body = None;
        self
    }

    /// Attach an explicit `Idempotency-Key` header value.
    pub fn idempotency_key(mut self, key: &str) -> Self {
        self.idempotency_key = Some(key.to_string());
        self
    }

    /// Derive a stable `ctx69-sdk-*` key from the rendered path plus body.
    pub fn with_auto_idempotency_key(mut self) -> Result<Self, Error> {
        let path = self.render_path()?;
        let body_bytes = self.body_bytes()?;
        self.idempotency_key = Some(stable_idempotency_key(&path, &body_bytes));
        Ok(self)
    }

    /// Path params in insertion order (for tests and debugging).
    pub fn path_params(&self) -> &[(String, String)] {
        &self.path_params
    }

    /// Query pairs in insertion order (for tests and debugging).
    pub fn query_params(&self) -> &[(String, String)] {
        &self.query_params
    }

    /// Attached idempotency key, if any.
    pub fn idempotency_key_value(&self) -> Option<&str> {
        self.idempotency_key.as_deref()
    }

    /// Render `path_template` with percent-encoded path params.
    pub fn render_path(&self) -> Result<String, Error> {
        let mut rendered = self.operation.path_template.to_string();
        for (name, value) in &self.path_params {
            let placeholder = format!("{{{name}}}");
            if !rendered.contains(&placeholder) {
                return Err(Error::InvalidResponse(format!(
                    "unknown path param '{name}' for {}",
                    self.operation.id
                )));
            }
            let encoded = crate::client::transport::encode_path_component(value);
            rendered = rendered.replace(&placeholder, &encoded);
        }
        if rendered.contains('{') || rendered.contains('}') {
            return Err(Error::InvalidResponse(format!(
                "missing path param for {}: {rendered}",
                self.operation.id
            )));
        }
        Ok(rendered)
    }

    fn body_bytes(&self) -> Result<Vec<u8>, Error> {
        if let Some(value) = &self.json_body {
            Ok(serde_json::to_vec(value)?)
        } else if let Some((_, bytes)) = &self.raw_body {
            Ok(bytes.clone())
        } else {
            Ok(Vec::new())
        }
    }
}

/// Raw HTTP result. Success bodies decode with [`RawResponse::decode`];
/// error statuses become [`Error::HttpStatus`] there or in `execute_decode`.
#[derive(Debug, Clone)]
pub struct RawResponse {
    pub operation_id: &'static str,
    pub status: reqwest::StatusCode,
    pub body: Vec<u8>,
}

impl RawResponse {
    /// True for 2xx statuses.
    pub fn is_success(&self) -> bool {
        self.status.is_success()
    }

    /// Body as UTF-8 text.
    pub fn body_text(&self) -> Result<&str, Error> {
        std::str::from_utf8(&self.body)
            .map_err(|error| Error::InvalidResponse(format!("response body is not utf-8: {error}")))
    }

    fn status_error(&self) -> Error {
        let body = String::from_utf8_lossy(&self.body).to_string();
        Error::HttpStatus {
            status: self.status,
            api_error: crate::client::transport::parse_api_error_message(&body),
            body,
        }
    }

    /// Decode a success JSON body into a shared contract type.
    pub fn decode<T: serde::de::DeserializeOwned>(&self) -> Result<T, Error> {
        if !self.status.is_success() {
            return Err(self.status_error());
        }
        if self.body.is_empty() {
            return Err(Error::InvalidResponse(format!(
                "empty body for {}",
                self.operation_id
            )));
        }
        Ok(serde_json::from_slice(&self.body)?)
    }

    /// Accept any 2xx status with no JSON body (204/empty/SSE handled by caller).
    pub fn decode_empty(&self) -> Result<(), Error> {
        if !self.status.is_success() {
            return Err(self.status_error());
        }
        Ok(())
    }
}

/// Borrowed raw executor. Created with [`Context69Client::raw`].
pub struct RawClient<'a> {
    inner: &'a Context69Client,
}

impl Context69Client {
    /// Access the complete OpenAPI-aligned raw transport.
    pub fn raw(&self) -> RawClient<'_> {
        RawClient { inner: self }
    }
}

impl RawClient<'_> {
    /// Execute one [`RawRequest`] and return the raw status plus bytes.
    /// Transport/auth/url failures are `Err`; HTTP error statuses stay `Ok`
    /// so callers can inspect them or map them with `decode`.
    pub async fn execute(&self, request: &RawRequest) -> Result<RawResponse, Error> {
        use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};

        let path = request.render_path()?;
        let url = self
            .inner
            .base_url_ref()
            .join(path.trim_start_matches('/'))
            .map_err(|source| Error::UrlJoin {
                path: path.clone(),
                source,
            })?;
        let method =
            reqwest::Method::from_bytes(request.operation.method.as_bytes()).map_err(|_| {
                Error::InvalidResponse(format!(
                    "unknown method {} for {}",
                    request.operation.method, request.operation.id
                ))
            })?;
        let mut builder = self.inner.http_client().request(method, url);
        if request.operation.requires_auth {
            let token = self
                .inner
                .bearer_token()
                .await
                .ok_or(Error::AuthenticationRequired)?;
            builder = builder.header(AUTHORIZATION, format!("Bearer {token}"));
        }
        if !request.query_params.is_empty() {
            builder = builder.query(&request.query_params);
        }
        if let Some(body) = &request.json_body {
            builder = builder.json(body);
        } else if let Some((content_type, bytes)) = &request.raw_body {
            builder = builder
                .header(CONTENT_TYPE, content_type.clone())
                .body(bytes.clone());
        }
        if let Some(key) = &request.idempotency_key {
            builder = builder.header("Idempotency-Key", key.clone());
        }
        let response = builder.send().await?;
        let status = response.status();
        let body = response.bytes().await?.to_vec();
        Ok(RawResponse {
            operation_id: request.operation.id,
            status,
            body,
        })
    }

    /// Execute and decode a success JSON body into a shared contract type.
    pub async fn execute_decode<T: serde::de::DeserializeOwned>(
        &self,
        request: &RawRequest,
    ) -> Result<T, Error> {
        self.execute(request).await?.decode()
    }

    /// Execute and accept any 2xx status without decoding a body.
    pub async fn execute_empty(&self, request: &RawRequest) -> Result<(), Error> {
        self.execute(request).await?.decode_empty()
    }
}
