//! Signed provider webhook ingress for Git repository refreshes (issue #28
//! phase P4).
//!
//! One unauthenticated provider-facing route receives a delivery, authenticates
//! it against the registration's own signing secret over the exact raw body,
//! records provider-delivery idempotency, and dispatches an authenticated push
//! into the same single-flight refresh admission manual sync uses.
//!
//! Forgejo 17 sends the raw HMAC-SHA256 hex in `X-Forgejo-Signature` (with
//! `X-Gitea-Signature` and `X-Gogs-Signature` aliases) and the prefixed form in
//! `X-Hub-Signature-256`. Any recognized SHA-256 carrier must verify, empty or
//! prefix-only values are rejected, and SHA-1 is never accepted. The delivery
//! header is an idempotency key only and is never treated as authentication.
//!
//! A validly signed event that is not a push, or a push whose repository/full
//! name or tracked ref does not match the registration, is recorded as
//! `ignored` and still acknowledged. A matching push records the durable
//! refresh demand before admission, so an event that arrives while a task is
//! already running leaves a guaranteed follow-up instead of being dropped.
//!
//! Failure is uniform. An unknown hook, an absent or unopenable signing secret,
//! and a bad signature all return the same bounded unauthorized response and
//! create no delivery row, so the route never reveals whether a hook exists.
//! The signing secret, signature values, derived key name, and raw body never
//! appear in an error, a response, or a log line.

use axum::{
    body::to_bytes,
    extract::{Path, Request, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use serde::Deserialize;

use crate::{
    contracts::sources::{GitProviderKind, GitWebhookDeliveryStatus},
    db::{Database, NewGitWebhookDelivery},
    domain_errors::DomainError,
    services::{
        app::Context69App,
        git_repository::{SafeRef, SafeSha, refresh},
        git_secrets::{GitSecretReader, GitSecretSlot, GitSecretTarget},
        git_webhook_signature::{
            MAX_GIT_WEBHOOK_BODY_BYTES, SignatureCarrier, is_valid_delivery_id, is_valid_hook_id,
            verify_sha256_carriers,
        },
        secret_store::SecretStore,
    },
};

use super::{ApiState, errors::internal_error_response, errors::library_management_error_response};

const RAW_SIGNATURE_HEADERS: [&str; 3] = [
    "x-forgejo-signature",
    "x-gitea-signature",
    "x-gogs-signature",
];
const HUB_SIGNATURE_HEADER: &str = "x-hub-signature-256";
const DELIVERY_ID_HEADERS: [&str; 4] = [
    "x-forgejo-delivery",
    "x-gitea-delivery",
    "x-gogs-delivery",
    "x-github-delivery",
];
const EVENT_HEADERS: [&str; 3] = ["x-forgejo-event", "x-gitea-event", "x-github-event"];

/// The bounded result of one ingress attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum WebhookIngressOutcome {
    /// The delivery was recorded idempotently with this status. `verified` is
    /// present only for a fresh, active delivery whose push may be dispatched.
    Recorded {
        status: GitWebhookDeliveryStatus,
        verified: Option<VerifiedRegistration>,
    },
    /// Unknown hook, absent or unopenable signing secret, or mismatched
    /// signature. No delivery row was written.
    Rejected,
}

/// The registration a verified delivery resolved, before event dispatch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct VerifiedRegistration {
    pub group_id: i64,
    pub repository_key: uuid::Uuid,
    pub provider: GitProviderKind,
    pub external_hook_id: String,
}

