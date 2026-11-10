//! Group-removal safety tests (issue #28 phase P13).
//!
//! Deleting a group cascades to its descendant groups, Git provider
//! connections, and repository sources. Those rows carry the only durable
//! record of remote vector ownership, webhook registrations, and credentials,
//! so removal must refuse while any of them exists instead of silently
//! orphaning remote state. The cases run against the migrated scratch database
//! named by `CONTEXT69_TEST_DATABASE_URL`.

use sqlx::Row;
use uuid::Uuid;

use crate::contracts::sources::{GitConnectionMode, GitIndexProfile, GitProviderKind};
use crate::db::{NewGitProviderConnection, NewGitRepositorySource};
use crate::domain_errors::DomainError;
use crate::services::git_repository::support::Fixture;
use context69_namespace::{MoveGroupInput, NamespaceActor};

struct Owner {
    fixture: Fixture,
    user_id: i64,
    group_path: String,
}

impl Owner {
    async fn setup() -> Self {
        let fixture = Fixture::setup().await;
        let user_id: i64 = sqlx::query(
            "INSERT INTO context69.users (login_name, display_name, password_hash) \
             VALUES ($1, 'Group Delete Test', 'x') RETURNING id",
        )
        .bind(format!("group-delete-{}", Uuid::new_v4()))
        .fetch_one(fixture.db().pool())
        .await
        .expect("seed the deleting user")
        .get("id");
        sqlx::query(
            "INSERT INTO context69.group_memberships (group_id, user_id, role) \
             VALUES ($1, $2, 'owner')",
        )
        .bind(fixture.group_id)
        .bind(user_id)
        .execute(fixture.db().pool())
        .await
        .expect("make the user an owner");
        let seeded_path: String =
            sqlx::query_scalar("SELECT full_path FROM context69.groups WHERE id = $1")
                .bind(fixture.group_id)
                .fetch_one(fixture.db().pool())
                .await
                .expect("read the group path");
        // The shared fixture seeds a slash-prefixed root path, while production
        // create/rebuild use the bare group key for a root. Normalize it so the
        // move path returned by production SQL matches this fixture.
        let group_path = seeded_path.trim_start_matches('/').to_string();
        sqlx::query("UPDATE context69.groups SET full_path = $1 WHERE id = $2")
            .bind(&group_path)
            .bind(fixture.group_id)
            .execute(fixture.db().pool())
            .await
            .expect("normalize the fixture root path");
        Self {
            fixture,
            user_id,
            group_path,
        }
    }

    async fn delete(&self) -> anyhow::Result<()> {
        self.fixture
            .db()
            .delete_group(
                &NamespaceActor {
                    user_id: self.user_id,
                    is_admin: false,
                },
                &self.group_path,
            )
            .await
    }

    async fn cleanup(self) {
        let db = self.fixture.db().clone();
        self.fixture.cleanup().await;
        sqlx::query("DELETE FROM context69.users WHERE id = $1")
            .bind(self.user_id)
            .execute(db.pool())
            .await
            .expect("clean up the deleting user");
    }
}

