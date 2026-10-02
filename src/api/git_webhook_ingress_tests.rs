//! Focused tests for the signed GitHub webhook ingress (issue #681 work unit
//! 4B1).
//!
//! The pure cases pin the path and header bounds the unauthenticated surface
//! accepts. The database cases run against a disposable migrated schema named by
//! `CONTEXT69_TEST_DATABASE_URL` and cover the security-relevant behavior: an
//! unknown hook and a bad signature write no delivery, an active registration
//! records `Received`, an inactive one records `Ignored`, a repeated delivery id
//! stays one row, and a sealed row this deployment cannot open fails closed.
//! They report statuses and counts only and never print a signing secret,
//! signature, derived key name, or body.

use super::{WebhookIngressOutcome, parse_provider, process_delivery};
use crate::{
    contracts::sources::{
        GitIndexProfile, GitProviderKind, GitRefreshPolicy, GitWebhookDeliveryStatus,
        GitWebhookOwnership,
    },
    db::{Database, NewGitRepositorySource, NewGitWebhookRegistration},
    services::{
        git_secrets::{GitSecretSlot, GitSecretTarget, GitSecretWriter},
        secret_store::SecretStore,
    },
};
use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;
use sqlx::Row;
use uuid::Uuid;

fn sign(secret: &[u8], body: &[u8]) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret).expect("HMAC accepts any key length");
    mac.update(body);
    format!(
        "sha256={}",
        mac.finalize()
            .into_bytes()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    )
}

#[tokio::test]
async fn the_raw_body_bound_accepts_the_limit_and_rejects_one_more_byte() {
    use crate::services::git_webhook_signature::MAX_GIT_WEBHOOK_BODY_BYTES;
    use axum::body::{Body, to_bytes};

    let at_limit = Body::from(vec![0_u8; MAX_GIT_WEBHOOK_BODY_BYTES]);
    assert!(
        to_bytes(at_limit, MAX_GIT_WEBHOOK_BODY_BYTES).await.is_ok(),
        "a body exactly at the bound is read"
    );
    let over = Body::from(vec![0_u8; MAX_GIT_WEBHOOK_BODY_BYTES + 1]);
    assert!(
        to_bytes(over, MAX_GIT_WEBHOOK_BODY_BYTES).await.is_err(),
        "one byte over the bound is refused before any signature work"
    );
}

#[test]
fn the_provider_path_parses_only_known_kinds() {
    for (path, kind) in [
        ("github", GitProviderKind::GitHub),
        ("forgejo", GitProviderKind::Forgejo),
        ("gitlab", GitProviderKind::GitLab),
        ("generic", GitProviderKind::Generic),
    ] {
        assert_eq!(parse_provider(path), Some(kind), "path {path}");
    }
    for unknown in ["", "GitHub", "github-app", "bitbucket"] {
        assert!(parse_provider(unknown).is_none(), "path {unknown}");
    }
}

/// One seeded group, repository, registration, and keyed store over the scratch
/// pool. Skipped unless `CONTEXT69_TEST_DATABASE_URL` names a migrated disposable
/// database.
struct Seed {
    db: Database,
    store: SecretStore,
    group_id: i64,
    repository_key: Uuid,
    hook_id: String,
    signing_secret: Vec<u8>,
}

async fn fixture(active: bool, seal: bool) -> Option<Seed> {
    let url = std::env::var("CONTEXT69_TEST_DATABASE_URL").ok()?;
    let db = Database::connect(&url)
        .await
        .expect("connect test database");
    let group_key = format!("git-webhook-ingress-{}", Uuid::new_v4());
    let group_id: i64 = sqlx::query(
        "INSERT INTO context69.groups (group_key, name, visibility, kind, full_path) \
         VALUES ($1, 'Git Webhook Ingress Test', 'private', 'shared', $2) RETURNING id",
    )
    .bind(&group_key)
    .bind(format!("/{group_key}"))
    .fetch_one(db.pool())
    .await
    .expect("seed group")
    .get("id");
    let nonce = Uuid::new_v4();
    let repository_key = db
        .upsert_git_repository_source(
            group_id,
            &NewGitRepositorySource {
                connection_key: None,
                provider: GitProviderKind::GitHub,
                canonical_url: format!("https://github.com/octo/{nonce}"),
                owner: "octo".to_string(),
                name: nonce.to_string(),
                default_branch: "main".to_string(),
                target_ref: "refs/heads/main".to_string(),
                target_commit_sha: None,
                index_profile: GitIndexProfile::Lexical,
                refresh_policy: GitRefreshPolicy::Manual,
            },
        )
        .await
        .expect("seed repository source")
        .repository_key;
    let hook_id = nonce.to_string();
    db.upsert_git_webhook_registration(
        group_id,
        &NewGitWebhookRegistration {
            repository_key,
            provider: GitProviderKind::GitHub,
            external_hook_id: hook_id.clone(),
            ownership: GitWebhookOwnership::Integration,
            active,
            signing_secret_key: None,
        },
    )
    .await
    .expect("seed webhook registration");
    // A per-run key, so nothing here is a credential of any deployment.
    let master_key = base64::Engine::encode(
        &base64::engine::general_purpose::STANDARD,
        Uuid::new_v4().into_bytes().repeat(2),
    );
    let store = SecretStore::new(
        context69_secret_store::SecretDatabase::new(db.pool().clone()),
        Some(master_key.as_str()),
        1,
    )
    .expect("a store builds from any master key");
    let signing_secret = format!("hook-secret-{}", Uuid::new_v4()).into_bytes();
    if seal {
        GitSecretWriter::new(db.clone(), store.clone())
            .write(
                &GitSecretTarget::Repository {
                    group_id,
                    repository_key,
                },
                GitSecretSlot::Webhook,
                Some(signing_secret.as_slice()),
            )
            .await
            .expect("seal the signing secret");
    }
    Some(Seed {
        db,
        store,
        group_id,
        repository_key,
        hook_id,
        signing_secret,
    })
}

