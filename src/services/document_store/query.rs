use anyhow::{Result, anyhow};
use chrono::{DateTime, Utc};
use context69_contracts::{DocumentQueryRequest, DocumentSortField, SortOrder};
use sqlx::{Postgres, QueryBuilder, Row, postgres::PgRow};

use crate::{
    db::{Database, StoredMetadataIndex},
    domain::AccessScope,
};

use super::{
    Cursor, filters_match,
    query_cursor::{push_keyset_condition, sort_expression},
    query_filters::{definition_for_path, push_metadata_filter},
    sorting::SortValue,
};

struct QueryCandidate {
    document_id: i64,
    sort_values: Vec<SortValue>,
}

pub(super) struct HydratedPage {
    pub(super) rows: Vec<(context69_contracts::DocumentResponse, Vec<SortValue>)>,
    pub(super) has_more: bool,
    pub(super) candidate_count: usize,
    pub(super) hydrated_count: usize,
    pub(super) metadata_dropped: usize,
}

/// Hydrate enough ordered candidates to produce a complete page after the final JSON filter.
/// The SQL filter is authoritative for normal operation; the loop protects against a briefly
/// stale typed index or a value representation that cannot be reproduced in the index table.
pub(super) async fn load_page(
    db: &Database,
    group_id: i64,
    request: &DocumentQueryRequest,
    definitions: &[StoredMetadataIndex],
    scope: &AccessScope,
    cursor: Option<&Cursor>,
    query_hash: &str,
) -> Result<HydratedPage> {
    let mut candidate_cursor = cursor.cloned();
    let mut rows = Vec::with_capacity(request.limit + 1);
    let mut candidate_count = 0;
    let mut hydrated_count = 0;
    let mut metadata_dropped = 0;

    loop {
        let candidates = list_page_candidates(
            db,
            group_id,
            request,
            definitions,
            scope,
            candidate_cursor.as_ref(),
        )
        .await?;
        if candidates.is_empty() {
            break;
        }
        candidate_count += candidates.len();
        let ids = candidates
            .iter()
            .map(|candidate| candidate.document_id)
            .collect::<Vec<_>>();

        let documents = db
            .get_documents_localized(&ids, request.locale.as_deref(), scope)
            .await?;
        hydrated_count += documents.len();

        let last_candidate = candidates.last().expect("non-empty candidate page");
        let last_candidate_id = last_candidate.document_id;
        let last_candidate_values = last_candidate.sort_values.clone();
        let page_has_more_candidates = candidates.len() == request.limit + 1;

        for candidate in candidates {
            let Some(document) = documents.get(&candidate.document_id).cloned() else {
                continue;
            };
            if filters_match(&document.metadata_json, request, definitions).unwrap_or(false) {
                rows.push((document, candidate.sort_values));
            } else {
                metadata_dropped += 1;
            }
        }

        if rows.len() > request.limit {
            break;
        }
        if !page_has_more_candidates {
            break;
        }

        candidate_cursor = Some(Cursor {
            version: 2,
            query_hash: query_hash.to_string(),
            values: last_candidate_values,
            document_id: last_candidate_id,
        });
    }

    let has_more = rows.len() > request.limit;
    Ok(HydratedPage {
        rows,
        has_more,
        candidate_count,
        hydrated_count,
        metadata_dropped,
    })
}

