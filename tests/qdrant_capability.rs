//! Opt-in capability test for the Qdrant conditional-write and payload-filter
//! primitives that a future staged re-ingest design would rely on.
//!
//! The test only ever talks to the single authorized test endpoint
//! (`http://unraid.cloud1ful.com:6335`, the DBX connection named
//! `context69 qdrant test`). It creates exactly one uniquely named
//! `context69_ci_<uuid>` collection, writes only synthetic test points into
//! it, and deletes that exact collection again — on success and on failure.
//! It never lists collections, never deletes anything it did not create, and
//! never runs a sweep over leftover names.
//!
//! The test is ignored by default. Run it explicitly, single-threaded, with
//! both endpoint variables set:
//!
//! ```text
//! CONTEXT69_TEST_QDRANT_URL=http://unraid.cloud1ful.com:6335 \
//! CONTEXT69_TEST_QDRANT_EXPECT=unraid.cloud1ful.com:6335 \
//! cargo test --test qdrant_capability --offline -- --ignored --test-threads=1
//! ```
//!
//! `CONTEXT69_TEST_QDRANT_URL` supplies the target and
//! `CONTEXT69_TEST_QDRANT_EXPECT` states the expected authority
//! independently, so a stale or inherited variable cannot silently redirect
//! the run. Both are mandatory and both are checked against the authorized
//! authority before any write is issued; a missing, mismatched, redirected,
//! credential-carrying or production endpoint aborts the test before it
//! touches the server.

use std::env;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, ensure};
use reqwest::{Client, Method, StatusCode, Url};
use serde_json::{Value, json};
use uuid::Uuid;

const EXPECTED_HOST: &str = "unraid.cloud1ful.com";
const EXPECTED_PORT: u16 = 6335;
const EXPECTED_AUTHORITY: &str = "unraid.cloud1ful.com:6335";
const AUTHORIZED_TEST_URL: &str = "http://unraid.cloud1ful.com:6335";
const PRODUCTION_PORT: u16 = 6333;
const URL_ENV: &str = "CONTEXT69_TEST_QDRANT_URL";
const EXPECT_ENV: &str = "CONTEXT69_TEST_QDRANT_EXPECT";
const COLLECTION_PREFIX: &str = "context69_ci_";
const PROTECTED_COLLECTION: &str = "context69_chunks";
const MINIMUM_SERVER_VERSION: (u64, u64, u64) = (1, 17, 0);
const VECTOR_SIZE: usize = 4;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

#[tokio::test]
#[ignore = "requires CONTEXT69_TEST_QDRANT_URL and CONTEXT69_TEST_QDRANT_EXPECT for the authorized test endpoint"]
async fn qdrant_conditional_write_capabilities() -> Result<()> {
    let target = resolve_target()?;
    let client = build_client()?;
    let collection = format!("{COLLECTION_PREFIX}{}", Uuid::new_v4());
    ensure!(
        collection != PROTECTED_COLLECTION,
        "generated collection name must never be the protected collection"
    );

    let scratch = Scratch {
        client: &client,
        base_url: target.clone(),
        collection: collection.clone(),
    };

    let server_version = read_server_version(&scratch).await?;
    println!("authorized test endpoint {EXPECTED_AUTHORITY} reports version {server_version}");

    let mut create_attempted = false;
    let outcome = async {
        ensure_collection_absent(&scratch).await?;
        create_attempted = true;
        create_collection(&scratch).await?;
        ensure_entity_field_indexes(&scratch).await?;
        run_capability_cases(&scratch).await
    }
    .await;

    if create_attempted {
        let cleanup = delete_collection(&scratch).await;
        return match (outcome, cleanup) {
            (Ok(()), Ok(())) => Ok(()),
            (Err(cases), Ok(())) => Err(cases),
            (Ok(()), Err(cleanup_error)) => Err(cleanup_error),
            (Err(cases), Err(cleanup_error)) => Err(anyhow!(
                "capability cases failed: {cases:#}; cleanup of {collection} also failed: {cleanup_error:#}"
            )),
        };
    }

    outcome
}

