use anyhow::Result;

use context69_contracts::{DocumentQueryRequest, MetadataFilterOperator};
use serde_json::Value;

use crate::db::StoredMetadataIndex;

use super::metadata;

pub(super) fn filters_match(
    metadata_json: &Value,
    request: &DocumentQueryRequest,
    definitions: &[StoredMetadataIndex],
) -> Result<bool> {
    for filter in &request.metadata_filters {
        let found = metadata::resolve_path(metadata_json, &filter.path);
        let data_type = definitions
            .iter()
            .find(|definition| definition.field_path == filter.path)
            .map(|definition| definition.data_type.as_str());
        let matched = match filter.operator {
            MetadataFilterOperator::Exists => {
                found.is_some_and(|value| !value.is_null())
                    == filter
                        .value
                        .as_ref()
                        .and_then(Value::as_bool)
                        .unwrap_or(true)
            }
            MetadataFilterOperator::Eq => found == filter.value.as_ref(),
            MetadataFilterOperator::In => filter
                .value
                .as_ref()
                .and_then(Value::as_array)
                .is_some_and(|values| found.is_some_and(|value| values.contains(value))),
            MetadataFilterOperator::Contains => {
                found.and_then(Value::as_array).is_some_and(|values| {
                    filter
                        .value
                        .as_ref()
                        .is_some_and(|value| values.contains(value))
                })
            }
            MetadataFilterOperator::Range => found.is_some_and(|value| {
                json_range(value, filter.min.as_ref(), filter.max.as_ref(), data_type)
            }),
        };
        if !matched {
            return Ok(false);
        }
    }
    Ok(true)
}

fn json_range(
    value: &Value,
    min: Option<&Value>,
    max: Option<&Value>,
    data_type: Option<&str>,
) -> bool {
    if let Some(data_type) = data_type {
        return match data_type {
            "integer" => typed_numeric_range(value, min, max, Value::as_i64),
            "float" => typed_numeric_range(value, min, max, Value::as_f64),
            "keyword" => typed_string_range(value, min, max),
            "datetime" => typed_datetime_range(value, min, max),
            "boolean" => min.is_none() && max.is_none() && value.is_boolean(),
            _ => false,
        };
    }

    let compare = |left: &Value, right: &Value| match (left.as_f64(), right.as_f64()) {
        (Some(left), Some(right)) => left.partial_cmp(&right),
        _ => left
            .as_str()
            .zip(right.as_str())
            .map(|(left, right)| left.cmp(right)),
    };
    min.is_none_or(|bound| compare(value, bound).is_some_and(|order| order.is_ge()))
        && max.is_none_or(|bound| compare(value, bound).is_some_and(|order| order.is_le()))
}

fn typed_numeric_range<T, F>(
    value: &Value,
    min: Option<&Value>,
    max: Option<&Value>,
    convert: F,
) -> bool
where
    T: PartialOrd,
    F: Fn(&Value) -> Option<T> + Copy,
{
    let Some(value) = convert(value) else {
        return false;
    };
    let lower = min.and_then(convert);
    let upper = max.and_then(convert);
    (min.is_none() || lower.is_some())
        && (max.is_none() || upper.is_some())
        && lower.is_none_or(|bound| value >= bound)
        && upper.is_none_or(|bound| value <= bound)
}

fn typed_string_range(value: &Value, min: Option<&Value>, max: Option<&Value>) -> bool {
    let Some(value) = value.as_str() else {
        return false;
    };
    let lower = min.and_then(Value::as_str);
    let upper = max.and_then(Value::as_str);
    (min.is_none() || lower.is_some())
        && (max.is_none() || upper.is_some())
        && lower.is_none_or(|bound| value >= bound)
        && upper.is_none_or(|bound| value <= bound)
}

fn typed_datetime_range(value: &Value, min: Option<&Value>, max: Option<&Value>) -> bool {
    let parse = |value: &Value| {
        value
            .as_str()
            .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
            .map(|value| value.with_timezone(&chrono::Utc))
    };
    typed_numeric_range(value, min, max, parse)
}

#[cfg(test)]
mod tests {
    use super::filters_match;
    use context69_contracts::{DocumentQueryRequest, MetadataFilter, MetadataFilterOperator};
    use serde_json::{Value, json};

    fn request(filters: Vec<MetadataFilter>) -> DocumentQueryRequest {
        DocumentQueryRequest {
            locale: None,
            source_key: Some("news".to_string()),
            published_after: None,
            published_before: None,
            metadata_filters: filters,
            sort: Vec::new(),
            limit: 50,
            cursor: None,
        }
    }

    fn filter(
        path: &str,
        operator: MetadataFilterOperator,
        value: Option<Value>,
        min: Option<Value>,
        max: Option<Value>,
    ) -> MetadataFilter {
        MetadataFilter {
            path: path.to_string(),
            operator,
            value,
            min,
            max,
        }
    }

    #[test]
    fn metadata_operators_compose_with_and_semantics() {
        let metadata = json!({
            "provider": {"name": "wire"},
            "score": 10,
            "tags": ["earnings", "urgent"]
        });
        let matching = request(vec![
            filter(
                "provider.name",
                MetadataFilterOperator::Eq,
                Some(json!("wire")),
                None,
                None,
            ),
            filter(
                "score",
                MetadataFilterOperator::In,
                Some(json!([2, 10])),
                None,
                None,
            ),
            filter(
                "score",
                MetadataFilterOperator::Range,
                None,
                Some(json!(10)),
                Some(json!(20)),
            ),
            filter(
                "tags",
                MetadataFilterOperator::Contains,
                Some(json!("urgent")),
                None,
                None,
            ),
            filter(
                "provider.name",
                MetadataFilterOperator::Exists,
                None,
                None,
                None,
            ),
        ]);
        assert!(filters_match(&metadata, &matching, &[]).unwrap());

        let mut non_matching = matching;
        non_matching.metadata_filters.push(filter(
            "missing",
            MetadataFilterOperator::Exists,
            None,
            None,
            None,
        ));
        assert!(!filters_match(&metadata, &non_matching, &[]).unwrap());
    }

    #[test]
    fn exists_false_treats_missing_and_null_as_absent() {
        let absent = filter(
            "value",
            MetadataFilterOperator::Exists,
            Some(json!(false)),
            None,
            None,
        );
        assert!(filters_match(&json!({}), &request(vec![absent.clone()]), &[]).unwrap());
        assert!(filters_match(&json!({"value": null}), &request(vec![absent]), &[]).unwrap());
    }

    #[test]
    fn datetime_range_compares_instants_instead_of_timestamp_text() {
        assert!(super::json_range(
            &json!("2026-07-23T10:00:00+01:00"),
            Some(&json!("2026-07-23T09:00:00Z")),
            Some(&json!("2026-07-23T09:00:00Z")),
            Some("datetime"),
        ));
    }
}