/// The query is dynamic because filters and sort columns are request-selected. User values are
/// always bound; SQL identifiers are limited to validated, server-owned column names.
async fn list_page_candidates(
    db: &Database,
    group_id: i64,
    request: &DocumentQueryRequest,
    definitions: &[StoredMetadataIndex],
    scope: &AccessScope,
    cursor: Option<&Cursor>,
) -> Result<Vec<QueryCandidate>> {
    let mut query = QueryBuilder::<Postgres>::new("SELECT d.id AS document_id");

    for (index, sort) in request.sort.iter().enumerate() {
        let expression = sort_expression(sort, index, definitions)?;
        query
            .push(", ")
            .push(&expression)
            .push(" AS sort_")
            .push(index.to_string());
    }
    query.push(" FROM context69.documents d INNER JOIN context69.groups g ON g.id = d.group_id");

    for (index, sort) in request.sort.iter().enumerate() {
        if let DocumentSortField::Metadata { path } = &sort.field {
            let definition = definition_for_path(definitions, path)?;
            query
                .push(" LEFT JOIN context69.document_metadata_values sort_")
                .push(index.to_string())
                .push(" ON sort_")
                .push(index.to_string())
                .push(".document_id = d.id AND sort_")
                .push(index.to_string())
                .push(".index_id = ")
                .push_bind(definition.index_id)
                .push(" AND sort_")
                .push(index.to_string())
                .push(".ordinal = 0");
        }
    }

    query
        .push(" WHERE d.group_id = ")
        .push_bind(group_id)
        .push(" AND (g.visibility = 'public' OR d.group_id = ANY(")
        .push_bind(&scope.private_group_ids)
        .push("))");
    if let Some(group_path) = scope.group_path.as_deref() {
        query.push(" AND g.full_path = ").push_bind(group_path);
    }
    if let Some(source_key) = request.source_key.as_deref() {
        query.push(" AND d.source_key = ").push_bind(source_key);
    }
    if let Some(published_after) = request.published_after {
        query
            .push(" AND d.published_at >= ")
            .push_bind(published_after);
    }
    if let Some(published_before) = request.published_before {
        query
            .push(" AND d.published_at <= ")
            .push_bind(published_before);
    }
    for filter in &request.metadata_filters {
        let definition = definition_for_path(definitions, &filter.path)?;
        push_metadata_filter(&mut query, filter, definition)?;
    }

    if let Some(cursor) = cursor {
        if cursor.values.len() != request.sort.len() {
            return Err(anyhow!("cursor sort values do not match query sort"));
        }
        if request.sort.is_empty() {
            query.push(" AND d.id > ").push_bind(cursor.document_id);
        } else {
            push_keyset_condition(&mut query, &request.sort, definitions, cursor)?;
        }
    }

    query.push(" ORDER BY ");
    if request.sort.is_empty() {
        query.push("d.id ASC");
    } else {
        for (index, sort) in request.sort.iter().enumerate() {
            if index > 0 {
                query.push(", ");
            }
            let expression = sort_expression(sort, index, definitions)?;
            query
                .push(&expression)
                .push(" IS NULL ASC, ")
                .push(&expression)
                .push(match sort.order {
                    SortOrder::Asc => " ASC",
                    SortOrder::Desc => " DESC",
                });
        }
        query.push(", d.id ASC");
    }
    query
        .push(" LIMIT ")
        .push_bind(i64::try_from(request.limit + 1).map_err(|_| anyhow!("limit is too large"))?);

    let rows = query.build().fetch_all(db.pool()).await?;
    rows.into_iter()
        .map(|row| {
            let document_id = row.try_get("document_id")?;
            let sort_values = request
                .sort
                .iter()
                .enumerate()
                .map(|(index, sort)| sort_value_from_row(&row, sort, index, definitions))
                .collect::<Result<Vec<_>>>()?;
            Ok(QueryCandidate {
                document_id,
                sort_values,
            })
        })
        .collect()
}

fn sort_value_from_row(
    row: &PgRow,
    sort: &context69_contracts::DocumentSort,
    index: usize,
    definitions: &[StoredMetadataIndex],
) -> Result<SortValue> {
    let alias = format!("sort_{index}");
    match &sort.field {
        DocumentSortField::PublishedAt | DocumentSortField::UpdatedAt => row
            .try_get::<Option<DateTime<Utc>>, _>(alias.as_str())
            .map(|value| value.map(SortValue::Datetime).unwrap_or(SortValue::Null))
            .map_err(Into::into),
        DocumentSortField::Metadata { path } => {
            let definition = definition_for_path(definitions, path)?;
            match definition.data_type.as_str() {
                "keyword" => row
                    .try_get::<Option<String>, _>(alias.as_str())
                    .map(|value| value.map(SortValue::Keyword).unwrap_or(SortValue::Null))
                    .map_err(Into::into),
                "integer" => row
                    .try_get::<Option<i64>, _>(alias.as_str())
                    .map(|value| value.map(SortValue::Integer).unwrap_or(SortValue::Null))
                    .map_err(Into::into),
                "float" => row
                    .try_get::<Option<f64>, _>(alias.as_str())
                    .map(|value| value.map(SortValue::Float).unwrap_or(SortValue::Null))
                    .map_err(Into::into),
                "boolean" => row
                    .try_get::<Option<bool>, _>(alias.as_str())
                    .map(|value| value.map(SortValue::Boolean).unwrap_or(SortValue::Null))
                    .map_err(Into::into),
                "datetime" => row
                    .try_get::<Option<DateTime<Utc>>, _>(alias.as_str())
                    .map(|value| value.map(SortValue::Datetime).unwrap_or(SortValue::Null))
                    .map_err(Into::into),
                other => Err(anyhow!("unsupported metadata data type '{other}'")),
            }
        }
    }
}