#[utoipa::path(
    post,
    path = "/v1/webhooks/{provider}/{external_hook_id}",
    params(
        ("provider" = String, Path, description = "Git provider kind"),
        ("external_hook_id" = String, Path, description = "Provider-issued webhook hook id")
    ),
    responses(
        (status = 202, description = "Delivery accepted"),
        (status = 401, description = "Unknown hook or invalid signature", body = crate::contracts::ApiErrorResponse),
        (status = 413, description = "Webhook body exceeds the accepted bound")
    )
)]
pub(crate) async fn receive_git_webhook(
    State(state): State<ApiState>,
    Path((provider, external_hook_id)): Path<(String, String)>,
    headers: HeaderMap,
    request: Request,
) -> Response {
    let Some(provider) = parse_provider(&provider) else {
        return webhook_rejected();
    };
    if !is_valid_hook_id(&external_hook_id) {
        return webhook_rejected();
    }
    let Some(delivery_id) = single_delivery_id(&headers) else {
        return webhook_rejected();
    };
    let carriers = signature_carriers(&headers);
    if carriers.is_empty() {
        return webhook_rejected();
    }
    // Read the exact bytes before any parsing, under an explicit bound; the
    // signature is computed over these bytes and nothing else.
    let body = match to_bytes(request.into_body(), MAX_GIT_WEBHOOK_BODY_BYTES).await {
        Ok(body) => body,
        Err(_) => return webhook_payload_rejected(),
    };
    match process_delivery(
        &state.app.db,
        &state.app.secrets,
        provider,
        &external_hook_id,
        &delivery_id,
        &carriers,
        &body,
    )
    .await
    {
        Ok(WebhookIngressOutcome::Recorded {
            status: GitWebhookDeliveryStatus::Received,
            verified: Some(registration),
        }) => dispatch_verified_push(&state.app, registration, &delivery_id, &headers, &body).await,
        Ok(WebhookIngressOutcome::Recorded { .. }) => StatusCode::ACCEPTED.into_response(),
        Ok(WebhookIngressOutcome::Rejected) => webhook_rejected(),
        Err(error) => {
            // A lookup or record failure is this deployment's problem, not the
            // caller's; the generic response keeps database detail off the wire.
            tracing::error!(error = %error, "git webhook delivery could not be processed");
            internal_error_response(anyhow::anyhow!(
                "git webhook delivery could not be processed"
            ))
        }
    }
}

