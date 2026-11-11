//! Caller-produced document-result envelope validation units.
//!
//! Blank identifiers, a non-object result, and an oversize serialized envelope
//! are refused by the shared contract before any I/O.

use context69::contracts::{CreateDocumentResultRequest, DOCUMENT_RESULT_REQUEST_MAX_BYTES};

#[test]
fn malformed_and_oversize_envelopes_are_rejected_before_any_io() {
    // Blank identifier alphabet, non-object result, and an oversize serialized
    // envelope are all refused by the shared contract.
    for bad in [
        serde_json::json!({
            "producer": "stock news", "result_kind": "k", "schema_ref": "s",
            "source_record_hash": "h", "idempotency_key": "i", "result": {}
        }),
        serde_json::json!({
            "producer": "stock.news", "result_kind": "k", "schema_ref": "s",
            "source_record_hash": "h", "idempotency_key": "i", "result": [1]
        }),
    ] {
        let parsed: CreateDocumentResultRequest =
            serde_json::from_value(bad).expect("envelope decodes");
        assert!(
            parsed.validate().is_err(),
            "invalid envelope must be refused"
        );
    }
    let oversize = serde_json::json!({
        "producer": "stock.news", "result_kind": "k", "schema_ref": "s",
        "source_record_hash": "h", "idempotency_key": "i",
        "result": {"blob": "a".repeat(DOCUMENT_RESULT_REQUEST_MAX_BYTES)}
    });
    let parsed: CreateDocumentResultRequest =
        serde_json::from_value(oversize).expect("oversize envelope decodes");
    assert!(
        parsed.validate().is_err(),
        "oversize envelope must be refused"
    );
}
