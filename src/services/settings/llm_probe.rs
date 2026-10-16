//! Non-persisting LLM provider connection probe.
//!
//! Tests the exact unsaved draft values a save would store (endpoint, model,
//! protocol, key) with one forced-tool structured call through the shared LLM
//! transport. Nothing is written, no observer fires, and no
//! credential, body, or model output ever reaches a response or a log: every
//! failure is a fixed outcome code.

use std::future::Future;
use std::time::{Duration, Instant};

use crate::contracts::{TestLlmProviderOutcome, TestLlmProviderRequest, TestLlmProviderResponse};
use anyhow::Result;

use super::SettingsService;
use crate::domain_errors::DomainError;
use crate::support::normalize::normalize_optional_string;

/// Whole-probe ceiling, covering secret resolve, connect, and the call.
pub const LLM_PROBE_TIMEOUT: Duration = Duration::from_secs(15);

const PROBE_SYSTEM_PROMPT: &str = "You are a connection probe. Reply with the structured result.";
const PROBE_USER_CONTENT: &str = "probe ping";
const PROBE_MAX_OUTPUT_TOKENS: i32 = 128;

impl SettingsService {
    /// Probe the submitted LLM draft without persisting anything.
    ///
    /// Local shape failures (endpoint URL, protocol, model) are rejected
    /// before any secret is opened or any request is built. A blank key falls
    /// back to the owned stored secret; a missing key is a typed
    /// `configuration_error`, not an outbound call.
    pub async fn test_llm_connection(
        &self,
        request: &TestLlmProviderRequest,
    ) -> Result<TestLlmProviderResponse> {
        let started = Instant::now();
        let validated = validate_probe_request(request)?;
        let outcome = match tokio::time::timeout(
            LLM_PROBE_TIMEOUT,
            self.probe_validated(request, &validated),
        )
        .await
        {
            Err(_elapsed) => TestLlmProviderResponse {
                outcome: TestLlmProviderOutcome::Timeout,
                safe_endpoint: Some(validated.safe_endpoint.clone()),
                llm_api_kind: request.llm_api_kind.clone(),
                http_status: None,
                latency_ms: started.elapsed().as_millis() as u64,
            },
            Ok(response) => {
                let mut response = response?;
                response.latency_ms = started.elapsed().as_millis() as u64;
                response
            }
        };
        Ok(outcome)
    }

    async fn probe_validated(
        &self,
        request: &TestLlmProviderRequest,
        validated: &ValidatedProbe,
    ) -> Result<TestLlmProviderResponse> {
        let api_key = resolve_probe_key(request.api_key.clone(), || {
            self.llm_provider_secrets.resolve()
        })
        .await?;
        let Some(api_key) = api_key else {
            return Ok(TestLlmProviderResponse {
                outcome: TestLlmProviderOutcome::ConfigurationError,
                safe_endpoint: Some(validated.safe_endpoint.clone()),
                llm_api_kind: request.llm_api_kind.clone(),
                http_status: None,
                latency_ms: 0,
            });
        };
        probe_with_key(request, &api_key).await
    }
}

/// Probe with an explicitly supplied key (the stored fallback already
/// resolved, or the submitted override).
///
/// Public so loopback integration tests can drive the real transport without
/// a database: validation, the bounded client, the forced-tool call, and the
/// fixed classification all live here. The service entry above only adds key
/// selection and the whole-call ceiling.
pub async fn probe_with_key(
    request: &TestLlmProviderRequest,
    api_key: &str,
) -> Result<TestLlmProviderResponse> {
    let validated = validate_probe_request(request)?;
    match tokio::time::timeout(LLM_PROBE_TIMEOUT, run_probe(request, &validated, api_key)).await {
        Err(_elapsed) => Ok(TestLlmProviderResponse {
            outcome: TestLlmProviderOutcome::Timeout,
            safe_endpoint: Some(validated.safe_endpoint.clone()),
            llm_api_kind: request.llm_api_kind.clone(),
            http_status: None,
            latency_ms: 0,
        }),
        Ok(response) => response,
    }
}

