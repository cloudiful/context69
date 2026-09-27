use anyhow::Result;

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use context69_contracts::DocumentQueryRequest;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::domain_errors::DomainError;

use super::sorting::SortValue;

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Cursor {
    pub(super) version: u8,
    pub(super) query_hash: String,
    pub(super) values: Vec<SortValue>,
    pub(super) document_id: i64,
}

pub(super) fn query_hash(request: &DocumentQueryRequest) -> Result<String> {
    let mut normalized = request.clone();
    normalized.cursor = None;
    let digest = Sha256::digest(serde_json::to_vec(&normalized)?);
    Ok(digest.iter().map(|byte| format!("{byte:02x}")).collect())
}

pub(super) fn encode_cursor(cursor: &Cursor) -> Result<String> {
    Ok(URL_SAFE_NO_PAD.encode(serde_json::to_vec(cursor)?))
}

pub(super) fn decode_cursor(value: Option<&str>, hash: &str) -> Result<Option<Cursor>> {
    let Some(value) = value else {
        return Ok(None);
    };
    let cursor: Cursor = serde_json::from_slice(
        &URL_SAFE_NO_PAD
            .decode(value)
            .map_err(|_| DomainError::invalid_argument("invalid cursor"))?,
    )?;
    if cursor.version != 2 || cursor.query_hash != hash {
        return Err(DomainError::invalid_argument("cursor does not match query").into());
    }
    Ok(Some(cursor))
}

#[cfg(test)]
mod tests {
    use super::{Cursor, decode_cursor, encode_cursor, query_hash};
    use context69_contracts::DocumentQueryRequest;

    fn request() -> DocumentQueryRequest {
        DocumentQueryRequest {
            locale: None,
            source_key: Some("news".to_string()),
            published_after: None,
            published_before: None,
            metadata_filters: Vec::new(),
            sort: Vec::new(),
            limit: 50,
            cursor: None,
        }
    }

    #[test]
    fn cursor_is_bound_to_query_and_ignores_cursor_field_in_hash() {
        let mut query = request();
        let hash = query_hash(&query).unwrap();
        let encoded = encode_cursor(&Cursor {
            version: 2,
            query_hash: hash.clone(),
            values: Vec::new(),
            document_id: 42,
        })
        .unwrap();
        query.cursor = Some(encoded.clone());

        assert_eq!(query_hash(&query).unwrap(), hash);
        assert_eq!(
            decode_cursor(Some(&encoded), &hash)
                .unwrap()
                .expect("cursor")
                .document_id,
            42
        );
        assert!(decode_cursor(Some(&encoded), "different-query").is_err());
        assert!(decode_cursor(Some("not-base64"), &hash).is_err());
    }

    #[test]
    fn cursor_failures_are_typed_invalid_argument() {
        use crate::domain_errors::{DomainError, find_domain_error};

        let assert_invalid = |error: anyhow::Error, expected: &str| {
            let message = error.to_string();
            assert!(
                matches!(
                    find_domain_error(&error),
                    Some(DomainError::InvalidArgument(_))
                ),
                "expected typed invalid_argument for {message}"
            );
            assert_eq!(message, expected);
        };

        let hash = query_hash(&request()).unwrap();
        let Err(error) = decode_cursor(Some("not-base64"), &hash) else {
            panic!("expected invalid cursor");
        };
        assert_invalid(error, "invalid cursor");

        let encoded = encode_cursor(&Cursor {
            version: 2,
            query_hash: "different-query".to_string(),
            values: Vec::new(),
            document_id: 1,
        })
        .unwrap();
        let Err(error) = decode_cursor(Some(&encoded), &hash) else {
            panic!("expected cursor mismatch");
        };
        assert_invalid(error, "cursor does not match query");
    }
}
