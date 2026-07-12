use axum::http::{Method, StatusCode, header};
use context69_contracts::{
    ImportLibraryFileFromUrlRequest, LibraryFileUploadMetadata, LibraryTextContentFormat,
    MoveFolderRequest, UpsertLibraryTextRequest,
};
use reqwest::multipart::Part;
use serde_json::json;
use uuid::Uuid;

use super::support::{client, spawn_empty, spawn_json};

fn tree_json() -> serde_json::Value {
    json!({
        "root": {
            "group_key": "", "group_path": "", "visibility": "private",
            "folder_id": null, "parent_folder_id": null, "name": "Library",
            "path": "/", "processing_count": 0, "children": [], "files": []
        }
    })
}

fn file_json(file_id: Uuid) -> serde_json::Value {
    json!({
        "file_id": file_id, "group_key": "ops", "group_path": "ops/platform",
        "visibility": "private", "folder_id": null, "folder_path": "/",
        "filename": "runbook.md", "media_type": "text/markdown", "size_bytes": 8,
        "sha256": "abc", "ingest_status": "succeeded", "error_message": null,
        "created_at": "2026-07-10T00:00:00Z", "updated_at": "2026-07-10T00:00:00Z",
        "ingested_at": "2026-07-10T00:00:00Z", "sections": [], "jobs": []
    })
}

fn url_import_job_json(job_id: Uuid) -> serde_json::Value {
    json!({
        "import_job_id": job_id, "group_path": "ops/platform",
        "source_url": "https://files.example.test/report.pdf", "status": "queued",
        "attempt_count": 0, "created_at": "2026-07-12T00:00:00Z",
        "started_at": null, "finished_at": null, "updated_at": "2026-07-12T00:00:00Z"
    })
}

#[tokio::test]
async fn group_library_imports_public_url() {
    let job_id = Uuid::new_v4();
    let (base_url, captured) = spawn_json(StatusCode::ACCEPTED, &url_import_job_json(job_id)).await;
    let request = ImportLibraryFileFromUrlRequest {
        url: "https://files.example.test/report.pdf".into(),
        folder_id: None,
        filename: None,
        media_type: None,
        metadata: Some(LibraryFileUploadMetadata {
            external_id: Some("report-42".into()),
            ..Default::default()
        }),
        translation: None,
    };
    let job = client(&base_url)
        .group("ops/platform")
        .library()
        .files()
        .import_url(&request)
        .await
        .expect("import URL");
    let captured = captured.await.expect("captured request");
    assert_eq!(job.import_job_id, job_id);
    assert_eq!(captured.method, Method::POST);
    assert_eq!(
        captured.uri.path(),
        "/v1/groups/by-path/ops%2Fplatform/library/files/import-url"
    );
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&captured.body).unwrap()["metadata"]["external_id"],
        "report-42"
    );
}

#[tokio::test]
async fn group_library_url_import_job_get_uses_group_scope() {
    let job_id = Uuid::new_v4();
    let (base_url, captured) = spawn_json(StatusCode::OK, &url_import_job_json(job_id)).await;
    client(&base_url)
        .group("ops/platform")
        .library()
        .url_import_job(job_id)
        .get()
        .await
        .expect("get URL import job");
    let captured = captured.await.expect("captured request");
    assert_eq!(
        captured.uri.path(),
        format!("/v1/groups/by-path/ops%2Fplatform/library/url-import-jobs/{job_id}")
    );
}

#[tokio::test]
async fn global_library_tree_uses_global_scope() {
    let (base_url, captured) = spawn_json(StatusCode::OK, &tree_json()).await;
    let tree = client(&base_url).library().tree().await.expect("tree");
    let request = captured.await.expect("captured request");
    assert_eq!(tree.root.path, "/");
    assert_eq!(request.uri.path(), "/v1/library/tree");
}

#[tokio::test]
async fn group_library_text_upsert_uses_group_scope() {
    let response = json!({"files": [], "jobs": []});
    let (base_url, captured) = spawn_json(StatusCode::OK, &response).await;
    let input = UpsertLibraryTextRequest {
        external_id: "incident-42".to_string(),
        folder_id: None,
        title: "Incident".to_string(),
        content: "body".to_string(),
        content_format: LibraryTextContentFormat::Markdown,
        source_uri: None,
        summary: None,
        published_at: None,
        metadata_json: json!({}),
        translation: None,
    };
    client(&base_url)
        .group("ops/platform")
        .library()
        .texts()
        .upsert(&input)
        .await
        .expect("upsert text");
    let request = captured.await.expect("captured request");
    assert_eq!(request.method, Method::PUT);
    assert_eq!(
        request.uri.path(),
        "/v1/groups/by-path/ops%2Fplatform/library/texts"
    );
}

#[tokio::test]
async fn temporary_group_library_file_chain_gets_file() {
    let file_id = Uuid::new_v4();
    let (base_url, captured) = spawn_json(StatusCode::OK, &file_json(file_id)).await;
    let file = client(&base_url)
        .group("ops/platform")
        .library()
        .file(file_id)
        .get()
        .await
        .expect("get file");
    let request = captured.await.expect("captured request");
    assert_eq!(file.file_id, file_id);
    assert_eq!(
        request.uri.path(),
        format!("/v1/groups/by-path/ops%2Fplatform/library/files/{file_id}")
    );
}

#[tokio::test]
async fn library_folder_resource_moves() {
    let folder_id = Uuid::new_v4();
    let response = json!({
        "folder_id": folder_id, "group_key": "", "group_path": "",
        "visibility": "private", "parent_folder_id": null, "name": "Docs", "path": "/Docs",
        "created_at": "2026-07-10T00:00:00Z", "updated_at": "2026-07-10T00:00:00Z"
    });
    let (base_url, captured) = spawn_json(StatusCode::OK, &response).await;
    client(&base_url)
        .library()
        .folder(folder_id)
        .move_to(&MoveFolderRequest {
            target_folder_id: None,
        })
        .await
        .expect("move folder");
    let request = captured.await.expect("captured request");
    assert_eq!(request.method, Method::POST);
    assert_eq!(
        request.uri.path(),
        format!("/v1/library/folders/{folder_id}/move")
    );
}

#[tokio::test]
async fn library_files_upload_builds_multipart_form() {
    let folder_id = Uuid::new_v4();
    let response = json!({"files": [], "jobs": []});
    let (base_url, captured) = spawn_json(StatusCode::OK, &response).await;
    client(&base_url)
        .library()
        .files()
        .upload(
            Some(folder_id),
            vec![Part::text("hello").file_name("note.md")],
        )
        .await
        .expect("upload files");
    let request = captured.await.expect("captured request");
    assert_eq!(request.uri.path(), "/v1/library/files/upload");
    assert!(
        request.headers[header::CONTENT_TYPE]
            .to_str()
            .unwrap()
            .starts_with("multipart/form-data; boundary=")
    );
    let body = String::from_utf8_lossy(&request.body);
    assert!(body.contains(&folder_id.to_string()));
    assert!(body.contains("filename=\"note.md\""));
}

#[tokio::test]
async fn library_file_delete_uses_item_endpoint() {
    let file_id = Uuid::new_v4();
    let (base_url, captured) = spawn_empty(StatusCode::NO_CONTENT).await;
    client(&base_url)
        .library()
        .file(file_id)
        .delete()
        .await
        .expect("delete file");
    assert_eq!(captured.await.unwrap().method, Method::DELETE);
}