/// Resolve the probe key through the runtime-used branch: a non-blank
/// submitted key wins without touching the store; otherwise the owned
/// resolver runs exactly once.
///
/// Resolver failures map to a fixed safe error without the raw cause, so no
/// store detail or key material can reach the error chain, logs, or API.
async fn resolve_probe_key<F, Fut>(submitted: Option<String>, resolve: F) -> Result<Option<String>>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<Option<String>>>,
{
    match normalize_optional_string(submitted) {
        Some(supplied) => Ok(Some(supplied)),
        None => Ok(resolve()
            .await
            .map_err(|_| DomainError::internal("stored LLM credential could not be loaded"))?),
    }
}

async fn run_probe(
    request: &TestLlmProviderRequest,
    validated: &ValidatedProbe,
    api_key: &str,
) -> Result<TestLlmProviderResponse> {
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(LLM_PROBE_TIMEOUT)
        .build()
        .map_err(|_| DomainError::internal("the LLM connection test could not start"))?;
    let schema = serde_json::json!({
        "type": "object",
        "properties": {"ok": {"type": "boolean"}},
        "required": ["ok"],
        "additionalProperties": false,
    });
    let call = context69_llm_support::StructuredToolCall {
        endpoint: &validated.base_endpoint,
        api_key,
        model: request.model.trim(),
        api_kind: validated.api_kind,
        tool_name: "submit_result",
        system_prompt: PROBE_SYSTEM_PROMPT,
        user_content: PROBE_USER_CONTENT,
        output_schema: &schema,
        max_output_tokens: PROBE_MAX_OUTPUT_TOKENS,
    };
    match context69_llm_support::structured_tool_call(&client, &call).await {
        Ok(_) => Ok(TestLlmProviderResponse {
            outcome: TestLlmProviderOutcome::Success,
            safe_endpoint: Some(validated.safe_endpoint.clone()),
            llm_api_kind: request.llm_api_kind.clone(),
            http_status: None,
            latency_ms: 0,
        }),
        Err(error) => Ok(classify_probe_error(
            &error,
            &validated.safe_endpoint,
            &request.llm_api_kind,
        )),
    }
}

struct ValidatedProbe {
    base_endpoint: String,
    safe_endpoint: String,
    api_kind: &'static str,
}

fn validate_probe_request(request: &TestLlmProviderRequest) -> Result<ValidatedProbe> {
    let api_kind = match request.llm_api_kind.as_str() {
        "openai_responses" => "openai_responses",
        "openai_chat_completions" => "openai_chat_completions",
        "anthropic_messages" => "anthropic_messages",
        _ => {
            return Err(DomainError::invalid_argument(
                "translation.llm.llm_api_kind must be one of openai_responses, openai_chat_completions, anthropic_messages",
            )
            .into());
        }
    };
    if request.model.trim().is_empty() {
        return Err(
            DomainError::invalid_argument("translation.llm.model must not be empty").into(),
        );
    }
    let suffix = match api_kind {
        "openai_responses" => "/responses",
        "openai_chat_completions" => "/chat/completions",
        _ => "/v1/messages",
    };
    let (base_endpoint, safe_endpoint) = validated_endpoints(&request.endpoint, suffix)?;
    Ok(ValidatedProbe {
        base_endpoint,
        safe_endpoint,
        api_kind,
    })
}

/// Validate the submitted endpoint and rebuild the safe display URL.
///
/// Only `http`/`https` URLs are accepted. Userinfo, query, and fragment are
/// rejected outright rather than stripped: a token in a query string must not
/// travel into the request, and appending an operation to a query string
/// misroutes. Private and loopback hosts stay allowed: this probe is
/// admin-triggered and local gateways must stay probeable. The display URL is
/// rebuilt from the parsed parts plus the operation suffix, never the raw
/// submitted string.
fn validated_endpoints(raw: &str, suffix: &str) -> Result<(String, String)> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(
            DomainError::invalid_argument("translation.llm.endpoint must not be empty").into(),
        );
    }
    let parsed = reqwest::Url::parse(trimmed).map_err(|_| {
        DomainError::invalid_argument("translation.llm.endpoint must be a valid URL")
    })?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(DomainError::invalid_argument(
            "translation.llm.endpoint must use the http or https scheme",
        )
        .into());
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(DomainError::invalid_argument(
            "translation.llm.endpoint must not carry credentials; enter the provider endpoint without a username or password",
        )
        .into());
    }
    if parsed.query().is_some() {
        return Err(DomainError::invalid_argument(
            "translation.llm.endpoint must not carry a query string",
        )
        .into());
    }
    if parsed.fragment().is_some() {
        return Err(DomainError::invalid_argument(
            "translation.llm.endpoint must not carry a fragment",
        )
        .into());
    }
    if let Some((_, after_scheme)) = trimmed.split_once("://") {
        let authority = after_scheme.split('/').next().unwrap_or("");
        if authority.is_empty() {
            return Err(
                DomainError::invalid_argument("translation.llm.endpoint must name a host").into(),
            );
        }
    }
    match parsed.host_str() {
        Some(host) if !host.trim().is_empty() => {}
        _ => {
            return Err(
                DomainError::invalid_argument("translation.llm.endpoint must name a host").into(),
            );
        }
    }
    let mut base = parsed.clone();
    base.set_query(None);
    base.set_fragment(None);
    let base_string = base.to_string();
    let base_trimmed = base_string.trim_end_matches('/');
    let base_endpoint = base_trimmed.to_string();
    let safe_endpoint = if base_trimmed.ends_with(suffix) {
        base_trimmed.to_string()
    } else {
        format!("{base_trimmed}{suffix}")
    };
    Ok((base_endpoint, safe_endpoint))
}