struct Scratch<'a> {
    client: &'a Client,
    base_url: String,
    collection: String,
}

impl Scratch<'_> {
    fn collection_base(&self) -> String {
        format!("{}/collections/{}", self.base_url, self.collection)
    }

    fn upsert_url(&self) -> String {
        format!("{}/points?wait=true", self.collection_base())
    }

    fn set_payload_url(&self) -> String {
        format!("{}/points/payload?wait=true", self.collection_base())
    }

    fn delete_points_url(&self) -> String {
        format!("{}/points/delete?wait=true", self.collection_base())
    }

    fn retrieve_url(&self) -> String {
        format!("{}/points", self.collection_base())
    }

    fn scroll_url(&self) -> String {
        format!("{}/points/scroll", self.collection_base())
    }

    fn search_url(&self) -> String {
        format!("{}/points/search", self.collection_base())
    }

    fn index_url(&self) -> String {
        format!("{}/index?wait=true", self.collection_base())
    }
}

fn resolve_target() -> Result<String> {
    let raw_url = env::var(URL_ENV).map_err(|_| {
        anyhow!("{URL_ENV} is not set; the capability test requires the authorized test endpoint")
    })?;
    let raw_url = raw_url.trim();
    ensure!(!raw_url.is_empty(), "{URL_ENV} is empty");

    let raw_expect = env::var(EXPECT_ENV).map_err(|_| {
        anyhow!("{EXPECT_ENV} is not set; state the expected authority independently")
    })?;
    let raw_expect = raw_expect.trim();
    ensure!(!raw_expect.is_empty(), "{EXPECT_ENV} is empty");
    ensure!(
        raw_expect == EXPECTED_AUTHORITY,
        "{EXPECT_ENV} must be exactly the authorized test authority {EXPECTED_AUTHORITY}"
    );

    let parsed =
        Url::parse(raw_url).map_err(|_| anyhow!("{URL_ENV} is not a valid absolute URL"))?;
    ensure!(
        parsed.scheme() == "http",
        "{URL_ENV} must use the http scheme of the authorized test endpoint"
    );
    ensure!(
        parsed.username().is_empty() && parsed.password().is_none(),
        "{URL_ENV} must not carry userinfo"
    );
    ensure!(
        parsed.query().is_none(),
        "{URL_ENV} must not carry a query string"
    );
    ensure!(
        parsed.fragment().is_none(),
        "{URL_ENV} must not carry a fragment"
    );
    ensure!(
        parsed.path().is_empty() || parsed.path() == "/",
        "{URL_ENV} must not carry a path"
    );
    ensure!(
        parsed.host_str() == Some(EXPECTED_HOST),
        "{URL_ENV} must address the authorized test host {EXPECTED_HOST}"
    );
    ensure!(
        parsed.port() != Some(PRODUCTION_PORT),
        "{URL_ENV} must not address the production port {PRODUCTION_PORT}"
    );
    ensure!(
        parsed.port() == Some(EXPECTED_PORT),
        "{URL_ENV} must address the authorized test port {EXPECTED_PORT}"
    );
    ensure!(
        parsed.as_str().trim_end_matches('/') == AUTHORIZED_TEST_URL,
        "{URL_ENV} must be exactly {AUTHORIZED_TEST_URL}"
    );

    Ok(AUTHORIZED_TEST_URL.to_string())
}

fn build_client() -> Result<Client> {
    Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        .timeout(REQUEST_TIMEOUT)
        .build()
        .context("failed to build the HTTP client for the capability test")
}

async fn request(
    client: &Client,
    method: Method,
    url: &str,
    body: Option<Value>,
) -> Result<(StatusCode, String)> {
    let mut builder = client.request(method.clone(), url);
    if let Some(body) = body {
        builder = builder.json(&body);
    }
    let response = builder
        .send()
        .await
        .with_context(|| format!("{method} request to the authorized test endpoint failed"))?;
    let status = response.status();
    let destination = response.url().clone();
    ensure!(
        destination.host_str() == Some(EXPECTED_HOST) && destination.port() == Some(EXPECTED_PORT),
        "{method} request resolved to an unexpected destination"
    );
    let raw = response
        .text()
        .await
        .context("failed to read the response body")?;
    Ok((status, raw))
}