/// Resolves the hook, opens its signing secret, verifies the signature, and
/// records the delivery. Split from the handler so the security-relevant
/// behavior is testable against a disposable database without an HTTP surface.
///
/// # Errors
///
/// A registration lookup or delivery write failure. A missing registration, a
/// reference-less registration, an unopenable sealed secret, and a mismatched
/// signature are all `Ok(Rejected)`, never an error, so they cannot be turned
/// into a distinguishing response.
pub(crate) async fn process_delivery(
    db: &Database,
    secrets: &SecretStore,
    provider: GitProviderKind,
    external_hook_id: &str,
    delivery_id: &str,
    carriers: &[SignatureCarrier<'_>],
    body: &[u8],
) -> anyhow::Result<WebhookIngressOutcome> {
    let Some((group_id, registration)) = db
        .get_git_webhook_registration_by_hook(provider, external_hook_id)
        .await?
    else {
        return Ok(WebhookIngressOutcome::Rejected);
    };
    // No configured reference means there is nothing to authenticate with.
    if registration.signing_secret_key.is_none() {
        return Ok(WebhookIngressOutcome::Rejected);
    }
    let target = GitSecretTarget::Repository {
        group_id,
        repository_key: registration.repository_key,
    };
    let secret = match GitSecretReader::new(secrets.clone())
        .read(&target, GitSecretSlot::Webhook)
        .await
    {
        Ok(Some(secret)) => secret,
        // Fail closed: an absent row and a sealed row this deployment cannot
        // open are the same rejection, never a fallback.
        Ok(None) => return Ok(WebhookIngressOutcome::Rejected),
        Err(error) => {
            tracing::warn!(error = %error, "git webhook signing secret could not be opened");
            return Ok(WebhookIngressOutcome::Rejected);
        }
    };
    if !verify_sha256_carriers(secret.expose(), body, carriers) {
        return Ok(WebhookIngressOutcome::Rejected);
    }
    let status = if registration.active {
        GitWebhookDeliveryStatus::Received
    } else {
        GitWebhookDeliveryStatus::Ignored
    };
    let stored = db
        .record_git_webhook_delivery(&NewGitWebhookDelivery {
            delivery_id: delivery_id.to_string(),
            external_hook_id: external_hook_id.to_string(),
            provider,
            repository_key: Some(registration.repository_key),
            status,
            target_commit_sha: None,
        })
        .await?;
    // A redelivery of an already processed id is acknowledged without touching
    // the repository again. A `received` row that never processed (a crash
    // after insert) is retried, because the admission path is idempotent.
    if stored.processed_at.is_some() {
        return Ok(WebhookIngressOutcome::Recorded {
            status: stored.status,
            verified: None,
        });
    }
    if status == GitWebhookDeliveryStatus::Ignored {
        db.mark_git_webhook_delivery_processed(
            provider,
            external_hook_id,
            registration.repository_key,
            delivery_id,
            GitWebhookDeliveryStatus::Ignored,
            None,
        )
        .await?;
        return Ok(WebhookIngressOutcome::Recorded {
            status,
            verified: None,
        });
    }
    Ok(WebhookIngressOutcome::Recorded {
        status,
        verified: Some(VerifiedRegistration {
            group_id,
            repository_key: registration.repository_key,
            provider,
            external_hook_id: external_hook_id.to_string(),
        }),
    })
}

/// Dispatches one verified active delivery after event and repository matching.
async fn dispatch_verified_push(
    app: &Context69App,
    registration: VerifiedRegistration,
    delivery_id: &str,
    headers: &HeaderMap,
    body: &[u8],
) -> Response {
    let Some(event) = event_name(headers) else {
        return ignore_delivery(app, &registration, delivery_id).await;
    };
    if event != "push" {
        return ignore_delivery(app, &registration, delivery_id).await;
    }
    let Some(push) = parse_push_payload(body) else {
        let _ = app
            .db
            .mark_git_webhook_delivery_processed(
                registration.provider,
                &registration.external_hook_id,
                registration.repository_key,
                delivery_id,
                GitWebhookDeliveryStatus::Failed,
                None,
            )
            .await;
        return StatusCode::BAD_REQUEST.into_response();
    };
    let source = match app
        .db
        .get_git_repository_source(registration.group_id, registration.repository_key)
        .await
    {
        Ok(Some(source)) => source,
        Ok(None) => return ignore_delivery(app, &registration, delivery_id).await,
        Err(error) => return webhook_dispatch_failure(error),
    };
    if !push.matches(
        &source.owner,
        &source.name,
        &source.version.ref_name,
        &source.default_branch,
    ) || !source.webhook_enabled
        || !source.enabled
    {
        return ignore_delivery(app, &registration, delivery_id).await;
    }
    let after = push.after.clone();
    let target_sha = after
        .filter(|sha| SafeSha::parse(sha).is_ok())
        .filter(|sha| !sha.bytes().all(|byte| byte == b'0'));
    if target_sha.is_none() && push.after.is_some() {
        return ignore_delivery(app, &registration, delivery_id).await;
    }
    match refresh::request_and_enqueue_refresh(
        app,
        registration.group_id,
        registration.repository_key,
    )
    .await
    {
        Ok(_) => {
            if let Err(error) = app
                .db
                .mark_git_webhook_delivery_processed(
                    registration.provider,
                    &registration.external_hook_id,
                    registration.repository_key,
                    delivery_id,
                    GitWebhookDeliveryStatus::Queued,
                    target_sha.as_deref(),
                )
                .await
            {
                return webhook_dispatch_failure(error);
            }
            StatusCode::ACCEPTED.into_response()
        }
        Err(error) => {
            let _ = app
                .db
                .mark_git_webhook_delivery_processed(
                    registration.provider,
                    &registration.external_hook_id,
                    registration.repository_key,
                    delivery_id,
                    GitWebhookDeliveryStatus::Failed,
                    None,
                )
                .await;
            webhook_dispatch_failure(error)
        }
    }
}

async fn ignore_delivery(
    app: &Context69App,
    registration: &VerifiedRegistration,
    delivery_id: &str,
) -> Response {
    match app
        .db
        .mark_git_webhook_delivery_processed(
            registration.provider,
            &registration.external_hook_id,
            registration.repository_key,
            delivery_id,
            GitWebhookDeliveryStatus::Ignored,
            None,
        )
        .await
    {
        Ok(_) => StatusCode::ACCEPTED.into_response(),
        Err(error) => webhook_dispatch_failure(error),
    }
}

fn webhook_dispatch_failure(error: anyhow::Error) -> Response {
    tracing::error!(error = %error, "git webhook delivery could not be dispatched");
    internal_error_response(anyhow::anyhow!(
        "git webhook delivery could not be dispatched"
    ))
}

/// The bounded push payload fields the ingress matches.
#[derive(Debug, Deserialize)]
struct PushPayload {
    #[serde(rename = "ref")]
    ref_name: Option<String>,
    after: Option<String>,
    repository: Option<PushRepository>,
}

#[derive(Debug, Deserialize)]
struct PushRepository {
    full_name: Option<String>,
    default_branch: Option<String>,
}

impl PushPayload {
    /// A push belongs to a source only when its stable repository identity and
    /// tracked ref match. Gitea's create/delete form may omit `refs/heads/`, so
    /// only that documented short branch form is accepted. A `HEAD`-tracked
    /// source follows the pushed repository's declared default branch, falling
    /// back to the stored default branch when the payload omits it.
    fn matches(
        &self,
        owner: &str,
        name: &str,
        target_ref: &str,
        stored_default_branch: &str,
    ) -> bool {
        let expected_full_name = format!("{owner}/{name}");
        let full_name_matches = self
            .repository
            .as_ref()
            .and_then(|repository| repository.full_name.as_deref())
            .is_some_and(|full_name| full_name.eq_ignore_ascii_case(&expected_full_name));
        if !full_name_matches {
            return false;
        }
        let Some(payload_ref) = self.ref_name.as_deref() else {
            return false;
        };
        if target_ref == "HEAD" {
            let declared = self
                .repository
                .as_ref()
                .and_then(|repository| repository.default_branch.as_deref());
            let branch = match declared {
                Some(branch) => branch,
                None => stored_default_branch,
            };
            let Ok(expected) = SafeRef::parse(&format!("refs/heads/{branch}")) else {
                return false;
            };
            return branch_ref_matches(payload_ref, expected.as_str());
        }
        if payload_ref == target_ref {
            return true;
        }
        match target_ref.strip_prefix("refs/heads/") {
            Some(branch) => payload_ref == branch,
            None => false,
        }
    }
}

fn branch_ref_matches(payload_ref: &str, expected: &str) -> bool {
    if payload_ref == expected {
        return true;
    }
    match expected.strip_prefix("refs/heads/") {
        Some(branch) => payload_ref == branch,
        None => false,
    }
}

fn parse_push_payload(body: &[u8]) -> Option<PushPayload> {
    serde_json::from_slice(body).ok()
}

/// The first recognized event header, preferring the Forgejo family.
fn event_name(headers: &HeaderMap) -> Option<String> {
    EVENT_HEADERS
        .iter()
        .find_map(|name| header_value(headers, name))
}

/// One delivery id, with conflicting duplicates rejected.
fn single_delivery_id(headers: &HeaderMap) -> Option<String> {
    let mut values = DELIVERY_ID_HEADERS
        .iter()
        .flat_map(|name| header_values(headers, name))
        .filter(|value| is_valid_delivery_id(value));
    let first = values.next()?;
    if values.any(|value| value != first) {
        return None;
    }
    Some(first.to_string())
}

/// Every recognized SHA-256 carrier value, including repeated headers.
fn signature_carriers(headers: &HeaderMap) -> Vec<SignatureCarrier<'_>> {
    let mut carriers = Vec::new();
    for name in RAW_SIGNATURE_HEADERS {
        for value in header_values(headers, name) {
            carriers.push(SignatureCarrier::RawHex(value));
        }
    }
    for value in header_values(headers, HUB_SIGNATURE_HEADER) {
        carriers.push(SignatureCarrier::HubSha256(value));
    }
    carriers
}