fn classify_probe_error(
    error: &anyhow::Error,
    safe_endpoint: &str,
    llm_api_kind: &str,
) -> TestLlmProviderResponse {
    let base =
        |outcome: TestLlmProviderOutcome, http_status: Option<u16>| TestLlmProviderResponse {
            outcome,
            safe_endpoint: Some(safe_endpoint.to_string()),
            llm_api_kind: llm_api_kind.to_string(),
            http_status,
            latency_ms: 0,
        };
    for cause in error.chain() {
        if let Some(http) = cause.downcast_ref::<context69_llm_support::LlmHttpError>() {
            let outcome = match http.status {
                401 | 403 => TestLlmProviderOutcome::AuthError,
                404 => TestLlmProviderOutcome::NotFound,
                405 => TestLlmProviderOutcome::RouteError,
                429 => TestLlmProviderOutcome::RateLimited,
                400 | 422 => TestLlmProviderOutcome::InvalidPayload,
                300..=399 => TestLlmProviderOutcome::RouteError,
                500..=599 => TestLlmProviderOutcome::UpstreamError,
                _ => TestLlmProviderOutcome::InvalidPayload,
            };
            return base(outcome, Some(http.status));
        }
        if let Some(req) = cause.downcast_ref::<reqwest::Error>() {
            if req.is_timeout() {
                return base(TestLlmProviderOutcome::Timeout, None);
            }
            if req.is_connect() {
                return base(TestLlmProviderOutcome::TransportError, None);
            }
            if let Some(status) = req.status() {
                let code = status.as_u16();
                let outcome = match code {
                    401 | 403 => TestLlmProviderOutcome::AuthError,
                    404 => TestLlmProviderOutcome::NotFound,
                    405 => TestLlmProviderOutcome::RouteError,
                    429 => TestLlmProviderOutcome::RateLimited,
                    400 | 422 => TestLlmProviderOutcome::InvalidPayload,
                    300..=399 => TestLlmProviderOutcome::RouteError,
                    500..=599 => TestLlmProviderOutcome::UpstreamError,
                    _ => TestLlmProviderOutcome::InvalidPayload,
                };
                return base(outcome, Some(code));
            }
            return base(TestLlmProviderOutcome::TransportError, None);
        }
        if cause
            .downcast_ref::<context69_llm_support::LlmPayloadError>()
            .is_some()
            || cause
                .downcast_ref::<context69_llm_support::LlmSchemaError>()
                .is_some()
        {
            return base(TestLlmProviderOutcome::InvalidPayload, None);
        }
    }
    let message = error.to_string().to_lowercase();
    if message.contains("timed out") || message.contains("timeout") || message.contains("deadline")
    {
        return base(TestLlmProviderOutcome::Timeout, None);
    }
    base(TestLlmProviderOutcome::UpstreamError, None)
}

#[cfg(test)]
mod tests {
    use super::validated_endpoints;

    #[test]
    fn endpoint_policy_rejects_userinfo_query_and_fragment() {
        for raw in [
            "https://user:pass@example.com/v1",
            "https://example.com/v1?key=secret",
            "https://example.com/v1#frag",
            "ftp://example.com/v1",
            "https:///v1",
            "   ",
        ] {
            assert!(
                validated_endpoints(raw, "/responses").is_err(),
                "must reject {raw:?}"
            );
        }
    }