fn unkeyed_store(db: &Database) -> SecretStore {
    SecretStore::new(
        context69_secret_store::SecretDatabase::new(db.pool().clone()),
        None,
        1,
    )
    .expect("a store with no master key cannot fail to build")
}

async fn delivery_count(db: &Database, delivery_id: &str) -> i64 {
    sqlx::query_scalar::<_, i64>(
        "SELECT count(*) FROM context69.git_webhook_deliveries WHERE delivery_id = $1",
    )
    .bind(delivery_id)
    .fetch_one(db.pool())
    .await
    .expect("count deliveries")
}

async fn delivery_status(db: &Database, delivery_id: &str) -> Option<String> {
    sqlx::query_scalar::<_, String>(
        "SELECT status FROM context69.git_webhook_deliveries WHERE delivery_id = $1",
    )
    .bind(delivery_id)
    .fetch_optional(db.pool())
    .await
    .expect("read delivery status")
}

#[tokio::test]
async fn unknown_hook_and_bad_signature_write_no_delivery() {
    let Some(seed) = fixture(true, true).await else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping the ingress round trip");
        return;
    };
    let body = b"{\"zen\":\"keep it logically awesome\"}";
    let valid = sign(&seed.signing_secret, body);

    for (label, hook, signature, payload) in [
        ("unknown hook", "no-such-hook", valid.clone(), &body[..]),
        (
            "wrong secret",
            seed.hook_id.as_str(),
            sign(b"not-the-signing-secret", body),
            &body[..],
        ),
        (
            "mutated body",
            seed.hook_id.as_str(),
            valid.clone(),
            &b"{\"zen\":\"mutated\"}"[..],
        ),
    ] {
        let delivery_id = format!("{}", Uuid::new_v4());
        let outcome = process_delivery(
            &seed.db,
            &seed.store,
            GitProviderKind::GitHub,
            hook,
            &delivery_id,
            &signature,
            payload,
        )
        .await
        .unwrap_or_else(|error| panic!("{label} must not error: {error}"));
        assert_eq!(outcome, WebhookIngressOutcome::Rejected, "{label}");
        assert_eq!(
            delivery_count(&seed.db, &delivery_id).await,
            0,
            "{label} must write no delivery row"
        );
    }
}

#[tokio::test]
async fn active_registration_records_received() {
    let Some(seed) = fixture(true, true).await else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping the ingress round trip");
        return;
    };
    let body = b"{\"ref\":\"refs/heads/main\"}";
    let delivery_id = format!("{}", Uuid::new_v4());
    let outcome = process_delivery(
        &seed.db,
        &seed.store,
        GitProviderKind::GitHub,
        &seed.hook_id,
        &delivery_id,
        &sign(&seed.signing_secret, body),
        body,
    )
    .await
    .expect("a valid active delivery is processed");

    assert_eq!(
        outcome,
        WebhookIngressOutcome::Recorded {
            status: GitWebhookDeliveryStatus::Received
        }
    );
    assert_eq!(
        delivery_status(&seed.db, &delivery_id).await.as_deref(),
        Some("received")
    );
}

#[tokio::test]
async fn inactive_registration_records_ignored() {
    let Some(seed) = fixture(false, true).await else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping the ingress round trip");
        return;
    };
    let body = b"{}";
    let delivery_id = format!("{}", Uuid::new_v4());
    let outcome = process_delivery(
        &seed.db,
        &seed.store,
        GitProviderKind::GitHub,
        &seed.hook_id,
        &delivery_id,
        &sign(&seed.signing_secret, body),
        body,
    )
    .await
    .expect("a valid inactive delivery is processed");

    assert_eq!(
        outcome,
        WebhookIngressOutcome::Recorded {
            status: GitWebhookDeliveryStatus::Ignored
        }
    );
    assert_eq!(
        delivery_status(&seed.db, &delivery_id).await.as_deref(),
        Some("ignored")
    );
}