async fn expect_json(
    client: &Client,
    method: Method,
    url: &str,
    body: Option<Value>,
    expected_status: StatusCode,
    label: &str,
) -> Result<Value> {
    let (status, raw) = request(client, method, url, body)
        .await
        .with_context(|| format!("{label} request failed"))?;
    ensure!(
        status == expected_status,
        "{label}: expected HTTP {expected_status}, observed {status}: {}",
        truncate(&raw)
    );
    serde_json::from_str(&raw)
        .with_context(|| format!("{label}: response is not valid JSON: {}", truncate(&raw)))
}

async fn expect_write_completed(
    client: &Client,
    method: Method,
    url: &str,
    body: Option<Value>,
    label: &str,
) -> Result<Value> {
    let value = expect_json(client, method, url, body, StatusCode::OK, label).await?;
    let rendered = value.to_string();
    ensure!(
        value.get("status").and_then(Value::as_str) == Some("ok"),
        "{label}: unexpected envelope status: {rendered}"
    );
    let result = value
        .get("result")
        .ok_or_else(|| anyhow!("{label}: response carries no result field: {rendered}"))?;
    ensure!(
        result.get("status").and_then(Value::as_str) == Some("completed"),
        "{label}: the write did not reach the completed state: {rendered}"
    );
    ensure!(
        result.get("operation_id").is_some(),
        "{label}: the completed write carries no operation_id: {rendered}"
    );
    Ok(value)
}

async fn read_server_version(scratch: &Scratch<'_>) -> Result<String> {
    let value = expect_json(
        scratch.client,
        Method::GET,
        &scratch.base_url,
        None,
        StatusCode::OK,
        "read server root",
    )
    .await?;
    let title = value
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or_default();
    ensure!(
        title.contains("qdrant"),
        "the authorized test endpoint does not identify as a Qdrant server"
    );
    let version = value
        .get("version")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("the authorized test endpoint reports no version"))?
        .to_string();
    let parsed = parse_version(&version)?;
    ensure!(
        parsed >= MINIMUM_SERVER_VERSION,
        "server version {version} is below the required {}.{}.{}",
        MINIMUM_SERVER_VERSION.0,
        MINIMUM_SERVER_VERSION.1,
        MINIMUM_SERVER_VERSION.2
    );
    Ok(version)
}

fn parse_version(raw: &str) -> Result<(u64, u64, u64)> {
    let core = raw
        .split(['-', '+'])
        .next()
        .ok_or_else(|| anyhow!("unparsable server version"))?;
    let mut parts = core.split('.');
    let major = parts
        .next()
        .and_then(|value| value.parse().ok())
        .ok_or_else(|| anyhow!("unparsable server version"))?;
    let minor = parts
        .next()
        .and_then(|value| value.parse().ok())
        .ok_or_else(|| anyhow!("unparsable server version"))?;
    let patch = parts
        .next()
        .and_then(|value| value.parse().ok())
        .unwrap_or(0);
    Ok((major, minor, patch))
}

async fn ensure_collection_absent(scratch: &Scratch<'_>) -> Result<()> {
    let (status, raw) = request(
        scratch.client,
        Method::GET,
        &scratch.collection_base(),
        None,
    )
    .await
    .with_context(|| format!("pre-existence check for {} failed", scratch.collection))?;
    ensure!(
        status == StatusCode::NOT_FOUND,
        "collection {} already exists; refusing to touch a collection this run did not create",
        scratch.collection
    );
    ensure!(
        raw.contains("Not found") || raw.contains("not found"),
        "unexpected pre-existence response for {}: {}",
        scratch.collection,
        truncate(&raw)
    );
    Ok(())
}

