use context69_contracts::{
    AuthLoginRequest, CreatePersonalAccessTokenRequest, CreateTextRequest, GroupKind,
    GroupResponse, HealthResponse, HealthStatus, LibraryUploadResponse, ListSourcesResponse,
    MembershipRole, PersonalAccessTokenResponse, PersonalAccessTokenScope, SearchRequest,
    SourceOriginStatusKind, SourceStatus, UpsertLibraryTextRequest, Visibility,
};
use serde_json::{Value, json};
use uuid::Uuid;

#[test]
fn serializes_and_deserializes_core_requests() {
    let request = SearchRequest {
        query: "policy".to_string(),
        limit: 5,
        source_key: Some("gov".to_string()),
        group_key: None,
        project_key: None,
        published_after: None,
        published_before: None,
    };

    let value = serde_json::to_value(&request).expect("serialize request");
    assert_eq!(
        value.get("query"),
        Some(&Value::String("policy".to_string()))
    );

    let login = AuthLoginRequest {
        login_name: "admin".to_string(),
        password: "secret".to_string(),
    };
    let roundtrip: AuthLoginRequest =
        serde_json::from_value(serde_json::to_value(login).expect("serialize login"))
            .expect("deserialize login");
    assert_eq!(roundtrip.login_name, "admin");

    let pat_request = CreatePersonalAccessTokenRequest {
        name: "CI".to_string(),
        scopes: vec![
            PersonalAccessTokenScope::Search,
            PersonalAccessTokenScope::Library,
        ],
        expires_in_days: 30,
    };
    let pat_request_json = serde_json::to_value(&pat_request).expect("serialize pat request");
    assert_eq!(pat_request_json["scopes"], json!(["search", "library"]));

    let text_request = CreateTextRequest {
        folder_id: Some(Uuid::nil()),
        title: "Doc".to_string(),
        content: "Hello".to_string(),
        source_uri: Some("https://example.test/doc".to_string()),
        summary: Some("Summary".to_string()),
    };
    let text_roundtrip: CreateTextRequest =
        serde_json::from_value(serde_json::to_value(text_request).expect("serialize text request"))
            .expect("deserialize text request");
    assert_eq!(text_roundtrip.title, "Doc");

    let upsert_request: UpsertLibraryTextRequest = serde_json::from_value(json!({
        "external_id": "doc-1",
        "title": "Doc",
        "content": "Hello",
        "published_at": "2026-06-10"
    }))
    .expect("deserialize upsert request");
    assert_eq!(upsert_request.metadata_json, json!({}));
    assert_eq!(upsert_request.external_id, "doc-1");
}

#[test]
fn serializes_core_responses() {
    let group = GroupResponse {
        group_id: 1,
        group_key: "team".to_string(),
        parent_group_key: None,
        name: "Team".to_string(),
        visibility: Visibility::Private,
        kind: GroupKind::Shared,
        current_role: Some(MembershipRole::Owner),
        created_at: chrono::DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z")
            .expect("created_at")
            .with_timezone(&chrono::Utc),
        updated_at: chrono::DateTime::parse_from_rfc3339("2026-01-02T00:00:00Z")
            .expect("updated_at")
            .with_timezone(&chrono::Utc),
    };
    let group_json = serde_json::to_value(group).expect("serialize group");
    assert_eq!(
        group_json.get("kind"),
        Some(&Value::String("shared".to_string()))
    );

    let sources = ListSourcesResponse {
        sources: vec![SourceStatus {
            group_key: "team".to_string(),
            project_key: "docs".to_string(),
            visibility: Visibility::Public,
            source_key: "news".to_string(),
            display_name: "News".to_string(),
            description: None,
            example_queries: vec!["latest".to_string()],
            connection: "default".to_string(),
            has_database_url: true,
            origin_status: SourceOriginStatusKind::Connected,
            origin_message: None,
            sync_strategy: "incremental".to_string(),
            connector_type: "postgres_sql".to_string(),
            base_query: "select 1".to_string(),
            batch_size: 100,
            last_cursor_updated_at: None,
            last_cursor_external_id: None,
            last_success_at: None,
        }],
    };
    let sources_json = serde_json::to_value(sources).expect("serialize sources");
    assert_eq!(
        sources_json["sources"][0]["origin_status"],
        json!("connected")
    );

    let health = HealthResponse {
        status: HealthStatus::Degraded,
        indexed_chunks: None,
        db_ok: Some(false),
        qdrant_ok: Some(true),
    };
    let health_json = serde_json::to_value(health).expect("serialize health");
    assert_eq!(health_json["status"], json!("degraded"));

    let library_upload = LibraryUploadResponse {
        files: vec![],
        jobs: vec![],
    };
    let upload_json = serde_json::to_value(library_upload).expect("serialize library upload");
    assert_eq!(upload_json, json!({ "files": [], "jobs": [] }));

    let pat = PersonalAccessTokenResponse {
        token_id: Uuid::nil(),
        name: "CLI".to_string(),
        display_prefix: "ctx_pat_abcd".to_string(),
        scopes: vec![PersonalAccessTokenScope::Search],
        expires_at: chrono::DateTime::parse_from_rfc3339("2026-12-31T00:00:00Z")
            .expect("expires_at")
            .with_timezone(&chrono::Utc),
        last_used_at: None,
        revoked_at: None,
        created_at: chrono::DateTime::parse_from_rfc3339("2026-06-01T00:00:00Z")
            .expect("created_at")
            .with_timezone(&chrono::Utc),
        updated_at: chrono::DateTime::parse_from_rfc3339("2026-06-02T00:00:00Z")
            .expect("updated_at")
            .with_timezone(&chrono::Utc),
    };
    let pat_json = serde_json::to_value(pat).expect("serialize pat");
    assert_eq!(pat_json["display_prefix"], json!("ctx_pat_abcd"));
}
