use anyhow::{Result, anyhow};
use chrono::{DateTime, Utc};
use context69_contracts::MetadataValueKind;
use serde_json::Value;

use crate::db::StoredMetadataIndex;

#[derive(Debug, Clone, Default)]
pub struct TypedMetadataValue {
    pub keyword_value: Option<String>,
    pub integer_value: Option<i64>,
    pub float_value: Option<f64>,
    pub boolean_value: Option<bool>,
    pub datetime_value: Option<DateTime<Utc>>,
}

pub fn validate_definition(
    path: &str,
    value_kind: MetadataValueKind,
    sortable: bool,
) -> Result<()> {
    let path = path.trim();
    if path.is_empty() || path.len() > 200 {
        return Err(anyhow!("metadata path must contain 1..=200 characters"));
    }
    if path.split('.').count() > 8 {
        return Err(anyhow!("metadata path must have at most 8 segments"));
    }
    if path.split('.').any(|segment| {
        segment.is_empty()
            || !segment
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
    }) {
        return Err(anyhow!(
            "metadata path segments may contain only ASCII letters, digits, '_' and '-'"
        ));
    }
    if sortable && value_kind == MetadataValueKind::Array {
        return Err(anyhow!("array metadata indexes cannot be sortable"));
    }
    Ok(())
}

pub fn extract_values(
    definition: &StoredMetadataIndex,
    metadata: &Value,
) -> Result<Vec<TypedMetadataValue>> {
    let Some(value) = resolve_path(metadata, &definition.field_path) else {
        return Ok(Vec::new());
    };
    if value.is_null() {
        return Ok(Vec::new());
    }
    let values = if definition.value_kind == "array" {
        value
            .as_array()
            .ok_or_else(|| anyhow!("expected array"))?
            .iter()
            .collect::<Vec<_>>()
    } else {
        vec![value]
    };
    values
        .into_iter()
        .map(|value| typed_value(&definition.data_type, value))
        .collect()
}

pub fn resolve_path<'a>(value: &'a Value, path: &str) -> Option<&'a Value> {
    path.split('.')
        .try_fold(value, |current, segment| current.as_object()?.get(segment))
}

fn typed_value(data_type: &str, value: &Value) -> Result<TypedMetadataValue> {
    let mut result = TypedMetadataValue::default();
    match data_type {
        "keyword" => {
            result.keyword_value = Some(
                value
                    .as_str()
                    .ok_or_else(|| anyhow!("expected string"))?
                    .to_string(),
            )
        }
        "integer" => {
            result.integer_value = Some(value.as_i64().ok_or_else(|| anyhow!("expected integer"))?)
        }
        "float" => {
            result.float_value = Some(value.as_f64().ok_or_else(|| anyhow!("expected number"))?)
        }
        "boolean" => {
            result.boolean_value = Some(value.as_bool().ok_or_else(|| anyhow!("expected boolean"))?)
        }
        "datetime" => {
            result.datetime_value = Some(
                DateTime::parse_from_rfc3339(
                    value
                        .as_str()
                        .ok_or_else(|| anyhow!("expected RFC 3339 string"))?,
                )?
                .with_timezone(&Utc),
            )
        }
        _ => return Err(anyhow!("unsupported metadata data type {data_type}")),
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::{extract_values, resolve_path, validate_definition};
    use crate::db::StoredMetadataIndex;
    use chrono::Utc;
    use context69_contracts::MetadataValueKind;
    use serde_json::json;
    use uuid::Uuid;

    fn definition(data_type: &str, value_kind: &str) -> StoredMetadataIndex {
        StoredMetadataIndex {
            index_id: Uuid::nil(),
            group_id: 1,
            group_path: "group".to_string(),
            source_key: "news".to_string(),
            field_path: "facts.values".to_string(),
            data_type: data_type.to_string(),
            value_kind: value_kind.to_string(),
            sortable: value_kind == "scalar",
            status: "ready".to_string(),
            processed_documents: 0,
            total_documents: 0,
            error_message: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn resolves_nested_dot_path() {
        let value = json!({"provider": {"name": "wire"}});
        assert_eq!(resolve_path(&value, "provider.name"), Some(&json!("wire")));
    }

    #[test]
    fn rejects_invalid_and_sortable_array_definitions() {
        assert!(validate_definition("provider..name", MetadataValueKind::Scalar, false).is_err());
        assert!(validate_definition("tags", MetadataValueKind::Array, true).is_err());
    }

    #[test]
    fn extracts_typed_array_values_and_rejects_type_drift() {
        let values = extract_values(
            &definition("integer", "array"),
            &json!({"facts": {"values": [2, 10]}}),
        )
        .expect("typed values");
        assert_eq!(
            values
                .iter()
                .map(|value| value.integer_value)
                .collect::<Vec<_>>(),
            [Some(2), Some(10)]
        );

        assert!(
            extract_values(
                &definition("integer", "array"),
                &json!({"facts": {"values": [2, "10"]}}),
            )
            .is_err()
        );
        assert!(
            extract_values(
                &definition("integer", "array"),
                &json!({"facts": {"values": 2}}),
            )
            .is_err()
        );
    }
}
