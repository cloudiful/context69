use anyhow::{Result, anyhow};
use chrono::{DateTime, Utc};
use context69_contracts::{MetadataFilter, MetadataFilterOperator};
use serde_json::Value;
use sqlx::{Postgres, QueryBuilder};

use crate::db::StoredMetadataIndex;

pub(super) fn definition_for_path<'a>(
    definitions: &'a [StoredMetadataIndex],
    path: &str,
) -> Result<&'a StoredMetadataIndex> {
    definitions
        .iter()
        .find(|definition| definition.field_path == path)
        .ok_or_else(|| anyhow!("metadata field '{path}' is not declared"))
}

pub(super) fn push_metadata_filter(
    query: &mut QueryBuilder<Postgres>,
    filter: &MetadataFilter,
    definition: &StoredMetadataIndex,
) -> Result<()> {
    query.push(" AND ");
    match filter.operator {
        MetadataFilterOperator::Exists => {
            if !filter
                .value
                .as_ref()
                .and_then(Value::as_bool)
                .unwrap_or(true)
            {
                query.push("NOT ");
            }
            query
                .push("EXISTS (SELECT 1 FROM context69.document_metadata_values v WHERE v.document_id = d.id AND v.index_id = ")
                .push_bind(definition.index_id)
                .push(")");
        }
        MetadataFilterOperator::Eq => {
            if definition.value_kind == "array" {
                push_json_path_eq(query, &filter.path, filter.value.as_ref());
            } else {
                push_typed_exists(query, definition, |query| {
                    push_typed_comparison(query, definition, filter.value.as_ref(), "=")
                })?;
            }
        }
        MetadataFilterOperator::In => {
            let Some(values) = filter.value.as_ref().and_then(Value::as_array) else {
                query.push("FALSE");
                return Ok(());
            };
            if values.is_empty() {
                query.push("FALSE");
            } else {
                push_typed_exists(query, definition, |query| {
                    query.push("(");
                    for (index, value) in values.iter().enumerate() {
                        if index > 0 {
                            query.push(" OR ");
                        }
                        push_typed_comparison(query, definition, Some(value), "=")?;
                    }
                    query.push(")");
                    Ok(())
                })?;
            }
        }
        MetadataFilterOperator::Contains => {
            if definition.value_kind != "array" {
                query.push("FALSE");
            } else {
                push_typed_exists(query, definition, |query| {
                    push_typed_comparison(query, definition, filter.value.as_ref(), "=")
                })?;
            }
        }
        MetadataFilterOperator::Range => {
            if filter.min.is_none() && filter.max.is_none() {
                query
                    .push("EXISTS (SELECT 1 FROM context69.document_metadata_values v WHERE v.document_id = d.id AND v.index_id = ")
                    .push_bind(definition.index_id)
                    .push(")");
            } else {
                push_typed_exists(query, definition, |query| {
                    let mut has_bound = false;
                    if filter.min.is_some() {
                        push_typed_comparison(query, definition, filter.min.as_ref(), ">=")?;
                        has_bound = true;
                    }
                    if filter.max.is_some() {
                        if has_bound {
                            query.push(" AND ");
                        }
                        push_typed_comparison(query, definition, filter.max.as_ref(), "<=")?;
                    }
                    Ok(())
                })?;
            }
        }
    }
    Ok(())
}

fn push_typed_exists<F>(
    query: &mut QueryBuilder<Postgres>,
    definition: &StoredMetadataIndex,
    predicate: F,
) -> Result<()>
where
    F: FnOnce(&mut QueryBuilder<Postgres>) -> Result<()>,
{
    query
        .push("EXISTS (SELECT 1 FROM context69.document_metadata_values v WHERE v.document_id = d.id AND v.index_id = ")
        .push_bind(definition.index_id)
        .push(" AND ");
    predicate(query)?;
    query.push(")");
    Ok(())
}

fn push_typed_comparison(
    query: &mut QueryBuilder<Postgres>,
    definition: &StoredMetadataIndex,
    value: Option<&Value>,
    operator: &str,
) -> Result<()> {
    let Some(value) = value.and_then(|value| typed_value(&definition.data_type, value).ok()) else {
        query.push("FALSE");
        return Ok(());
    };
    query
        .push("v.")
        .push(typed_column(&definition.data_type))
        .push(" ")
        .push(operator)
        .push(" ");
    match value {
        SqlValue::Keyword(value) => query.push_bind(value),
        SqlValue::Integer(value) => query.push_bind(value),
        SqlValue::Float(value) => query.push_bind(value),
        SqlValue::Boolean(value) => query.push_bind(value),
        SqlValue::Datetime(value) => query.push_bind(value),
    };
    Ok(())
}

fn push_json_path_eq(query: &mut QueryBuilder<Postgres>, path: &str, value: Option<&Value>) {
    let Some(value) = value else {
        query.push("FALSE");
        return;
    };
    query
        .push("d.metadata_json #> ")
        .push_bind(path.split('.').map(str::to_owned).collect::<Vec<_>>())
        .push(" = ")
        .push_bind(value);
}

pub(super) fn typed_column(data_type: &str) -> &'static str {
    match data_type {
        "keyword" => "keyword_value",
        "integer" => "integer_value",
        "float" => "float_value",
        "boolean" => "boolean_value",
        "datetime" => "datetime_value",
        _ => "keyword_value",
    }
}

enum SqlValue {
    Keyword(String),
    Integer(i64),
    Float(f64),
    Boolean(bool),
    Datetime(DateTime<Utc>),
}

fn typed_value(data_type: &str, value: &Value) -> Result<SqlValue> {
    match data_type {
        "keyword" => Ok(SqlValue::Keyword(
            value
                .as_str()
                .ok_or_else(|| anyhow!("metadata value must be a string"))?
                .to_string(),
        )),
        "integer" => {
            Ok(SqlValue::Integer(value.as_i64().ok_or_else(|| {
                anyhow!("metadata value must be an integer")
            })?))
        }
        "float" => Ok(SqlValue::Float(
            value
                .as_f64()
                .filter(|value| value.is_finite())
                .ok_or_else(|| anyhow!("metadata value must be a finite number"))?,
        )),
        "boolean" => {
            Ok(SqlValue::Boolean(value.as_bool().ok_or_else(|| {
                anyhow!("metadata value must be a boolean")
            })?))
        }
        "datetime" => Ok(SqlValue::Datetime(
            DateTime::parse_from_rfc3339(
                value
                    .as_str()
                    .ok_or_else(|| anyhow!("metadata value must be RFC 3339"))?,
            )?
            .with_timezone(&Utc),
        )),
        other => Err(anyhow!("unsupported metadata data type '{other}'")),
    }
}
