use axum::http::{Method, StatusCode};
use context69_contracts::{MembershipRole, TaskRef, UpsertMembershipRequest};
use serde_json::json;
use uuid::Uuid;

use super::support::{assert_authorized, client, spawn_empty, spawn_json};

fn group_json() -> serde_json::Value {
    json!({
        "group_id": 1,
        "group_key": "platform",
        "group_path": "ops/platform",
        "parent_group_path": "ops",
        "name": "Platform",
        "visibility": "private",
        "kind": "shared",
        "current_role": "owner",
        "created_at": "2026-07-10T00:00:00Z",
        "updated_at": "2026-07-10T00:00:00Z"
    })
}

fn group_page_json() -> serde_json::Value {
    json!({
        "items": [group_json()],
        "pagination": {"page": 1, "page_size": 50, "total": 1, "total_pages": 1}
    })
}

#[tokio::test]
async fn groups_collection_lists() {
    let (base_url, captured) = spawn_json(StatusCode::OK, &group_page_json()).await;
    let groups = client(&base_url)
        .groups()
        .list()
        .await
        .expect("list groups");
    let request = captured.await.expect("captured request");
    assert_eq!(groups.items[0].group_key, "platform");
    assert_eq!(request.method, Method::GET);
    assert_eq!(request.uri.path(), "/v1/groups");
    assert_authorized(&request);
}

#[tokio::test]
async fn bound_group_encodes_path() {
    let (base_url, captured) = spawn_json(StatusCode::OK, &group_json()).await;
    client(&base_url)
        .group("ops/platform")
        .get()
        .await
        .expect("get group");
    let request = captured.await.expect("captured request");
    assert_eq!(request.method, Method::GET);
    assert_eq!(request.uri.path(), "/v1/groups/by-path/ops%2Fplatform");
}

#[tokio::test]
async fn group_members_upsert_uses_bound_scope() {
    let (base_url, captured) = spawn_empty(StatusCode::NO_CONTENT).await;
    let request_body = UpsertMembershipRequest {
        login_name: "alice".to_string(),
        role: MembershipRole::Maintainer,
    };
    client(&base_url)
        .group("ops/platform")
        .members()
        .upsert(&request_body)
        .await
        .expect("upsert member");
    let request = captured.await.expect("captured request");
    assert_eq!(request.method, Method::POST);
    assert_eq!(
        request.uri.path(),
        "/v1/groups/by-path/ops%2Fplatform/members"
    );
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&request.body).unwrap()["login_name"],
        "alice"
    );
}

#[tokio::test]
async fn group_member_encodes_login_name() {
    let (base_url, captured) = spawn_empty(StatusCode::NO_CONTENT).await;
    client(&base_url)
        .group("ops/platform")
        .member("alice+ops@example.com")
        .delete()
        .await
        .expect("delete member");
    let request = captured.await.expect("captured request");
    assert_eq!(request.method, Method::DELETE);
    assert_eq!(
        request.uri.path(),
        "/v1/groups/by-path/ops%2Fplatform/members/alice%2Bops%40example.com"
    );
}

#[tokio::test]
async fn group_source_folder_chain_syncs() {
    let folder_id = Uuid::new_v4();
    let task = TaskRef {
        task_id: Uuid::new_v4(),
        item_ids: vec![Uuid::new_v4()],
    };
    let (base_url, captured) = spawn_json(StatusCode::ACCEPTED, &task).await;
    let actual = client(&base_url)
        .group("ops/platform")
        .source_folder(folder_id)
        .sync()
        .await
        .expect("sync source folder");
    let request = captured.await.expect("captured request");
    assert_eq!(actual, task);
    assert_eq!(request.method, Method::POST);
    assert_eq!(
        request.uri.path(),
        format!("/v1/groups/by-path/ops%2Fplatform/source-folders/{folder_id}/sync")
    );
}

#[tokio::test]
async fn group_translation_settings_use_bound_scope() {
    let response = json!({
        "enabled": true,
        "default_target_locales": ["zh-CN"],
        "source_locale": "en-US",
        "glossary": [],
        "queued_count": 1,
        "running_count": 0,
        "succeeded_count": 2,
        "failed_count": 0
    });
    let (base_url, captured) = spawn_json(StatusCode::OK, &response).await;
    let settings = client(&base_url)
        .group("ops/platform")
        .translations()
        .settings()
        .await
        .expect("translation settings");
    let request = captured.await.unwrap();
    assert!(settings.enabled);
    assert_eq!(request.method, Method::GET);
    assert_eq!(
        request.uri.path(),
        "/v1/groups/by-path/ops%2Fplatform/translation-settings"
    );
}