async fn create_collection(scratch: &Scratch<'_>) -> Result<()> {
    let body = json!({
        "vectors": { "size": VECTOR_SIZE, "distance": "Cosine" }
    });
    let value = expect_json(
        scratch.client,
        Method::PUT,
        &scratch.collection_base(),
        Some(body),
        StatusCode::OK,
        "create scratch collection",
    )
    .await?;
    ensure!(
        value.get("result").and_then(Value::as_bool) == Some(true),
        "creating {} did not report success: {}",
        scratch.collection,
        truncate(&value.to_string())
    );
    Ok(())
}

/// Create the payload indexes the search filter relies on.
///
/// `library_file_id` is a keyword field and `document_id` an integer field in
/// the production collection, and the entity exclusion clauses match them with
/// `match any`. Creating them here keeps the scratch proof on the same index
/// shape as production instead of a full scan.
async fn ensure_entity_field_indexes(scratch: &Scratch<'_>) -> Result<()> {
    for (field, schema) in [("library_file_id", "keyword"), ("document_id", "integer")] {
        let value = expect_json(
            scratch.client,
            Method::PUT,
            &scratch.index_url(),
            Some(json!({ "field_name": field, "field_schema": schema })),
            StatusCode::OK,
            "create scratch payload index",
        )
        .await?;
        // `wait=true` answers either the boolean result of the older API or the
        // completed operation object of the newer one; both mean the index is
        // in place before the filter runs.
        let created = value.get("result").is_some_and(|result| match result {
            Value::Bool(created) => *created,
            Value::Object(_) => result.get("status").and_then(Value::as_str) == Some("completed"),
            _ => false,
        });
        ensure!(
            created,
            "creating the {field} index did not report success: {}",
            truncate(&value.to_string())
        );
    }
    Ok(())
}

async fn delete_collection(scratch: &Scratch<'_>) -> Result<()> {
    let (status, raw) = request(
        scratch.client,
        Method::DELETE,
        &scratch.collection_base(),
        None,
    )
    .await
    .with_context(|| format!("cleanup of {} failed", scratch.collection))?;
    ensure!(
        status == StatusCode::OK || status == StatusCode::NOT_FOUND,
        "cleanup of {} returned HTTP {status}: {}",
        scratch.collection,
        truncate(&raw)
    );

    let (verify_status, verify_raw) = request(
        scratch.client,
        Method::GET,
        &scratch.collection_base(),
        None,
    )
    .await
    .with_context(|| format!("cleanup verification for {} failed", scratch.collection))?;
    ensure!(
        verify_status == StatusCode::NOT_FOUND,
        "collection {} was retained after cleanup: {}",
        scratch.collection,
        truncate(&verify_raw)
    );
    println!(
        "collection {} created by this run was deleted and verified absent",
        scratch.collection
    );
    Ok(())
}

