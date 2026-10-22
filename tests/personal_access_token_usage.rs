//! The guarded batch that records personal access token usage (issue #1021, P1).
//!
//! The recorder writes usage as one guarded batch, so the throttle lives in the
//! database statement: a token is written at most once per minute, only while
//! it is active, and only forward in time. These cases run against a real
//! migrated database and prove:
//!
//! 1. the guard writes the tokens whose usage is missing or stale, and leaves
//!    the recent, the future-dated, the revoked, and the expired ones alone;
//! 2. `updated_at` never moves, because usage is telemetry and not a change to
//!    the token's configuration;
//! 3. an empty batch touches nothing, and a second flush inside the window
//!    writes nothing;
//! 4. two concurrent flushes of one stale token write a single row version;
//! 5. the authorization path stays fresh and free while a telemetry write is
//!    blocked: a revoked or expired token is still refused, and recording an
//!    observation never waits for the write.
//!
//! Every row here is synthetic, carries a unique marker, and is deleted again.
//! No token value, hash, or database URL is ever logged.

use std::time::Duration;

use chrono::{DateTime, Duration as ChronoDuration, Utc};
use context69::config::Config;
use context69::db::{Database, NewPersonalAccessToken, PersonalAccessTokenRecord};
use context69::domain_errors::{DomainError, find_domain_error};
use context69::services::auth::AuthService;
use context69::services::personal_access_tokens::{LastUsedRecorder, PersonalAccessTokenService};
use context69::services::token_utils::hash_token;
use sqlx::Row;
use uuid::Uuid;

const DATABASE_NOW: &str = include_str!("personal_access_token_usage/database_now.sql");
const SET_LAST_USED_AT: &str = include_str!("personal_access_token_usage/set_last_used_at.sql");
const LOCK_TOKEN: &str = include_str!("personal_access_token_usage/lock_token.sql");
const DELETE_TOKENS: &str = include_str!("personal_access_token_usage/delete_tokens.sql");
const DELETE_PERSONAL_GROUP: &str =
    include_str!("personal_access_token_usage/delete_personal_group.sql");
const DELETE_USER: &str = include_str!("personal_access_token_usage/delete_user.sql");

/// How far in the past a usage timestamp has to be to count as stale.
const STALE: ChronoDuration = ChronoDuration::seconds(120);

/// How recently a usage timestamp may be and still count as current.
const RECENT: ChronoDuration = ChronoDuration::seconds(5);

/// One synthetic token and the identity the database stores it under.
struct Token {
    id: Uuid,
    value: String,
    hash: String,
}

/// The synthetic rows one case owns.
struct Fixture {
    db: Database,
    user_id: i64,
    login_name: String,
    tokens: Vec<Token>,
}

impl Fixture {
    async fn create(db: &Database) -> Self {
        let marker = Uuid::new_v4().simple().to_string();
        let login_name = format!("pat-usage-{marker}");
        let user = db
            .create_user(&login_name, &login_name, "not-a-password-hash", false)
            .await
            .expect("create the synthetic user");
        Self {
            db: db.clone(),
            user_id: user.id,
            login_name,
            tokens: Vec::new(),
        }
    }

    /// Create one token that expires at the given instant.
    async fn token(&mut self, name: &str, expires_at: DateTime<Utc>) -> Token {
        let value = format!("ctx_pat_{}", Uuid::new_v4().simple());
        let hash = hash_token(&value);
        let id = Uuid::new_v4();
        self.db
            .insert_personal_access_token(&NewPersonalAccessToken {
                id,
                user_id: self.user_id,
                name: name.to_string(),
                token_hash: hash.clone(),
                display_prefix: value.chars().take(18).collect(),
                scopes: vec!["search".to_string()],
                expires_at,
            })
            .await
            .expect("insert the synthetic token");
        let token = Token { id, value, hash };
        self.tokens.push(Token {
            id: token.id,
            value: token.value.clone(),
            hash: token.hash.clone(),
        });
        token
    }

    async fn set_last_used(&self, token_id: Uuid, at: DateTime<Utc>) {
        sqlx::query(SET_LAST_USED_AT)
            .bind(token_id)
            .bind(at)
            .execute(self.db.pool())
            .await
            .expect("place the usage timestamp");
    }

    async fn read(&self, token: &Token) -> PersonalAccessTokenRecord {
        self.db
            .get_personal_access_token_by_hash(&token.hash)
            .await
            .expect("read the token row")
            .expect("the token row exists")
    }