fn assert_owned_state_conflict(error: anyhow::Error) {
    assert!(
        matches!(
            error.downcast_ref::<DomainError>(),
            Some(DomainError::Conflict(code)) if code == "git_group_owned_state_present"
        ),
        "{error}"
    );
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_group_with_a_git_source_is_not_deleted() {
    let owner = Owner::setup().await;
    assert_owned_state_conflict(
        owner
            .delete()
            .await
            .expect_err("a source's remote ownership must not be dropped by deleting its group"),
    );
    assert!(
        owner
            .fixture
            .db()
            .get_group_by_id(owner.fixture.group_id)
            .await
            .expect("read the group")
            .is_some(),
        "the refused delete leaves the group and its source"
    );

    owner
        .fixture
        .db()
        .delete_git_repository_source(owner.fixture.group_id, owner.fixture.repository_key)
        .await
        .expect("remove the source explicitly");
    owner
        .delete()
        .await
        .expect("a group without Git-owned state can be deleted");
    assert!(
        owner
            .fixture
            .db()
            .get_group_by_id(owner.fixture.group_id)
            .await
            .expect("read the deleted group")
            .is_none(),
        "the explicit cleanup allows the delete"
    );

    owner.cleanup().await;
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_group_with_only_a_provider_connection_is_deleted() {
    let owner = Owner::setup().await;
    owner
        .fixture
        .db()
        .delete_git_repository_source(owner.fixture.group_id, owner.fixture.repository_key)
        .await
        .expect("remove the fixture source so only a connection remains");
    let key = format!("child-{}", Uuid::new_v4());
    let child_id = seed_child_group(&owner, &key).await;
    owner
        .fixture
        .db()
        .upsert_git_provider_connection(
            child_id,
            &NewGitProviderConnection {
                connection_key: "github-main".to_string(),
                provider: GitProviderKind::GitHub,
                mode: GitConnectionMode::Public,
                display_name: "GitHub".to_string(),
                base_url: "https://api.github.com".to_string(),
                credential_secret_key: None,
                webhook_secret_key: None,
            },
        )
        .await
        .expect("seed the child connection");

    owner
        .delete()
        .await
        .expect("a provider connection alone owns no remote points and does not block removal");
    assert!(
        owner
            .fixture
            .db()
            .get_group_by_id(child_id)
            .await
            .expect("read the cascaded child group")
            .is_none(),
        "the parent delete cascades the child group"
    );
    assert!(
        owner
            .fixture
            .db()
            .get_git_provider_connection(child_id, "github-main")
            .await
            .expect("read the cascaded connection")
            .is_none(),
        "the connection cascades with the deleted group"
    );

    owner.cleanup().await;
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_concurrent_descendant_registration_is_fenced_against_the_parent_delete() {
    let owner = Owner::setup().await;
    owner
        .fixture
        .db()
        .delete_git_repository_source(owner.fixture.group_id, owner.fixture.repository_key)
        .await
        .expect("remove the fixture source so only the concurrent registration remains");
    let key = format!("child-{}", Uuid::new_v4());
    let child_id = seed_child_group(&owner, &key).await;

    // An in-flight registration in the descendant holds the shared ancestor
    // lock, exactly as `upsert_git_repository_source` does before its insert.
    let mut in_flight = owner
        .fixture
        .db()
        .pool()
        .begin()
        .await
        .expect("open the in-flight registration transaction");
    sqlx::query_file!(
        "src/sql/db/git_repositories/lock_git_group_source_registration.sql",
        child_id
    )
    .execute(&mut *in_flight)
    .await
    .expect("hold the shared ancestor lock");

    let db = owner.fixture.db().clone();
    let group_path = owner.group_path.clone();
    let user_id = owner.user_id;
    let delete = tokio::spawn(async move {
        db.delete_group(
            &NamespaceActor {
                user_id,
                is_admin: false,
            },
            &group_path,
        )
        .await
    });

    let nonce = Uuid::new_v4();
    let source = owner
        .fixture
        .db()
        .upsert_git_repository_source(
            child_id,
            &NewGitRepositorySource {
                connection_key: None,
                provider: GitProviderKind::GitHub,
                canonical_url: format!("https://github.com/octo/{nonce}"),
                owner: "octo".to_string(),
                name: nonce.to_string(),
                default_branch: "main".to_string(),
                target_ref: "refs/heads/main".to_string(),
                pinned_commit_sha: None,
                index_profile: GitIndexProfile::Hybrid,
                include_patterns: Vec::new(),
                exclude_patterns: Vec::new(),
                interval_seconds: None,
                webhook_enabled: false,
                enabled: true,
                retention_count: 1,
                created_by_user_id: None,
            },
        )
        .await
        .expect("the registration completes while the parent delete waits");
    in_flight
        .commit()
        .await
        .expect("release the in-flight ancestor lock");

    let error = delete
        .await
        .expect("join the delete task")
        .expect_err("the delete must observe the committed descendant source");
    assert_owned_state_conflict(error);
    assert!(
        owner
            .fixture
            .db()
            .get_git_repository_source(child_id, source.repository_key)
            .await
            .expect("read the registered source")
            .is_some(),
        "the fenced registration is not dropped by the cascade"
    );
    assert!(
        owner
            .fixture
            .db()
            .get_group_by_id(owner.fixture.group_id)
            .await
            .expect("read the parent group")
            .is_some(),
        "the refused delete leaves the parent group"
    );

    owner.cleanup().await;
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_descendant_registration_waits_for_an_ancestor_row_lock() {
    let owner = Owner::setup().await;
    owner
        .fixture
        .db()
        .delete_git_repository_source(owner.fixture.group_id, owner.fixture.repository_key)
        .await
        .expect("remove the fixture source");
    let key = format!("child-{}", Uuid::new_v4());
    let child_id = seed_child_group(&owner, &key).await;

    // This is the row lock a group delete takes. It is independent of the
    // advisory ownership protocol, so only the registration's own ancestor row
    // locks can make the registration wait.
    let mut ancestor_row_lock = owner
        .fixture
        .db()
        .pool()
        .begin()
        .await
        .expect("open the ancestor row-lock transaction");
    let blocker: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *ancestor_row_lock)
        .await
        .expect("read the blocking backend id");
    sqlx::query("SELECT id FROM context69.groups WHERE id = $1 FOR UPDATE")
        .bind(owner.fixture.group_id)
        .execute(&mut *ancestor_row_lock)
        .await
        .expect("take the ancestor row lock a delete holds");

    let db = owner.fixture.db().clone();
    let nonce = Uuid::new_v4();
    let registration = tokio::spawn(async move {
        db.upsert_git_repository_source(child_id, &sample_source(nonce))
            .await
    });
    wait_until_blocked_by(owner.fixture.db().pool(), blocker).await;
    ancestor_row_lock
        .commit()
        .await
        .expect("release the ancestor row lock");

    let source = registration
        .await
        .expect("join the registration task")
        .expect("the registration completes once the ancestor row lock is released");
    assert!(
        owner
            .fixture
            .db()
            .get_git_repository_source(child_id, source.repository_key)
            .await
            .expect("read the registered source")
            .is_some(),
        "the registration commits after the ancestor row lock is released"
    );

    owner.cleanup().await;
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_move_into_a_deleting_ancestor_waits_for_ownership() {
    let owner = Owner::setup().await;
    let key = format!("moved-{}", Uuid::new_v4());
    let moved_id = seed_root_group(&owner, &key).await;
    let moved_path = format!("/{key}");

    // An ownership holder for the target ancestor, as a concurrent delete or
    // move would be. The move must not re-parent the subtree until it is
    // released.
    let mut holder = owner
        .fixture
        .db()
        .pool()
        .begin()
        .await
        .expect("open the ownership holder transaction");
    let blocker: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *holder)
        .await
        .expect("read the blocking backend id");
    sqlx::query_file!(
        "src/sql/db/namespaces/lock_group_ownership_for_deletion.sql",
        owner.fixture.group_id
    )
    .execute(&mut *holder)
    .await
    .expect("hold the target ancestor ownership lock");

    let db = owner.fixture.db().clone();
    let target_parent = owner.group_path.clone();
    let user_id = owner.user_id;
    let moved = tokio::spawn(async move {
        db.move_group(
            &NamespaceActor {
                user_id,
                is_admin: false,
            },
            &moved_path,
            &MoveGroupInput {
                target_parent_group_path: Some(target_parent),
            },
        )
        .await
    });
    wait_until_blocked_by(owner.fixture.db().pool(), blocker).await;
    holder
        .commit()
        .await
        .expect("release the target ancestor ownership lock");

    let moved = moved
        .await
        .expect("join the move task")
        .expect("the move completes once ownership is released");
    assert!(
        moved
            .group_path
            .starts_with(&format!("{}/", owner.group_path)),
        "the moved group is now below the target parent"
    );
    assert!(
        owner
            .fixture
            .db()
            .get_group_by_id(moved_id)
            .await
            .expect("read the moved group")
            .is_some(),
        "the moved subtree is never dropped"
    );

    owner.cleanup().await;
}

async fn wait_until_blocked_by(pool: &sqlx::PgPool, blocker: i32) {
    for _ in 0..500 {
        let blocked: bool = sqlx::query_scalar(
            "SELECT EXISTS (\
                 SELECT 1 FROM pg_stat_activity \
                 WHERE pid <> pg_backend_pid() \
                   AND wait_event_type = 'Lock' \
                   AND $1 = ANY(pg_blocking_pids(pid))\
             )",
        )
        .bind(blocker)
        .fetch_one(pool)
        .await
        .expect("inspect blocked backends");
        if blocked {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("the operation never waited for the ownership lock");
}

fn sample_source(nonce: Uuid) -> NewGitRepositorySource {
    NewGitRepositorySource {
        connection_key: None,
        provider: GitProviderKind::GitHub,
        canonical_url: format!("https://github.com/octo/{nonce}"),
        owner: "octo".to_string(),
        name: nonce.to_string(),
        default_branch: "main".to_string(),
        target_ref: "refs/heads/main".to_string(),
        pinned_commit_sha: None,
        index_profile: GitIndexProfile::Hybrid,
        include_patterns: Vec::new(),
        exclude_patterns: Vec::new(),
        interval_seconds: None,
        webhook_enabled: false,
        enabled: true,
        retention_count: 1,
        created_by_user_id: None,
    }
}

async fn seed_root_group(owner: &Owner, key: &str) -> i64 {
    let id: i64 = sqlx::query(
        "INSERT INTO context69.groups \
             (parent_group_id, group_key, name, visibility, kind, full_path) \
         VALUES (NULL, $1, 'Moved', 'private', 'shared', $2) RETURNING id",
    )
    .bind(key)
    .bind(format!("/{key}"))
    .fetch_one(owner.fixture.db().pool())
    .await
    .expect("seed the moved root group")
    .get("id");
    sqlx::query(
        "INSERT INTO context69.group_memberships (group_id, user_id, role) \
         VALUES ($1, $2, 'owner')",
    )
    .bind(id)
    .bind(owner.user_id)
    .execute(owner.fixture.db().pool())
    .await
    .expect("make the user an owner of the moved group");
    id
}

async fn seed_child_group(owner: &Owner, key: &str) -> i64 {
    sqlx::query(
        "INSERT INTO context69.groups \
             (parent_group_id, group_key, name, visibility, kind, full_path) \
         VALUES ($1, $2, 'Child', 'private', 'shared', $3) RETURNING id",
    )
    .bind(owner.fixture.group_id)
    .bind(key)
    .bind(format!("{}/{}", owner.group_path, key))
    .fetch_one(owner.fixture.db().pool())
    .await
    .expect("seed the child group")
    .get("id")
}