async fn run_capability_cases(scratch: &Scratch<'_>) -> Result<()> {
    let live_id = Uuid::new_v4().to_string();
    let r1_pending_id = Uuid::new_v4().to_string();
    let r2_pending_id = Uuid::new_v4().to_string();
    let r3_pending_id = Uuid::new_v4().to_string();
    let r2_live_id = Uuid::new_v4().to_string();
    let no_state_id = Uuid::new_v4().to_string();
    let staged_id = Uuid::new_v4().to_string();
    // Entity tombstone fixtures (issue #1138 P24): the two highest-ranked
    // points of the P24 run belong to a deleted file and a deleted document,
    // the lowest-ranked point is live.
    let deleted_file_id = Uuid::new_v4();
    let deleted_document_id: i64 = 8_675_309;
    let deleted_file_point = Uuid::new_v4().to_string();
    let deleted_document_point = Uuid::new_v4().to_string();
    let survive_point = Uuid::new_v4().to_string();

    let fixtures = json!({
        "points": [
            { "id": live_id, "vector": [1.0, 0.0, 0.0, 0.0], "payload": { "run": "R1", "state": "live", "marker": "live" } },
            { "id": r1_pending_id, "vector": [0.0, 1.0, 0.0, 0.0], "payload": { "run": "R1", "state": "pending" } },
            { "id": r2_pending_id, "vector": [0.0, 0.0, 1.0, 0.0], "payload": { "run": "R2", "state": "pending" } },
            { "id": r3_pending_id, "vector": [0.5, 0.0, 0.5, 0.0], "payload": { "run": "R3", "state": "pending" } },
            { "id": r2_live_id, "vector": [0.0, 0.0, 0.0, 1.0], "payload": { "run": "R2", "state": "live" } },
            { "id": no_state_id, "vector": [1.0, 1.0, 0.0, 0.0] },
            { "id": staged_id, "vector": [1.0, 0.0, 1.0, 0.0], "payload": { "run": "R4", "state": "staged" } },
            { "id": deleted_file_point, "vector": [1.0, 0.0, 0.0, 0.0], "payload": { "run": "P24", "library_file_id": deleted_file_id } },
            { "id": deleted_document_point, "vector": [0.0, 1.0, 0.0, 0.0], "payload": { "run": "P24", "document_id": deleted_document_id } },
            { "id": survive_point, "vector": [0.5, 0.5, 0.0, 0.0], "payload": { "run": "P24" } }
        ]
    });
    expect_write_completed(
        scratch.client,
        Method::PUT,
        &scratch.upsert_url(),
        Some(fixtures),
        "insert fixture points",
    )
    .await?;

    case_insert_only_keeps_live_payload(scratch, &live_id).await?;
    case_filtered_set_payload(scratch, &r1_pending_id, &r2_pending_id).await?;
    case_filtered_delete(scratch, &r2_pending_id, &r3_pending_id).await?;
    case_reader_filter(scratch, &live_id, &no_state_id, &staged_id, &r3_pending_id).await?;
    case_retrieve_omits_missing_ids(scratch, &r2_live_id).await?;
    case_deleted_file_does_not_consume_the_limit(
        scratch,
        &deleted_file_id,
        &deleted_file_point,
        &survive_point,
    )
    .await?;
    case_deleted_document_does_not_consume_the_limit(
        scratch,
        deleted_document_id,
        &deleted_document_point,
        &survive_point,
    )
    .await?;

    Ok(())
}

/// A deleted file's highest-ranked point is filtered before the limit, so the
/// live point below it still fills the single result slot.
async fn case_deleted_file_does_not_consume_the_limit(
    scratch: &Scratch<'_>,
    deleted_file_id: &Uuid,
    deleted_point_id: &str,
    survive_point_id: &str,
) -> Result<()> {
    let body = json!({
        "vector": [1.0, 0.0, 0.0, 0.0],
        "limit": 1,
        "with_payload": true,
        "filter": {
            "must": [ { "key": "run", "match": { "value": "P24" } } ],
            "must_not": [
                { "key": "library_file_id", "match": { "any": [deleted_file_id.to_string()] } }
            ]
        }
    });
    let value = expect_json(
        scratch.client,
        Method::POST,
        &scratch.search_url(),
        Some(body),
        StatusCode::OK,
        "deleted file exclusion search",
    )
    .await?;
    let hits = value
        .get("result")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("deleted file exclusion search returned no result array"))?;
    ensure!(
        hits.len() == 1,
        "limit=1 returned {} hits after the deleted file exclusion",
        hits.len()
    );
    let id = record_id(&hits[0]);
    ensure!(
        id != deleted_point_id,
        "the deleted file's point still occupied the candidate slot"
    );
    ensure!(
        id == survive_point_id,
        "the live point below the deleted file must fill the slot, got {id}"
    );
    println!("a deleted file point was excluded before the limit=1 slice");
    Ok(())
}