    #[test]
    fn endpoint_display_appends_operation_once_and_keeps_v1() {
        let (base, safe) =
            validated_endpoints("https://api.openai.com/v1", "/responses").expect("valid base");
        assert_eq!(base, "https://api.openai.com/v1");
        assert_eq!(safe, "https://api.openai.com/v1/responses");
        let (_, full) = validated_endpoints("https://api.openai.com/v1/responses", "/responses")
            .expect("full endpoint");
        assert_eq!(full, "https://api.openai.com/v1/responses");
        let (_, chat) = validated_endpoints("https://example.com/", "/chat/completions")
            .expect("trailing slash");
        assert_eq!(chat, "https://example.com/chat/completions");
    }

    #[test]
    fn loopback_hosts_stay_probeable() {
        let (_, safe) =
            validated_endpoints("http://127.0.0.1:4010", "/responses").expect("loopback");
        assert!(safe.starts_with("http://127.0.0.1:4010/"));
    }

    #[test]
    fn request_validation_rejects_unknown_protocol_and_blank_model() {
        use crate::contracts::TestLlmProviderRequest;

        use super::validate_probe_request;
        let bad_protocol = TestLlmProviderRequest {
            endpoint: "https://example.com/v1".to_string(),
            model: "gpt-4o-mini".to_string(),
            llm_api_kind: "nope".to_string(),
            api_key: None,
        };
        assert!(validate_probe_request(&bad_protocol).is_err());
        let blank_model = TestLlmProviderRequest {
            endpoint: "https://example.com/v1".to_string(),
            model: "  ".to_string(),
            llm_api_kind: "openai_responses".to_string(),
            api_key: None,
        };
        assert!(validate_probe_request(&blank_model).is_err());
    }

    #[test]
    fn nonblank_override_never_reads_the_store() {
        use std::cell::Cell;

        // A non-blank override (trimmed) never reads the store.
        let reads = Cell::new(0);
        let outcome = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime")
            .block_on(super::resolve_probe_key(
                Some("  override-key  ".to_string()),
                || {
                    reads.set(reads.get() + 1);
                    async { Ok::<_, anyhow::Error>(None) }
                },
            ))
            .expect("override must resolve");
        assert_eq!(outcome, Some("override-key".to_string()));
        assert_eq!(reads.get(), 0);
    }

    #[tokio::test]
    async fn blank_keys_call_the_owned_resolver_exactly_once() {
        use std::cell::Cell;

        use super::resolve_probe_key;
        for submitted in [None, Some(String::new()), Some("   ".to_string())] {
            let reads = Cell::new(0);
            let stored = Some("stored-key".to_string());
            let outcome = resolve_probe_key(submitted, || {
                reads.set(reads.get() + 1);
                let stored = stored.clone();
                async move { Ok::<_, anyhow::Error>(stored) }
            })
            .await
            .expect("resolver success must flow through");
            assert_eq!(outcome, stored);
            assert_eq!(reads.get(), 1, "blank input must read the store once");
        }
        // Stored absence flows through as None (typed ConfigurationError downstream).
        let reads = Cell::new(0);
        let outcome = resolve_probe_key(None, || {
            reads.set(reads.get() + 1);
            async { Ok::<_, anyhow::Error>(None) }
        })
        .await
        .expect("absent stored key must flow through");
        assert_eq!(outcome, None);
        assert_eq!(reads.get(), 1);
    }

    #[tokio::test]
    async fn resolver_failure_is_a_fixed_error_without_token() {
        use std::cell::Cell;

        use super::resolve_probe_key;
        let dummy = "dummy-token-abc-123";
        let reads = Cell::new(0);
        let error = resolve_probe_key(None, || {
            reads.set(reads.get() + 1);
            async move { Err::<_, anyhow::Error>(anyhow::anyhow!("sealed row holds {dummy}")) }
        })
        .await
        .expect_err("a resolver failure must fail before any outbound call");
        let message = format!("{error:#}");
        assert!(
            message.contains("stored LLM credential could not be loaded"),
            "fixed safe message, got: {message}"
        );
        assert!(
            !message.contains(dummy),
            "the injected token must not leak via Debug/Display: {message}"
        );
        assert_eq!(reads.get(), 1);
    }
}
