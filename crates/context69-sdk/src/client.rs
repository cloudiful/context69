mod facade;
mod transport;

use std::{sync::Arc, time::Duration};

use reqwest::{Url, header::USER_AGENT};
use tokio::sync::RwLock;

use crate::Error;

pub use facade::{
    AuthMeResponse, BatchGetDocumentsRequest, BatchGetDocumentsResponse, CompactSearchHit,
    CompactSearchResponse, CreateMetadataIndexRequest, DeleteBatchRequest, DocumentChunkResponse,
    DocumentKey, DocumentResponse, EnsureScopeResponse, FileBatchItem, FileBatchRequest,
    FileMetadata, GroupKind, GroupResponse, HealthResponse, MetadataDataType, MetadataFilter,
    MetadataFilterOperator, MetadataValueKind, RerunTaskResponse, ScopeMetadataIndex, ScopeSpec,
    SearchRequest, TaskItemResponse, TaskItemStatus, TaskItemsResponse, TaskKind, TaskListQuery,
    TaskPageResponse, TaskProgress, TaskRef, TaskResponse, TaskRetryResponse, TaskStatus,
    TextBatchItem, TextBatchRequest, TextContentFormat, TranslationDirective, TranslationStatus,
    UrlBatchItem, UrlBatchRequest, Visibility, WaitOptions,
};

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
                personal_access_token: Some(transport::validate_personal_access_token(
                    token.into(),
                )?),
            })),
        })
    }
}

impl Context69ClientBuilder {
    pub fn base_url(mut self, base_url: &str) -> Result<Self, Error> {
        let mut url =
            Url::parse(base_url).map_err(|_| Error::InvalidBaseUrl(base_url.to_string()))?;
        if !url.path().ends_with('/') {
            url.set_path(&format!("{}/", url.path()));
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
        self.personal_access_token = Some(transport::validate_personal_access_token(token.into())?);
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
        Ok(Context69Client {
            client: builder.build()?,
            base_url,
            session: Arc::new(RwLock::new(SessionState {
                personal_access_token: self.personal_access_token,
            })),
        })
    }
}