/// The same proof for the deleted document identity.
async fn case_deleted_document_does_not_consume_the_limit(
    scratch: &Scratch<'_>,
    deleted_document_id: i64,
    deleted_point_id: &str,
    survive_point_id: &str,
) -> Result<()> {
    let body = json!({
        "vector": [0.0, 1.0, 0.0, 0.0],
        "limit": 1,
        "with_payload": true,
        "filter": {
            "must": [ { "key": "run", "match": { "value": "P24" } } ],
            "must_not": [
                { "key": "document_id", "match": { "any": [deleted_document_id] } }
            ]
        }
    });
    let value = expect_json(
        scratch.client,
        Method::POST,
        &scratch.search_url(),
        Some(body),
        StatusCode::OK,
        "deleted document exclusion search",
    )
    .await?;
    let hits = value
        .get("result")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("deleted document exclusion search returned no result array"))?;
    ensure!(
        hits.len() == 1,
        "limit=1 returned {} hits after the deleted document exclusion",
        hits.len()
    );
    let id = record_id(&hits[0]);
    ensure!(
        id != deleted_point_id,
        "the deleted document's point still occupied the candidate slot"
    );
    ensure!(
        id == survive_point_id,
        "the live point below the deleted document must fill the slot, got {id}"
    );
    println!("a deleted document point was excluded before the limit=1 slice");
    Ok(())
}

async fn case_insert_only_keeps_live_payload(scratch: &Scratch<'_>, point_id: &str) -> Result<()> {
    let body = json!({
        "points": [
            {
                "id": point_id,
                "vector": [0.5, 0.5, 0.5, 0.5],
                "payload": { "run": "R1", "state": "staged", "marker": "replacement" }
            }
        ],
        "update_mode": "insert_only"
    });
    expect_write_completed(
        scratch.client,
        Method::PUT,
        &scratch.upsert_url(),
        Some(body),
        "insert_only conditional upsert",
    )
    .await?;

    let records = retrieve(scratch, &[point_id.to_string()]).await?;
    ensure!(
        records.len() == 1,
        "insert_only: expected exactly one retrievable point, observed {}",
        records.len()
    );
    let payload = records[0]
        .get("payload")
        .and_then(Value::as_object)
        .ok_or_else(|| anyhow!("insert_only: the point carries no payload"))?;
    ensure!(
        payload.get("state").and_then(Value::as_str) == Some("live"),
        "insert_only overwrote the live state: {}",
        truncate(&records[0].to_string())
    );
    ensure!(
        payload.get("marker").and_then(Value::as_str) == Some("live"),
        "insert_only overwrote the live payload marker: {}",
        truncate(&records[0].to_string())
    );
    println!("insert_only left the live payload untouched");
    Ok(())
}

async fn case_filtered_set_payload(
    scratch: &Scratch<'_>,
    matching_id: &str,
    other_id: &str,
) -> Result<()> {
    let body = json!({
        "payload": { "state": "live" },
        "filter": run_and_state_filter("R1", "pending")
    });
    expect_write_completed(
        scratch.client,
        Method::POST,
        &scratch.set_payload_url(),
        Some(body),
        "selector-filtered set payload",
    )
    .await?;

    let records = retrieve(scratch, &[matching_id.to_string(), other_id.to_string()]).await?;
    let matching = point_payload(&records, matching_id)?;
    let other = point_payload(&records, other_id)?;
    ensure!(
        matching.get("state").and_then(Value::as_str) == Some("live"),
        "the selector-filtered set payload did not update the matching point"
    );
    ensure!(
        other.get("state").and_then(Value::as_str) == Some("pending"),
        "the selector-filtered set payload changed a non-matching point"
    );
    println!("selector-filtered set payload touched only the matching run and state");
    Ok(())
}