/// Parses the provider path segment into the existing provider enum.
fn parse_provider(value: &str) -> Option<GitProviderKind> {
    Some(match value {
        "github" => GitProviderKind::GitHub,
        "forgejo" => GitProviderKind::Forgejo,
        "gitlab" => GitProviderKind::GitLab,
        "generic" => GitProviderKind::Generic,
        _ => return None,
    })
}

/// Reads one header as an owned string, or `None` when it is absent or not
/// valid ASCII. The caller applies the value's own bound.
fn header_value(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string)
}

/// Reads every value of one header name, preserving repeats.
fn header_values<'a>(headers: &'a HeaderMap, name: &str) -> impl Iterator<Item = &'a str> {
    headers
        .get_all(name)
        .iter()
        .filter_map(|value| value.to_str().ok())
}

/// The one bounded rejection every unauthenticated failure maps to.
fn webhook_rejected() -> Response {
    library_management_error_response(
        DomainError::unauthorized("webhook delivery could not be verified").into(),
    )
}

/// A body over the accepted bound is refused before any signature work. The
/// response depends only on the request size, so it distinguishes no hook.
fn webhook_payload_rejected() -> Response {
    library_management_error_response(
        DomainError::payload_too_large("git_webhook_body_too_large").into(),
    )
}

#[cfg(test)]
#[path = "git_webhook_ingress_tests.rs"]
mod tests;