    /// Delete every row this fixture created.
    async fn cleanup(self) {
        let ids: Vec<Uuid> = self.tokens.iter().map(|token| token.id).collect();
        sqlx::query(DELETE_TOKENS)
            .bind(&ids)
            .execute(self.db.pool())
            .await
            .expect("delete the synthetic tokens");
        sqlx::query(DELETE_PERSONAL_GROUP)
            .bind(format!("personal-{}", self.login_name))
            .execute(self.db.pool())
            .await
            .expect("delete the synthetic personal group");
        sqlx::query(DELETE_USER)
            .bind(self.user_id)
            .execute(self.db.pool())
            .await
            .expect("delete the synthetic user");
    }
}

/// The database clock, so every case places its rows on the same timeline as
/// the guard it exercises.
async fn database_now(db: &Database) -> DateTime<Utc> {
    let row = sqlx::query(DATABASE_NOW)
        .fetch_one(db.pool())
        .await
        .expect("read the database clock");
    row.get::<DateTime<Utc>, _>("now")
}

/// The authorization error a refused token produced.
fn unauthorized_message(error: &anyhow::Error) -> String {
    match find_domain_error(error) {
        Some(DomainError::Unauthorized(message)) => message.clone(),
        Some(other) => panic!("the token was refused with another error: {other:?}"),
        None => panic!("the token was refused without a typed error"),
    }
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn the_guarded_batch_updates_only_stale_active_tokens() {
    let url = context69_test_support::accessor::database_url();
    let db = Database::connect(&url)
        .await
        .expect("connect the test database");
    let mut fixture = Fixture::create(&db).await;
    let now = database_now(&db).await;

    let unused = fixture.token("unused", now + ChronoDuration::days(1)).await;
    let stale = fixture.token("stale", now + ChronoDuration::days(1)).await;
    let recent = fixture.token("recent", now + ChronoDuration::days(1)).await;
    let future = fixture.token("future", now + ChronoDuration::days(1)).await;
    let revoked = fixture
        .token("revoked", now + ChronoDuration::days(1))
        .await;
    let expired = fixture
        .token("expired", now - ChronoDuration::days(1))
        .await;

    let stale_at = now - STALE;
    let recent_at = now - RECENT;
    let future_at = now + ChronoDuration::hours(1);
    fixture.set_last_used(stale.id, stale_at).await;
    fixture.set_last_used(recent.id, recent_at).await;
    fixture.set_last_used(future.id, future_at).await;
    db.revoke_personal_access_token(revoked.id, fixture.user_id)
        .await
        .expect("revoke the token")
        .expect("the token exists");

    let before: Vec<PersonalAccessTokenRecord> = vec![
        fixture.read(&unused).await,
        fixture.read(&stale).await,
        fixture.read(&recent).await,
        fixture.read(&future).await,
        fixture.read(&revoked).await,
        fixture.read(&expired).await,
    ];

    let written = db
        .touch_personal_access_tokens_last_used(&[
            unused.id, stale.id, recent.id, future.id, revoked.id, expired.id,
        ])
        .await
        .expect("the guarded batch runs");
    assert_eq!(
        written, 2,
        "only the missing and the stale usage are written"
    );

    let after: Vec<PersonalAccessTokenRecord> = vec![
        fixture.read(&unused).await,
        fixture.read(&stale).await,
        fixture.read(&recent).await,
        fixture.read(&future).await,
        fixture.read(&revoked).await,
        fixture.read(&expired).await,
    ];

    let unused_after = after[0].last_used_at.expect("the missing usage is written");
    assert!(
        unused_after >= now,
        "the written usage is the current database time: {unused_after}"
    );
    let stale_after = after[1].last_used_at.expect("the stale usage is written");
    assert!(
        stale_after > stale_at && stale_after >= now,
        "the stale usage moves forward to the current database time: {stale_after}"
    );
    assert_eq!(
        after[2].last_used_at,
        Some(recent_at),
        "a usage inside the window is left alone"
    );
    assert_eq!(
        after[3].last_used_at,
        Some(future_at),
        "a usage that is ahead of the window never regresses"
    );
    assert_eq!(
        after[4].last_used_at, None,
        "a revoked token records no usage"
    );
    assert_eq!(
        after[5].last_used_at, None,
        "an expired token records no usage"
    );

    for (before_row, after_row) in before.iter().zip(after.iter()) {
        assert_eq!(
            after_row.updated_at, before_row.updated_at,
            "usage telemetry leaves the token's configuration timestamp alone"
        );
    }

    fixture.cleanup().await;
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn an_empty_batch_and_a_second_flush_inside_the_window_write_nothing() {
    let url = context69_test_support::accessor::database_url();
    let db = Database::connect(&url)
        .await
        .expect("connect the test database");
    let mut fixture = Fixture::create(&db).await;
    let now = database_now(&db).await;
    let stale = fixture.token("stale", now + ChronoDuration::days(1)).await;
    fixture.set_last_used(stale.id, now - STALE).await;

    let empty = db
        .touch_personal_access_tokens_last_used(&[])
        .await
        .expect("an empty batch runs");
    assert_eq!(empty, 0, "an empty batch writes nothing");

    let first = db
        .touch_personal_access_tokens_last_used(&[stale.id])
        .await
        .expect("the guarded batch runs");
    assert_eq!(first, 1, "the stale usage is written once");

    let second = db
        .touch_personal_access_tokens_last_used(&[stale.id])
        .await
        .expect("the guarded batch runs again");
    assert_eq!(second, 0, "the window suppresses the second write");

    fixture.cleanup().await;
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn two_concurrent_guarded_flushes_write_one_row_version() {
    let url = context69_test_support::accessor::database_url();
    let db = Database::connect(&url)
        .await
        .expect("connect the test database");
    let mut fixture = Fixture::create(&db).await;
    let now = database_now(&db).await;
    let stale = fixture.token("stale", now + ChronoDuration::days(1)).await;
    fixture.set_last_used(stale.id, now - STALE).await;

    let stale_id = stale.id;
    let ids = [stale_id];
    let (first, second) = tokio::join!(
        db.touch_personal_access_tokens_last_used(&ids),
        db.touch_personal_access_tokens_last_used(&ids),
    );
    let first = first.expect("the first guarded batch runs");
    let second = second.expect("the second guarded batch runs");
    assert_eq!(
        first + second,
        1,
        "one guarded write per window, whichever flush wins the row"
    );

    fixture.cleanup().await;
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_blocked_telemetry_write_leaves_the_authorization_path_fresh_and_free() {
    let url = context69_test_support::accessor::database_url();
    let db = Database::connect(&url)
        .await
        .expect("connect the test database");
    let mut fixture = Fixture::create(&db).await;
    let now = database_now(&db).await;
    let active = fixture.token("active", now + ChronoDuration::days(1)).await;
    let revoked = fixture
        .token("revoked", now + ChronoDuration::days(1))
        .await;
    let expired = fixture
        .token("expired", now - ChronoDuration::days(1))
        .await;
    fixture.set_last_used(active.id, now - STALE).await;
    db.revoke_personal_access_token(revoked.id, fixture.user_id)
        .await
        .expect("revoke the token")
        .expect("the token exists");

    let config = Config::default();
    let auth = AuthService::new(db.clone(), config.auth.clone()).expect("build the auth service");
    let service =
        PersonalAccessTokenService::new(db.clone(), auth, LastUsedRecorder::start(db.clone()));

    // The telemetry write waits for the row, while the authorization read does
    // not. The lock belongs to this test and is always released again.
    let mut lock = db.pool().begin().await.expect("begin the lock transaction");
    sqlx::query(LOCK_TOKEN)
        .bind(active.id)
        .execute(&mut *lock)
        .await
        .expect("lock the token row");

    let writer = {
        let db = db.clone();
        let active_id = active.id;
        tokio::spawn(async move {
            db.touch_personal_access_tokens_last_used(&[active_id])
                .await
        })
    };
    tokio::time::sleep(Duration::from_millis(250)).await;
    assert!(
        !writer.is_finished(),
        "the telemetry write waits for the locked row"
    );

    let verified = service
        .verify(&active.value)
        .await
        .expect("the active token still verifies");
    assert_eq!(verified.token_id, active.id);

    let revoked_error = service
        .verify(&revoked.value)
        .await
        .expect_err("a revoked token stays denied");
    assert!(
        unauthorized_message(&revoked_error).contains("revoked"),
        "the revoked token is refused: {revoked_error}"
    );

    let expired_error = service
        .verify(&expired.value)
        .await
        .expect_err("an expired token stays denied");
    assert!(
        unauthorized_message(&expired_error).contains("expired"),
        "the expired token is refused: {expired_error}"
    );

    // Recording an observation returns without waiting for the write, so the
    // usage timestamp is still the old one at this point.
    service.record_last_used(active.id);
    let during = fixture.read(&active).await;
    assert_eq!(
        during.last_used_at,
        Some(now - STALE),
        "the usage write is deferred, never part of the request"
    );

    lock.rollback().await.expect("release the locked row");
    let written = writer
        .await
        .expect("the telemetry write finished")
        .expect("the guarded batch ran");
    assert_eq!(written, 1, "the deferred write lands once the row is free");

    fixture.cleanup().await;
}