async fn case_filtered_delete(
    scratch: &Scratch<'_>,
    matching_id: &str,
    other_id: &str,
) -> Result<()> {
    let body = json!({ "filter": run_and_state_filter("R2", "pending") });
    expect_write_completed(
        scratch.client,
        Method::POST,
        &scratch.delete_points_url(),
        Some(body.clone()),
        "selector-filtered delete",
    )
    .await?;
    expect_write_completed(
        scratch.client,
        Method::POST,
        &scratch.delete_points_url(),
        Some(body),
        "repeated selector-filtered delete",
    )
    .await?;

    let records = retrieve(scratch, &[matching_id.to_string(), other_id.to_string()]).await?;
    ensure!(
        !records
            .iter()
            .any(|record| record_id(record) == matching_id),
        "the selector-filtered delete kept the matching point"
    );
    let other = point_payload(&records, other_id)?;
    ensure!(
        other.get("state").and_then(Value::as_str) == Some("pending"),
        "the selector-filtered delete changed a non-matching point"
    );
    println!("selector-filtered delete removed only the matching run and state");
    Ok(())
}

async fn case_reader_filter(
    scratch: &Scratch<'_>,
    live_id: &str,
    no_state_id: &str,
    staged_id: &str,
    pending_id: &str,
) -> Result<()> {
    let body = json!({
        "filter": {
            "must_not": [
                { "key": "state", "match": { "value": "staged" } },
                { "key": "state", "match": { "value": "pending" } }
            ]
        },
        "limit": 100,
        "with_payload": true
    });
    let value = expect_json(
        scratch.client,
        Method::POST,
        &scratch.scroll_url(),
        Some(body),
        StatusCode::OK,
        "reader filter scroll",
    )
    .await?;
    let points = value
        .get("result")
        .and_then(|result| result.get("points"))
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("reader filter scroll returned no points array"))?;

    let ids: Vec<String> = points.iter().map(record_id).collect();
    ensure!(
        ids.iter().any(|id| id == live_id),
        "the reader filter excluded the live point"
    );
    ensure!(
        ids.iter().any(|id| id == no_state_id),
        "the reader filter excluded the point without a state"
    );
    ensure!(
        !ids.iter().any(|id| id == staged_id),
        "the reader filter admitted the staged point"
    );
    ensure!(
        !ids.iter().any(|id| id == pending_id),
        "the reader filter admitted the pending point"
    );
    println!("reader filter admitted absent and live points and excluded pending and staged ones");
    Ok(())
}

async fn case_retrieve_omits_missing_ids(scratch: &Scratch<'_>, existing_id: &str) -> Result<()> {
    let missing_id = Uuid::new_v4().to_string();
    let records = retrieve(scratch, &[existing_id.to_string(), missing_id.clone()]).await?;
    ensure!(
        records.len() == 1,
        "retrieve returned {} records for one existing and one missing id",
        records.len()
    );
    ensure!(
        record_id(&records[0]) == existing_id,
        "retrieve returned a record other than the existing id"
    );
    ensure!(
        !records.iter().any(|record| record_id(record) == missing_id),
        "retrieve returned the missing id"
    );
    println!("retrieve omitted the missing id");
    Ok(())
}

async fn retrieve(scratch: &Scratch<'_>, ids: &[String]) -> Result<Vec<Value>> {
    let body = json!({ "ids": ids, "with_payload": true });
    let value = expect_json(
        scratch.client,
        Method::POST,
        &scratch.retrieve_url(),
        Some(body),
        StatusCode::OK,
        "retrieve points",
    )
    .await?;
    let records = value
        .get("result")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("retrieve points returned no result array"))?;
    Ok(records.clone())
}

fn point_payload(records: &[Value], point_id: &str) -> Result<Value> {
    let record = records
        .iter()
        .find(|record| record_id(record) == point_id)
        .ok_or_else(|| anyhow!("point {point_id} was not returned"))?;
    record
        .get("payload")
        .cloned()
        .ok_or_else(|| anyhow!("point {point_id} carries no payload"))
}

fn record_id(record: &Value) -> String {
    record
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn run_and_state_filter(run: &str, state: &str) -> Value {
    json!({
        "must": [
            { "key": "run", "match": { "value": run } },
            { "key": "state", "match": { "value": state } }
        ]
    })
}

fn truncate(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.chars().count() <= 400 {
        return trimmed.to_string();
    }
    let head: String = trimmed.chars().take(400).collect();
    format!("{head}...")
}
