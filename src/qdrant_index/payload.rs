use chrono::{DateTime, Utc};
use qdrant_client::qdrant::{Condition, DatetimeRange, Filter, Range, Timestamp};
use serde_json::json;
use uuid::Uuid;

use super::date_to_timestamp;
use crate::contracts::{MetadataFilter, MetadataFilterOperator};
use crate::domain::ChunkPayload;

/// Top-level payload key carrying the internal index state of a point. The
/// `c69_` namespace keeps the marker clear of user metadata, which only ever
/// lives nested under `metadata_index`.
pub(super) const POINT_STATE_KEY: &str = "c69_ix_state";

/// Indexed payload field that carries the library file a point belongs to.
pub(super) const LIBRARY_FILE_ID_FIELD: &str = "library_file_id";

/// Indexed payload field that carries the document a point belongs to.
pub(super) const DOCUMENT_ID_FIELD: &str = "document_id";

/// Index states that are never visible to search. Every other value — a live
/// value, an unrelated value, or no value at all — stays visible.
const HIDDEN_POINT_STATES: [&str; 2] = ["staged", "pending"];

pub(super) fn chunk_payload_json(payload: &ChunkPayload) -> serde_json::Value {
    let mut payload_json = json!({
        "chunk_id": payload.chunk_id.to_string(),
        "document_id": payload.document_id,
        "group_id": payload.group_id,
        "group_key": payload.group_key,
        "group_path": payload.group_path,
        "visibility": payload.visibility.as_str(),
        "source_key": payload.source_key,
        "external_id": payload.external_id,
        "title": payload.title,
        "source_uri": payload.source_uri,
        "published_ts": payload.published_at.map(date_to_timestamp),
        "record_hash": payload.record_hash,
        "chunk_index": payload.chunk_index,
        "canonical_document_id": payload.document_id,
        "content_locale": payload.content_locale,
        "source_locale": payload.source_locale,
        "translation_provider": payload.translation_provider,
    });

    if let Some(value) = payload.metadata_json.get("is_library_file") {
        payload_json["is_library_file"] = value.clone();
    }
    if let Some(value) = payload.metadata_json.get("library_file_id") {
        payload_json["library_file_id"] = value.clone();
    }
    if let Some(value) = payload.metadata_json.get("library_path") {
        payload_json["library_path"] = value.clone();
    }
    if let Some(value) = payload.metadata_json.get("library_section_label") {
        payload_json["library_section_label"] = value.clone();
    }
    payload_json["metadata_index"] = payload.metadata_json.clone();

    payload_json
}

/// Condition that hides points whose index state is one of the hidden states.
///
/// The clause is a `match`-based `must_not`, never an `is_empty` test: a
/// point without the key (every legacy point), a point whose value is null,
/// and a point carrying a live or unrelated value never match it and stay
/// visible. Only the exact hidden values are excluded.
pub(super) fn hidden_point_condition() -> Condition {
    Condition::from(Filter::must_not(
        HIDDEN_POINT_STATES
            .iter()
            .map(|state| Condition::matches(POINT_STATE_KEY, state.to_string()))
            .collect::<Vec<_>>(),
    ))
}

/// The `must_not` clause that hides every point of a deleted library file.
///
/// The clause matches the indexed `library_file_id` payload field against any
/// of the deleted file ids, so one clause excludes a whole file without one
/// registry reservation per point. An empty set adds no clause at all.
pub(super) fn deleted_file_condition(file_ids: &[Uuid]) -> Option<Condition> {
    if file_ids.is_empty() {
        return None;
    }
    Some(Condition::matches(
        LIBRARY_FILE_ID_FIELD,
        file_ids.iter().map(Uuid::to_string).collect::<Vec<_>>(),
    ))
}

/// The `must_not` clause that hides every point of a deleted document.
///
/// The clause matches the indexed `document_id` payload field against any of
/// the deleted document ids. An empty set adds no clause at all.
pub(super) fn deleted_document_condition(document_ids: &[i64]) -> Option<Condition> {
    if document_ids.is_empty() {
        return None;
    }
    Some(Condition::matches(DOCUMENT_ID_FIELD, document_ids.to_vec()))
}

pub(super) fn metadata_filter_condition(filter: &MetadataFilter) -> Option<Condition> {
    let key = format!("metadata_index.{}", filter.path);
    match filter.operator {
        MetadataFilterOperator::Exists => {
            let exists = filter
                .value
                .as_ref()
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(true);
            if exists {
                Some(Condition::from(Filter::must_not([Condition::is_empty(
                    key,
                )])))
            } else {
                Some(Condition::is_empty(key))
            }
        }
        MetadataFilterOperator::Eq | MetadataFilterOperator::Contains => filter
            .value
            .as_ref()
            .and_then(|value| qdrant_match_condition(&key, value)),
        MetadataFilterOperator::In => {
            let values = filter.value.as_ref()?.as_array()?;
            let conditions = values
                .iter()
                .filter_map(|value| qdrant_match_condition(&key, value))
                .collect::<Vec<_>>();
            (!conditions.is_empty()).then(|| Condition::from(Filter::should(conditions)))
        }
        MetadataFilterOperator::Range => {
            let min = filter.min.as_ref().and_then(serde_json::Value::as_f64);
            let max = filter.max.as_ref().and_then(serde_json::Value::as_f64);
            if (min.is_some() || max.is_some())
                && filter
                    .min
                    .as_ref()
                    .is_none_or(|value| value.as_f64().is_some())
                && filter
                    .max
                    .as_ref()
                    .is_none_or(|value| value.as_f64().is_some())
            {
                return Some(Condition::range(
                    key,
                    Range {
                        gte: min,
                        lte: max,
                        ..Default::default()
                    },
                ));
            }

            let min = filter.min.as_ref().and_then(qdrant_datetime);
            let max = filter.max.as_ref().and_then(qdrant_datetime);
            (filter.min.as_ref().is_none_or(|_| min.is_some())
                && filter.max.as_ref().is_none_or(|_| max.is_some())
                && (min.is_some() || max.is_some()))
            .then(|| {
                Condition::datetime_range(
                    key,
                    DatetimeRange {
                        gte: min,
                        lte: max,
                        ..Default::default()
                    },
                )
            })
        }
    }
}

fn qdrant_datetime(value: &serde_json::Value) -> Option<Timestamp> {
    let value = value.as_str()?;
    let value = DateTime::parse_from_rfc3339(value)
        .ok()?
        .with_timezone(&Utc);
    Some(Timestamp {
        seconds: value.timestamp(),
        nanos: value.timestamp_subsec_nanos() as i32,
    })
}

fn qdrant_match_condition(key: &str, value: &serde_json::Value) -> Option<Condition> {
    if let Some(value) = value.as_str() {
        return Some(Condition::matches(key, value.to_string()));
    }
    if let Some(value) = value.as_i64() {
        return Some(Condition::matches(key, value));
    }
    if let Some(value) = value.as_bool() {
        return Some(Condition::matches(key, value));
    }
    value
        .as_f64()
        .filter(|value| value.is_finite())
        .map(|value| {
            Condition::range(
                key,
                Range {
                    gte: Some(value),
                    lte: Some(value),
                    ..Default::default()
                },
            )
        })
}
