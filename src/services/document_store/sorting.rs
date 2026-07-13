use std::cmp::Ordering;

use anyhow::{Result, anyhow};
use chrono::{DateTime, Utc};
use context69_contracts::{DocumentResponse, DocumentSort, DocumentSortField, SortOrder};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::db::StoredMetadataIndex;

use super::metadata;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub(super) enum SortValue {
    Null,
    Keyword(String),
    Integer(i64),
    Float(f64),
    Boolean(bool),
    Datetime(DateTime<Utc>),
}

pub(super) fn values_for_document(
    document: &DocumentResponse,
    sort: &[DocumentSort],
    definitions: &[StoredMetadataIndex],
) -> Result<Vec<SortValue>> {
    sort.iter()
        .map(|item| match &item.field {
            DocumentSortField::PublishedAt => Ok(document
                .published_at
                .map(SortValue::Datetime)
                .unwrap_or(SortValue::Null)),
            DocumentSortField::UpdatedAt => Ok(SortValue::Datetime(document.updated_at)),
            DocumentSortField::Metadata(path) => {
                let definition = definitions
                    .iter()
                    .find(|definition| definition.field_path == *path)
                    .ok_or_else(|| anyhow!("metadata field '{path}' is not declared"))?;
                parse_metadata_value(
                    metadata::resolve_path(&document.metadata_json, path),
                    &definition.data_type,
                    path,
                )
            }
        })
        .collect()
}

fn parse_metadata_value(value: Option<&Value>, data_type: &str, path: &str) -> Result<SortValue> {
    let Some(value) = value.filter(|value| !value.is_null()) else {
        return Ok(SortValue::Null);
    };
    let invalid = || {
        anyhow!("metadata field '{path}' contains a value inconsistent with its {data_type} index")
    };
    match data_type {
        "keyword" => value
            .as_str()
            .map(|value| SortValue::Keyword(value.to_owned()))
            .ok_or_else(invalid),
        "integer" => value.as_i64().map(SortValue::Integer).ok_or_else(invalid),
        "float" => value
            .as_f64()
            .filter(|value| value.is_finite())
            .map(SortValue::Float)
            .ok_or_else(invalid),
        "boolean" => value.as_bool().map(SortValue::Boolean).ok_or_else(invalid),
        "datetime" => value
            .as_str()
            .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
            .map(|value| SortValue::Datetime(value.with_timezone(&Utc)))
            .ok_or_else(invalid),
        _ => Err(anyhow!("unsupported metadata data type '{data_type}'")),
    }
}

pub(super) fn compare_rows(
    left_values: &[SortValue],
    left_id: i64,
    right_values: &[SortValue],
    right_id: i64,
    sort: &[DocumentSort],
) -> Ordering {
    for ((left, right), item) in left_values.iter().zip(right_values).zip(sort) {
        let order = compare_value(left, right, item.order);
        if !order.is_eq() {
            return order;
        }
    }
    left_id.cmp(&right_id)
}

fn compare_value(left: &SortValue, right: &SortValue, direction: SortOrder) -> Ordering {
    match (left, right) {
        (SortValue::Null, SortValue::Null) => Ordering::Equal,
        (SortValue::Null, _) => Ordering::Greater,
        (_, SortValue::Null) => Ordering::Less,
        _ => {
            let order = match (left, right) {
                (SortValue::Keyword(left), SortValue::Keyword(right)) => left.cmp(right),
                (SortValue::Integer(left), SortValue::Integer(right)) => left.cmp(right),
                (SortValue::Float(left), SortValue::Float(right)) => left.total_cmp(right),
                (SortValue::Boolean(left), SortValue::Boolean(right)) => left.cmp(right),
                (SortValue::Datetime(left), SortValue::Datetime(right)) => left.cmp(right),
                _ => unreachable!("sort values are built from one declared type"),
            };
            if direction == SortOrder::Desc {
                order.reverse()
            } else {
                order
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_and_nulls_use_typed_stable_order() {
        assert_eq!(
            compare_value(
                &SortValue::Integer(2),
                &SortValue::Integer(10),
                SortOrder::Asc
            ),
            Ordering::Less
        );
        assert_eq!(
            compare_value(&SortValue::Null, &SortValue::Integer(10), SortOrder::Desc),
            Ordering::Greater
        );
        assert_eq!(
            compare_value(
                &SortValue::Integer(2),
                &SortValue::Integer(10),
                SortOrder::Desc
            ),
            Ordering::Greater
        );
        assert_eq!(
            compare_value(
                &SortValue::Float(2.5),
                &SortValue::Float(10.0),
                SortOrder::Asc
            ),
            Ordering::Less
        );
        assert_eq!(
            compare_value(
                &SortValue::Boolean(false),
                &SortValue::Boolean(true),
                SortOrder::Asc
            ),
            Ordering::Less
        );
        assert_eq!(
            compare_value(
                &SortValue::Keyword("10".into()),
                &SortValue::Keyword("2".into()),
                SortOrder::Asc
            ),
            Ordering::Less
        );
        let earlier = "2026-07-12T09:30:00Z".parse().expect("datetime");
        let later = "2026-07-12T10:30:00+00:00".parse().expect("datetime");
        assert_eq!(
            compare_value(
                &SortValue::Datetime(earlier),
                &SortValue::Datetime(later),
                SortOrder::Asc
            ),
            Ordering::Less
        );
    }

    #[test]
    fn mixed_directions_then_document_id_produce_stable_order() {
        let sort = [
            DocumentSort {
                field: DocumentSortField::Metadata("score".to_string()),
                order: SortOrder::Desc,
            },
            DocumentSort {
                field: DocumentSortField::Metadata("name".to_string()),
                order: SortOrder::Asc,
            },
        ];
        let left = [SortValue::Integer(10), SortValue::Keyword("alpha".into())];
        let right = [SortValue::Integer(10), SortValue::Keyword("beta".into())];

        assert_eq!(compare_rows(&left, 9, &right, 1, &sort), Ordering::Less);
        assert_eq!(compare_rows(&left, 1, &left, 9, &sort), Ordering::Less);
        assert_eq!(
            compare_rows(
                &[SortValue::Null, SortValue::Keyword("alpha".into())],
                1,
                &right,
                2,
                &sort,
            ),
            Ordering::Greater
        );
    }
}
