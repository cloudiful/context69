use anyhow::{Result, anyhow};
use context69_contracts::{DocumentSort, DocumentSortField, SortOrder};
use sqlx::{Postgres, QueryBuilder};

use crate::db::StoredMetadataIndex;

use super::{
    Cursor,
    query_filters::{definition_for_path, typed_column},
    sorting::SortValue,
};

pub(super) fn sort_expression(
    sort: &DocumentSort,
    index: usize,
    definitions: &[StoredMetadataIndex],
) -> Result<String> {
    Ok(match &sort.field {
        DocumentSortField::PublishedAt => "d.published_at".to_string(),
        DocumentSortField::UpdatedAt => "d.updated_at_source".to_string(),
        DocumentSortField::Metadata(path) => {
            let definition = definition_for_path(definitions, path)?;
            format!("sort_{index}.{}", typed_column(&definition.data_type))
        }
    })
}

pub(super) fn push_keyset_condition(
    query: &mut QueryBuilder<Postgres>,
    sorts: &[DocumentSort],
    definitions: &[StoredMetadataIndex],
    cursor: &Cursor,
) -> Result<()> {
    query.push(" AND (");
    for (index, sort) in sorts.iter().enumerate() {
        if index > 0 {
            query.push(" OR ");
        }
        query.push("(");
        for (previous, previous_sort) in sorts.iter().enumerate().take(index) {
            if previous > 0 {
                query.push(" AND ");
            }
            push_sort_equal(
                query,
                previous_sort,
                previous,
                definitions,
                &cursor.values[previous],
            )?;
        }
        if index > 0 {
            query.push(" AND ");
        }
        push_sort_after(query, sort, index, definitions, &cursor.values[index])?;
        query.push(")");
    }
    query.push(" OR (");
    for (index, sort) in sorts.iter().enumerate() {
        if index > 0 {
            query.push(" AND ");
        }
        push_sort_equal(query, sort, index, definitions, &cursor.values[index])?;
    }
    query
        .push(" AND d.id > ")
        .push_bind(cursor.document_id)
        .push(")");
    query.push(")");
    Ok(())
}

fn push_sort_equal(
    query: &mut QueryBuilder<Postgres>,
    sort: &DocumentSort,
    index: usize,
    definitions: &[StoredMetadataIndex],
    value: &SortValue,
) -> Result<()> {
    let expression = sort_expression(sort, index, definitions)?;
    query.push("(");
    if matches!(value, SortValue::Null) {
        query.push(&expression).push(" IS NULL");
    } else {
        query.push(&expression).push(" = ");
        push_sort_bind(query, sort, definitions, value)?;
    }
    query.push(")");
    Ok(())
}

fn push_sort_after(
    query: &mut QueryBuilder<Postgres>,
    sort: &DocumentSort,
    index: usize,
    definitions: &[StoredMetadataIndex],
    value: &SortValue,
) -> Result<()> {
    let expression = sort_expression(sort, index, definitions)?;
    if matches!(value, SortValue::Null) {
        query.push("FALSE");
        return Ok(());
    }
    query.push("(").push(&expression).push(" IS NULL OR (");
    query.push(&expression).push(" IS NOT NULL AND ");
    query.push(&expression).push(match sort.order {
        SortOrder::Asc => " > ",
        SortOrder::Desc => " < ",
    });
    push_sort_bind(query, sort, definitions, value)?;
    query.push("))");
    Ok(())
}

fn push_sort_bind(
    query: &mut QueryBuilder<Postgres>,
    sort: &DocumentSort,
    definitions: &[StoredMetadataIndex],
    value: &SortValue,
) -> Result<()> {
    let data_type = match &sort.field {
        DocumentSortField::PublishedAt | DocumentSortField::UpdatedAt => "datetime",
        DocumentSortField::Metadata(path) => &definition_for_path(definitions, path)?.data_type,
    };
    match (data_type, value) {
        ("keyword", SortValue::Keyword(value)) => query.push_bind(value.clone()),
        ("integer", SortValue::Integer(value)) => query.push_bind(*value),
        ("float", SortValue::Float(value)) => query.push_bind(*value),
        ("boolean", SortValue::Boolean(value)) => query.push_bind(*value),
        ("datetime", SortValue::Datetime(value)) => query.push_bind(*value),
        _ => return Err(anyhow!("cursor sort value type does not match query sort")),
    };
    Ok(())
}
