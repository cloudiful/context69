#[path = "cases_metadata.rs"]
mod metadata;
#[path = "cases_text.rs"]
mod text;
#[path = "cases_translation.rs"]
mod translation;

use context69::contracts::{MetadataFilter, MetadataFilterOperator, SearchRequest};
use context69::domain::AccessScope;
use serde_json::Value;

use super::{SearchCase, SeededFixture};

pub fn search_cases(fixture: &SeededFixture) -> Vec<SearchCase> {
    let mut cases = text::search_cases(fixture);
    cases.extend(metadata::search_cases(fixture));
    cases.extend(translation::search_cases(fixture));
    cases
}

pub(super) fn metadata_filter(
    path: &str,
    operator: MetadataFilterOperator,
    value: Option<Value>,
    min: Option<Value>,
    max: Option<Value>,
) -> MetadataFilter {
    MetadataFilter {
        path: path.to_owned(),
        operator,
        value,
        min,
        max,
    }
}

pub(super) fn case(
    name: &'static str,
    request: SearchRequest,
    scope: AccessScope,
    limit: usize,
    expected_count: usize,
    expected_reason: Option<&'static str>,
) -> SearchCase {
    SearchCase {
        name,
        request,
        scope,
        limit,
        expected_count,
        expected_reason,
    }
}