#[tokio::test]
async fn a_repeated_delivery_id_stays_one_row() {
    let Some(seed) = fixture(true, true).await else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping the ingress round trip");
        return;
    };
    let body = b"{}";
    let signature = sign(&seed.signing_secret, body);
    let delivery_id = format!("{}", Uuid::new_v4());
    for _ in 0..2 {
        let outcome = process_delivery(
            &seed.db,
            &seed.store,
            GitProviderKind::GitHub,
            &seed.hook_id,
            &delivery_id,
            &signature,
            body,
        )
        .await
        .expect("a redelivery is processed idempotently");
        assert!(matches!(outcome, WebhookIngressOutcome::Recorded { .. }));
    }
    assert_eq!(
        delivery_count(&seed.db, &delivery_id).await,
        1,
        "the provider delivery id is the idempotency key"
    );
}

#[tokio::test]
async fn a_sealed_row_without_a_master_key_fails_closed() {
    let Some(seed) = fixture(true, true).await else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping the ingress round trip");
        return;
    };
    let unkeyed = unkeyed_store(&seed.db);
    // The reader must surface the missing key rather than fall back to plaintext.
    assert!(
        crate::services::git_secrets::GitSecretReader::new(unkeyed.clone())
            .read(
                &GitSecretTarget::Repository {
                    group_id: seed.group_id,
                    repository_key: seed.repository_key,
                },
                GitSecretSlot::Webhook,
            )
            .await
            .is_err(),
        "a sealed row cannot be opened without a master key"
    );

    let body = b"{}";
    let delivery_id = format!("{}", Uuid::new_v4());
    let outcome = process_delivery(
        &seed.db,
        &unkeyed,
        GitProviderKind::GitHub,
        &seed.hook_id,
        &delivery_id,
        &sign(&seed.signing_secret, body),
        body,
    )
    .await
    .expect("an unopenable secret is a bounded rejection, not an error");
    assert_eq!(outcome, WebhookIngressOutcome::Rejected);
    assert_eq!(delivery_count(&seed.db, &delivery_id).await, 0);
}

#[tokio::test]
async fn a_reference_less_registration_is_rejected() {
    let Some(seed) = fixture(true, false).await else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping the ingress round trip");
        return;
    };
    let body = b"{}";
    let delivery_id = format!("{}", Uuid::new_v4());
    let outcome = process_delivery(
        &seed.db,
        &seed.store,
        GitProviderKind::GitHub,
        &seed.hook_id,
        &delivery_id,
        &sign(&seed.signing_secret, body),
        body,
    )
    .await
    .expect("a registration without a signing reference is a bounded rejection");
    assert_eq!(outcome, WebhookIngressOutcome::Rejected);
    assert_eq!(delivery_count(&seed.db, &delivery_id).await, 0);
}

#[tokio::test]
async fn the_signing_secret_is_read_through_the_owning_record_scope() {
    let Some(first) = fixture(true, true).await else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping the ingress round trip");
        return;
    };
    let second = fixture(true, true).await.expect("second seed");
    // Each record is opened through its own group-scoped store; the two stores
    // use independent master keys, so a cross read cannot accidentally decrypt.
    let read = |store: &SecretStore, group_id: i64, repository_key: Uuid| {
        let reader = crate::services::git_secrets::GitSecretReader::new(store.clone());
        async move {
            reader
                .read(
                    &GitSecretTarget::Repository {
                        group_id,
                        repository_key,
                    },
                    GitSecretSlot::Webhook,
                )
                .await
        }
    };
    assert_ne!(
        first.group_id, second.group_id,
        "the two seeds must own distinct groups for this read to mean anything"
    );
    assert_eq!(
        read(&first.store, first.group_id, first.repository_key)
            .await
            .expect("open first")
            .expect("first stored")
            .expose(),
        &first.signing_secret[..]
    );
    assert_eq!(
        read(&second.store, second.group_id, second.repository_key)
            .await
            .expect("open second")
            .expect("second stored")
            .expose(),
        &second.signing_secret[..]
    );
    // The same repository key under a different group derives a different name,
    // so a record-scoped read cannot reach another group's secret.
    assert!(
        read(&first.store, first.group_id, second.repository_key)
            .await
            .expect("open cross")
            .is_none()
    );

    // A signature made with the other record's secret does not verify here.
    let body = b"{}";
    let delivery_id = format!("{}", Uuid::new_v4());
    let outcome = process_delivery(
        &first.db,
        &first.store,
        GitProviderKind::GitHub,
        &first.hook_id,
        &delivery_id,
        &sign(&second.signing_secret, body),
        body,
    )
    .await
    .expect("a foreign signature is a bounded rejection");
    assert_eq!(outcome, WebhookIngressOutcome::Rejected);
    assert_eq!(delivery_count(&first.db, &delivery_id).await, 0);
}
