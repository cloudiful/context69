use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use context69_search::SearchExclusions;
use qdrant_client::qdrant::{Condition, CountPointsBuilder, Filter, Range, SearchPointsBuilder};
use std::time::Instant;
use tracing::info;
use uuid::Uuid;

use super::errors::format_qdrant_error;
use super::payload::{
    deleted_document_condition, deleted_file_condition, hidden_point_condition,
    metadata_filter_condition,
};
use super::{QdrantIndex, SearchPointHit, date_to_timestamp, point_id_to_uuid};
use crate::contracts::SearchRequest;
use crate::domain::AccessScope;

impl QdrantIndex {
    pub async fn search(
        &self,
        vector: Vec<f32>,
        request: &SearchRequest,
        scope: &AccessScope,
        exclusions: &SearchExclusions,
    ) -> Result<Vec<SearchPointHit>> {
        let conditions = self.build_search_conditions(request, scope, exclusions)?;
        if conditions.is_short_circuit {
            return Ok(Vec::new());
        }

        let builder = SearchPointsBuilder::new(&self.collection_name, vector, request.limit as u64);
        let builder = if conditions.is_empty() {
            builder
        } else {
            builder.filter(conditions.into_filter())
        };

        let started = Instant::now();
        let operation = "search_points";
        let collection = self.collection_name.clone();
        let extra = format!("limit={}", request.limit);
        let result = self
            .client
            .search_points(builder)
            .await
            .map_err(|err| format_qdrant_error(operation, &collection, &extra, err.into()))?;
        let hits = result
            .result
            .into_iter()
            .map(|point| {
                let point_id = point.id.context("missing qdrant point id")?;
                let chunk_id = point_id_to_uuid(point_id)?;
                Ok(SearchPointHit {
                    chunk_id,
                    score: point.score,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        info!(
            candidate_count = hits.len(),
            metadata_filter_count = request.metadata_filters.len(),
            elapsed_ms = started.elapsed().as_millis() as u64,
            "qdrant search completed"
        );
        Ok(hits)
    }

    /// Build the access/locale/source/date/metadata conditions shared by the
    /// relevance search and the date-mode window. Returns
    /// `(filter_conditions, is_short_circuit)` where `is_short_circuit = true`
    /// means the request resolves to an empty result set (e.g. a scoped group
    /// path that no longer maps to a group id) and the caller should skip the
    /// Qdrant round-trip.
    ///
    /// `exclusions` carries the attempt's one registry snapshot: the point ids
    /// the lifecycle hides and the file/document entities that were explicitly
    /// deleted. The point ids become exactly one `must_not has_id` condition,
    /// and the deleted entities become one `must_not` match-any clause on the
    /// indexed `library_file_id` and `document_id` payload fields, so the index
    /// excludes them before applying the request limit. Empty input adds
    /// nothing and leaves the pre-existing filter unchanged. The lists are
    /// never truncated: every caller-supplied id is part of the condition.
    ///
    /// The shared `must` list always carries the hidden index-state clause, so
    /// points marked `staged` or `pending` stay invisible in every search
    /// mode while legacy points without the key keep their current results.
    pub(super) fn build_search_conditions(
        &self,
        request: &SearchRequest,
        scope: &AccessScope,
        exclusions: &SearchExclusions,
    ) -> Result<BuiltSearchConditions> {
        let mut filter = Vec::new();

        // Points in a hidden index state are never visible. The clause is part
        // of the shared `must` list, so relevance search, the date-mode
        // window and the date ceiling all inherit it, and it is added before
        // any early return so the short-circuit path carries it too.
        filter.push(hidden_point_condition());

        if let Some(source_key) = &request.source_key {
            filter.push(Condition::matches("source_key", source_key.clone()));
        }

        let locale_filter = match request.locale.as_deref() {
            Some(locale) => Filter::should(vec![
                Condition::matches("content_locale", locale.to_string()),
                Condition::matches("content_locale", "original".to_string()),
                Condition::is_empty("content_locale"),
            ]),
            None => Filter::should(vec![
                Condition::matches("content_locale", "original".to_string()),
                Condition::is_empty("content_locale"),
            ]),
        };
        filter.push(Condition::from(locale_filter));

        if request.published_after.is_some() || request.published_before.is_some() {
            let range = Range {
                gte: request.published_after.map(date_to_timestamp_f64),
                lte: request.published_before.map(date_to_timestamp_f64),
                ..Default::default()
            };
            filter.push(Condition::range("published_ts", range));
        }

        filter.extend(
            request
                .metadata_filters
                .iter()
                .filter_map(metadata_filter_condition),
        );

        if let Some(group_id) = scope.scoped_group_id {
            filter.push(Condition::matches("group_id", group_id));
        } else if scope.group_path.is_some() {
            // The request was scoped to a group path that no longer resolves.
            // Returning nothing is safer than silently widening the scope.
            return Ok(BuiltSearchConditions {
                filter,
                must_not: exclusion_conditions(exclusions),
                is_short_circuit: true,
            });
        }

        let access_condition = if scope.private_group_ids.is_empty() {
            Condition::matches("visibility", "public".to_string())
        } else {
            Condition::from(Filter::should(vec![
                Condition::matches("visibility", "public".to_string()),
                Condition::matches("group_id", scope.private_group_ids.clone()),
            ]))
        };
        filter.push(access_condition);

        Ok(BuiltSearchConditions {
            filter,
            must_not: exclusion_conditions(exclusions),
            is_short_circuit: false,
        })
    }

    pub async fn count_points(&self) -> Result<u64> {
        let operation = "count_points";
        let collection = self.collection_name.clone();
        let extra = String::new();
        let result = self
            .client
            .count(CountPointsBuilder::new(&self.collection_name).exact(true))
            .await
            .map_err(|err| format_qdrant_error(operation, &collection, &extra, err.into()))?;
        Ok(result.result.map(|count| count.count).unwrap_or_default())
    }
}

/// Output of `QdrantIndex::build_search_conditions`. `is_short_circuit = true`
/// means the conditions (e.g. a scoped-but-missing group) resolve to an empty
/// result set and the caller should skip the Qdrant round-trip.
pub(super) struct BuiltSearchConditions {
    pub(super) filter: Vec<Condition>,
    pub(super) must_not: Vec<Condition>,
    pub(super) is_short_circuit: bool,
}

impl BuiltSearchConditions {
    pub(super) fn is_empty(&self) -> bool {
        self.filter.is_empty() && self.must_not.is_empty()
    }

    pub(super) fn into_filter(self) -> Filter {
        Filter {
            must: self.filter,
            must_not: self.must_not,
            ..Default::default()
        }
    }
}

/// The `must_not` conditions of one attempt: the hidden point ids, the
/// deleted library files and the deleted documents.
///
/// The point ids are never truncated or sampled. An empty part adds no clause,
/// so the pre-existing filter survives unchanged.
fn exclusion_conditions(exclusions: &SearchExclusions) -> Vec<Condition> {
    let mut conditions = Vec::new();
    if !exclusions.point_ids.is_empty() {
        conditions.push(Condition::has_id(
            exclusions.point_ids.iter().map(Uuid::to_string),
        ));
    }
    if let Some(condition) = deleted_file_condition(&exclusions.file_ids) {
        conditions.push(condition);
    }
    if let Some(condition) = deleted_document_condition(&exclusions.document_ids) {
        conditions.push(condition);
    }
    conditions
}

fn date_to_timestamp_f64(date: DateTime<Utc>) -> f64 {
    i64_to_f64(date_to_timestamp(date))
}

pub(super) fn i64_to_f64(value: i64) -> f64 {
    value as f64
}

#[cfg(test)]
mod tests {
    use qdrant_client::qdrant::condition::ConditionOneOf;
    use qdrant_client::qdrant::r#match::MatchValue;
    use qdrant_client::qdrant::point_id::PointIdOptions;
    use qdrant_client::qdrant::{Condition, Filter, Match, PointId};
    use uuid::Uuid;

    use super::super::payload::{DOCUMENT_ID_FIELD, LIBRARY_FILE_ID_FIELD, POINT_STATE_KEY};
    use super::QdrantIndex;
    use crate::contracts::SearchRequest;
    use crate::domain::AccessScope;
    use context69_search::SearchExclusions;

    fn point_exclusions(point_ids: Vec<Uuid>) -> SearchExclusions {
        SearchExclusions {
            point_ids,
            ..SearchExclusions::default()
        }
    }

    /// The exclusion set of a snapshot that hides deleted entities only.
    fn entity_exclusions(file_ids: Vec<Uuid>, document_ids: Vec<i64>) -> SearchExclusions {
        SearchExclusions {
            point_ids: Vec::new(),
            file_ids,
            document_ids,
        }
    }

    /// The keyword matches of a condition, whatever their match shape.
    fn matched_keywords(condition: &Condition) -> Option<(String, Vec<String>)> {
        let ConditionOneOf::Field(field) = condition.condition_one_of.as_ref()? else {
            return None;
        };
        let Some(Match {
            match_value: Some(MatchValue::Keywords(values)),
        }) = field.r#match.as_ref()
        else {
            return None;
        };
        Some((field.key.clone(), values.strings.clone()))
    }

    /// The integer matches of a condition, whatever their match shape.
    fn matched_integers(condition: &Condition) -> Option<(String, Vec<i64>)> {
        let ConditionOneOf::Field(field) = condition.condition_one_of.as_ref()? else {
            return None;
        };
        let Some(Match {
            match_value: Some(MatchValue::Integers(values)),
        }) = field.r#match.as_ref()
        else {
            return None;
        };
        Some((field.key.clone(), values.integers.clone()))
    }

    fn test_request() -> SearchRequest {
        SearchRequest {
            query: "query".to_string(),
            locale: None,
            limit: 10,
            page: 1,
            source_key: None,
            group_path: None,
            published_after: None,
            published_before: None,
            cursor: None,
            metadata_filters: Vec::new(),
            sort: crate::contracts::SearchSort::Relevance,
        }
    }

    fn public_scope() -> AccessScope {
        AccessScope {
            user_id: None,
            include_public: true,
            private_group_ids: Vec::new(),
            group_path: None,
            scoped_group_id: None,
        }
    }

    fn condition_ids(condition: &Condition) -> Vec<String> {
        let ConditionOneOf::HasId(has_id) = condition
            .condition_one_of
            .as_ref()
            .expect("condition payload")
        else {
            panic!("expected a has_id condition");
        };
        has_id
            .has_id
            .iter()
            .map(
                |point_id: &PointId| match point_id.point_id_options.as_ref() {
                    Some(PointIdOptions::Uuid(value)) => value.clone(),
                    other => panic!("expected a uuid point id, got {other:?}"),
                },
            )
            .collect()
    }

    /// The keyword match of a condition, or `None` for every other condition
    /// shape (empty, null, id, nested filter, ...).
    fn matched_keyword(condition: &Condition) -> Option<(String, String)> {
        let ConditionOneOf::Field(field) = condition.condition_one_of.as_ref()? else {
            return None;
        };
        let Some(Match {
            match_value: Some(MatchValue::Keyword(value)),
        }) = field.r#match.as_ref()
        else {
            return None;
        };
        Some((field.key.clone(), value.clone()))
    }

    /// The hidden index-state clause: the nested filter whose `must_not` group
    /// carries the state matches.
    fn hidden_state_clause(filter: &[Condition]) -> Option<Filter> {
        filter.iter().find_map(|condition| {
            let ConditionOneOf::Filter(nested) = condition.condition_one_of.as_ref()? else {
                return None;
            };
            (nested.must.is_empty() && nested.should.is_empty() && !nested.must_not.is_empty())
                .then(|| nested.clone())
        })
    }

    #[test]
    fn empty_exclusions_leave_the_condition_builder_unchanged() {
        let index = QdrantIndex::for_test("test", 8);
        let conditions = index
            .build_search_conditions(&test_request(), &public_scope(), &SearchExclusions::empty())
            .expect("conditions");

        assert!(!conditions.is_short_circuit);
        assert!(
            conditions.must_not.is_empty(),
            "an empty exclusion set must add no filter condition"
        );
        assert!(
            !conditions.filter.is_empty(),
            "the pre-existing access/locale conditions must remain"
        );
    }

    #[test]
    fn populated_exclusions_become_one_must_not_has_id_condition() {
        let index = QdrantIndex::for_test("test", 8);
        let staged = Uuid::from_u128(0x3001);
        let retired = Uuid::from_u128(0x3002);
        let conditions = index
            .build_search_conditions(
                &test_request(),
                &public_scope(),
                &point_exclusions(vec![staged, retired]),
            )
            .expect("conditions");

        assert_eq!(conditions.must_not.len(), 1);
        assert_eq!(
            condition_ids(&conditions.must_not[0]),
            vec![staged.to_string(), retired.to_string()]
        );
        assert!(
            conditions.filter.iter().all(|condition| !matches!(
                condition.condition_one_of.as_ref(),
                Some(ConditionOneOf::HasId(_))
            )),
            "the exclusion must be a must_not condition, never a must condition"
        );
    }

    #[test]
    fn exclusions_are_never_truncated() {
        let index = QdrantIndex::for_test("test", 8);
        let ids: Vec<Uuid> = (1..=150).map(Uuid::from_u128).collect();
        let conditions = index
            .build_search_conditions(
                &test_request(),
                &public_scope(),
                &point_exclusions(ids.clone()),
            )
            .expect("conditions");

        assert_eq!(
            condition_ids(&conditions.must_not[0]).len(),
            ids.len(),
            "every excluded id must reach the index-side filter"
        );
    }

    #[test]
    fn hidden_state_clause_excludes_exactly_staged_and_pending() {
        let index = QdrantIndex::for_test("test", 8);
        let conditions = index
            .build_search_conditions(&test_request(), &public_scope(), &SearchExclusions::empty())
            .expect("conditions");

        let clause = hidden_state_clause(&conditions.filter).expect("hidden state clause");
        let mut matched = clause
            .must_not
            .iter()
            .map(|condition| {
                matched_keyword(condition).expect("the clause must only use keyword matches")
            })
            .collect::<Vec<_>>();
        matched.sort();

        assert_eq!(
            matched,
            vec![
                (POINT_STATE_KEY.to_string(), "pending".to_string()),
                (POINT_STATE_KEY.to_string(), "staged".to_string()),
            ],
            "only the exact hidden states may be excluded"
        );
    }

    #[test]
    fn hidden_state_clause_keeps_legacy_and_live_points_visible() {
        let index = QdrantIndex::for_test("test", 8);
        let conditions = index
            .build_search_conditions(&test_request(), &public_scope(), &SearchExclusions::empty())
            .expect("conditions");

        let clause = hidden_state_clause(&conditions.filter).expect("hidden state clause");
        assert!(
            clause.must_not.iter().all(|condition| matches!(
                condition.condition_one_of.as_ref(),
                Some(ConditionOneOf::Field(_))
            )),
            "the clause must use match conditions only: is_empty would also exclude legacy points and is_null only null points"
        );

        let values = clause
            .must_not
            .iter()
            .filter_map(matched_keyword)
            .map(|(_, value)| value)
            .collect::<Vec<_>>();
        assert!(
            !values.contains(&"live".to_string()),
            "a live point must stay visible"
        );

        assert!(
            !conditions.filter.iter().any(|condition| matches!(
                condition.condition_one_of.as_ref(),
                Some(ConditionOneOf::IsEmpty(condition)) if condition.key == POINT_STATE_KEY
            )),
            "the visibility rule must never be an is_empty test on the state key"
        );
        assert!(
            !conditions.filter.iter().any(|condition| matches!(
                condition.condition_one_of.as_ref(),
                Some(ConditionOneOf::Filter(nested))
                    if nested.must_not.iter().any(|condition| matches!(
                        condition.condition_one_of.as_ref(),
                        Some(ConditionOneOf::IsEmpty(condition)) if condition.key == POINT_STATE_KEY
                    ))
            )),
            "no nested clause may exclude the state key with is_empty"
        );
    }

    #[test]
    fn hidden_state_clause_coexists_with_acl_and_registry_exclusions() {
        let index = QdrantIndex::for_test("test", 8);
        let staged = Uuid::from_u128(0x3001);
        let scope = AccessScope {
            user_id: None,
            include_public: true,
            private_group_ids: vec![4001i64],
            group_path: None,
            scoped_group_id: None,
        };
        let conditions = index
            .build_search_conditions(&test_request(), &scope, &point_exclusions(vec![staged]))
            .expect("conditions");

        assert!(
            hidden_state_clause(&conditions.filter).is_some(),
            "the visibility clause must stay in the shared must list"
        );
        assert_eq!(
            conditions.must_not.len(),
            1,
            "the registry exclusion must remain the only top-level must_not condition"
        );
        assert_eq!(
            condition_ids(&conditions.must_not[0]),
            vec![staged.to_string()],
            "the registry exclusion must survive unchanged"
        );
        assert!(
            conditions.filter.iter().any(|condition| matches!(
                condition.condition_one_of.as_ref(),
                Some(ConditionOneOf::Filter(nested)) if !nested.should.is_empty()
            )),
            "the pre-existing locale/access conditions must remain"
        );
    }

    /// The date-mode window composes the shared `must` list and the date
    /// ceiling composes the whole filter, so both inherit the clause.
    #[test]
    fn date_window_and_ceiling_filters_carry_the_hidden_state_clause() {
        let index = QdrantIndex::for_test("test", 8);
        let staged = Uuid::from_u128(0x3001);

        let window_conditions = index
            .build_search_conditions(
                &test_request(),
                &public_scope(),
                &point_exclusions(vec![staged]),
            )
            .expect("conditions");
        assert!(
            hidden_state_clause(&window_conditions.filter).is_some(),
            "the date window consumes the shared must list and must carry the clause"
        );

        let ceiling_filter = index
            .build_search_conditions(
                &test_request(),
                &public_scope(),
                &point_exclusions(vec![staged]),
            )
            .expect("conditions")
            .into_filter();
        assert!(
            hidden_state_clause(&ceiling_filter.must).is_some(),
            "the date ceiling consumes the composed filter and must carry the clause"
        );
        assert_eq!(
            ceiling_filter.must_not.len(),
            1,
            "the registry exclusion must survive the composed filter"
        );
    }

    /// A deleted file is excluded by the indexed `library_file_id` field, so
    /// the whole file leaves the candidate set without one registry
    /// reservation per point.
    #[test]
    fn a_deleted_file_becomes_an_indexed_match_any_clause() {
        let index = QdrantIndex::for_test("test", 8);
        let first = Uuid::from_u128(0x4001);
        let second = Uuid::from_u128(0x4002);
        let conditions = index
            .build_search_conditions(
                &test_request(),
                &public_scope(),
                &entity_exclusions(vec![first, second], Vec::new()),
            )
            .expect("conditions");

        assert_eq!(conditions.must_not.len(), 1);
        let (key, values) = matched_keywords(&conditions.must_not[0])
            .expect("the deleted file clause must be a keyword match");
        assert_eq!(key, LIBRARY_FILE_ID_FIELD);
        assert_eq!(
            values,
            vec![first.to_string(), second.to_string()],
            "every deleted file id must reach the index-side filter"
        );
    }

    /// A deleted document is excluded by the indexed `document_id` field.
    #[test]
    fn a_deleted_document_becomes_an_indexed_match_any_clause() {
        let index = QdrantIndex::for_test("test", 8);
        let conditions = index
            .build_search_conditions(
                &test_request(),
                &public_scope(),
                &entity_exclusions(Vec::new(), vec![11, 12]),
            )
            .expect("conditions");

        assert_eq!(conditions.must_not.len(), 1);
        let (key, values) = matched_integers(&conditions.must_not[0])
            .expect("the deleted document clause must be an integer match");
        assert_eq!(key, DOCUMENT_ID_FIELD);
        assert_eq!(values, vec![11, 12]);
    }

    /// The point, file and document exclusions coexist with each other and
    /// with the pre-existing access, locale and hidden-state clauses.
    #[test]
    fn the_entity_exclusions_compose_with_the_point_exclusion() {
        let index = QdrantIndex::for_test("test", 8);
        let staged = Uuid::from_u128(0x3001);
        let file = Uuid::from_u128(0x5001);
        let conditions = index
            .build_search_conditions(
                &test_request(),
                &public_scope(),
                &SearchExclusions {
                    point_ids: vec![staged],
                    file_ids: vec![file],
                    document_ids: vec![7],
                },
            )
            .expect("conditions");

        assert_eq!(
            conditions.must_not.len(),
            3,
            "the point, file and document exclusions must all reach the filter"
        );
        assert_eq!(
            condition_ids(&conditions.must_not[0]),
            vec![staged.to_string()],
            "the point exclusion stays the has_id clause"
        );
        let (file_key, file_values) = matched_keywords(&conditions.must_not[1])
            .expect("the deleted file clause must be a keyword match");
        assert_eq!(file_key, LIBRARY_FILE_ID_FIELD);
        assert_eq!(file_values, vec![file.to_string()]);
        let (document_key, document_values) = matched_integers(&conditions.must_not[2])
            .expect("the deleted document clause must be an integer match");
        assert_eq!(document_key, DOCUMENT_ID_FIELD);
        assert_eq!(document_values, vec![7]);
        assert!(
            hidden_state_clause(&conditions.filter).is_some(),
            "the hidden state clause must survive the entity exclusions"
        );
    }

    /// The date-mode window and the date ceiling consume the same composed
    /// filter, so both exclude the deleted entities.
    #[test]
    fn the_date_paths_carry_the_entity_exclusions() {
        let index = QdrantIndex::for_test("test", 8);
        let file = Uuid::from_u128(0x6001);
        let filter = index
            .build_search_conditions(
                &test_request(),
                &public_scope(),
                &entity_exclusions(vec![file], vec![3]),
            )
            .expect("conditions")
            .into_filter();

        assert_eq!(filter.must_not.len(), 2);
        let (file_key, _) = matched_keywords(&filter.must_not[0])
            .expect("the deleted file clause must be a keyword match");
        assert_eq!(file_key, LIBRARY_FILE_ID_FIELD);
        let (document_key, _) = matched_integers(&filter.must_not[1])
            .expect("the deleted document clause must be an integer match");
        assert_eq!(document_key, DOCUMENT_ID_FIELD);
    }
}
