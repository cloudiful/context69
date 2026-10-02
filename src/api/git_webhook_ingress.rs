//! Signed GitHub webhook ingress (issue #681 work unit 4B1).
//!
//! One unauthenticated provider-facing route receives a delivery, authenticates
//! it against the registration's own signing secret, and records its
//! idempotency state. It is deliberately the smallest slice after the secret
//! store cleanup: no token transport, installation setup, provider network
//! call, event parsing, task enqueue, or schema change is reachable from here.
//!
//! The route sits outside the protected and source-scope routers because a
//! provider has no session; its authentication *is* the HMAC signature over the
//! exact raw body, read before any parsing. The hook id in the path is bounded
//! and is never used as a secret-store key — the key is derived from the record
//! the lookup resolves, so a caller cannot aim the read at another record.
//!
//! Failure is uniform. An unknown hook, an absent or unopenable signing secret,
//! and a bad signature all return the same bounded unauthorized response and
//! create no delivery row, so the route never reveals whether a hook exists.
//! A valid delivery records `Ignored` for an inactive registration and
//! `Received` for an active one through the existing provider-delivery
//! idempotency statement, so a redelivery of one delivery id never adds a row.
//! The signing secret, the signature, the derived key name, and the raw body
//! never appear in an error, a response, or a log line.

use axum::{
    body::to_bytes,
    extract::{Path, Request, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};

use crate::{
    contracts::sources::{GitProviderKind, GitWebhookDeliveryStatus},
    db::{Database, NewGitWebhookDelivery},
    domain_errors::DomainError,
    services::{
        git_secrets::{GitSecretReader, GitSecretSlot, GitSecretTarget},
        git_webhook_signature::{
            MAX_GIT_WEBHOOK_BODY_BYTES, is_valid_delivery_id, is_valid_hook_id,
            verify_github_signature,
        },
        secret_store::SecretStore,
    },
};

use super::{ApiState, errors::internal_error_response, errors::library_management_error_response};

const SIGNATURE_HEADER: &str = "x-hub-signature-256";
const DELIVERY_ID_HEADER: &str = "x-github-delivery";

/// The bounded result of one ingress attempt.
///
/// `Rejected` is one value on purpose: the caller must not be able to tell an
/// unknown hook from a bad signature, so none of those cases gets its own
/// variant or message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WebhookIngressOutcome {
    /// The delivery was recorded idempotently with this status.
    Recorded { status: GitWebhookDeliveryStatus },
    /// Unknown hook, absent or unopenable signing secret, or mismatched
    /// signature. No delivery row was written.
    Rejected,
}

#[utoipa::path(
    post,
    path = "/v1/webhooks/{provider}/{external_hook_id}",
    params(
        ("provider" = String, Path, description = "Git provider kind"),
        ("external_hook_id" = String, Path, description = "Provider-issued webhook hook id")
    ),
    responses(
        (status = 202, description = "Delivery recorded"),
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
    let Some(delivery_id) = header_value(&headers, DELIVERY_ID_HEADER) else {
        return webhook_rejected();
    };
    if !is_valid_delivery_id(&delivery_id) {
        return webhook_rejected();
    }
    let Some(signature) = header_value(&headers, SIGNATURE_HEADER) else {
        return webhook_rejected();
    };
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
        &signature,
        &body,
    )
    .await
    {
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
    signature: &str,
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
    if !verify_github_signature(secret.expose(), body, signature) {
        return Ok(WebhookIngressOutcome::Rejected);
    }
    let status = if registration.active {
        GitWebhookDeliveryStatus::Received
    } else {
        GitWebhookDeliveryStatus::Ignored
    };
    db.record_git_webhook_delivery(&NewGitWebhookDelivery {
        delivery_id: delivery_id.to_string(),
        provider,
        repository_key: Some(registration.repository_key),
        status,
        target_commit_sha: None,
    })
    .await?;
    Ok(WebhookIngressOutcome::Recorded { status })
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
