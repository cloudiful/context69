//! Wire-value to contract-enum decoding for Git persistence.
//!
//! Every decode fails closed on an unknown value: a stored value the code does
//! not recognise is a schema drift, and reporting it beats silently defaulting
//! to a permissive policy.

use anyhow::{Result, anyhow};

use crate::contracts::Visibility;
use crate::contracts::sources::{
    GitConnectionMode, GitGenerationStatus, GitIndexProfile, GitIndexStatus, GitProviderKind,
    GitRefreshPolicy, GitWebhookDeliveryStatus, GitWebhookOwnership,
};

pub(crate) fn provider_kind(value: &str) -> Result<GitProviderKind> {
    match value {
        "github" => Ok(GitProviderKind::GitHub),
        "forgejo" => Ok(GitProviderKind::Forgejo),
        "gitlab" => Ok(GitProviderKind::GitLab),
        "generic" => Ok(GitProviderKind::Generic),
        other => Err(anyhow!("unknown git provider kind: {other}")),
    }
}

pub(crate) fn connection_mode(value: &str) -> Result<GitConnectionMode> {
    match value {
        "public" => Ok(GitConnectionMode::Public),
        "installation" => Ok(GitConnectionMode::Installation),
        "token" => Ok(GitConnectionMode::Token),
        other => Err(anyhow!("unknown git connection mode: {other}")),
    }
}

pub(crate) fn refresh_policy(value: &str) -> Result<GitRefreshPolicy> {
    match value {
        "manual" => Ok(GitRefreshPolicy::Manual),
        "webhook" => Ok(GitRefreshPolicy::Webhook),
        "reconcile" => Ok(GitRefreshPolicy::Reconcile),
        other => Err(anyhow!("unknown git refresh policy: {other}")),
    }
}

pub(crate) fn index_profile(value: &str) -> Result<GitIndexProfile> {
    match value {
        "lexical" => Ok(GitIndexProfile::Lexical),
        "hybrid" => Ok(GitIndexProfile::Hybrid),
        "full_semantic" => Ok(GitIndexProfile::FullSemantic),
        other => Err(anyhow!("unknown git index profile: {other}")),
    }
}

pub(crate) fn index_status(value: &str) -> Result<GitIndexStatus> {
    match value {
        "pending" => Ok(GitIndexStatus::Pending),
        "indexing" => Ok(GitIndexStatus::Indexing),
        "ready" => Ok(GitIndexStatus::Ready),
        "stale" => Ok(GitIndexStatus::Stale),
        "failed" => Ok(GitIndexStatus::Failed),
        "disabled" => Ok(GitIndexStatus::Disabled),
        other => Err(anyhow!("unknown git index status: {other}")),
    }
}

pub(crate) fn generation_status(value: &str) -> Result<GitGenerationStatus> {
    match value {
        "building" => Ok(GitGenerationStatus::Building),
        "ready" => Ok(GitGenerationStatus::Ready),
        "failed" => Ok(GitGenerationStatus::Failed),
        "superseded" => Ok(GitGenerationStatus::Superseded),
        other => Err(anyhow!("unknown git generation status: {other}")),
    }
}

pub(crate) fn webhook_ownership(value: &str) -> Result<GitWebhookOwnership> {
    match value {
        "integration" => Ok(GitWebhookOwnership::Integration),
        "external" => Ok(GitWebhookOwnership::External),
        "unknown" => Ok(GitWebhookOwnership::Unknown),
        other => Err(anyhow!("unknown git webhook ownership: {other}")),
    }
}

pub(crate) fn delivery_status(value: &str) -> Result<GitWebhookDeliveryStatus> {
    match value {
        "received" => Ok(GitWebhookDeliveryStatus::Received),
        "queued" => Ok(GitWebhookDeliveryStatus::Queued),
        "ignored" => Ok(GitWebhookDeliveryStatus::Ignored),
        "failed" => Ok(GitWebhookDeliveryStatus::Failed),
        other => Err(anyhow!("unknown git webhook delivery status: {other}")),
    }
}

/// The group's current visibility, read from `context69.groups` at query time.
pub(crate) fn visibility(value: &str) -> Result<Visibility> {
    value
        .parse()
        .map_err(|_| anyhow!("unknown group visibility: {value}"))
}
