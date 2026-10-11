use context69::contracts::MetadataFilterOperator;
use serde_json::json;

use super::super::{SearchCase, SeededFixture, public_scope, request};
use super::{case, metadata_filter};

pub fn search_cases(fixture: &SeededFixture) -> Vec<SearchCase> {
    let mut cases = Vec::new();
    for (name, filter, count) in [
        (
            "metadata exists true",
            metadata_filter(
                "tier",
                MetadataFilterOperator::Exists,
                Some(json!(true)),
                None,
                None,
            ),
            2,
        ),
        (
            "metadata equality including missing-path SQL null",
            metadata_filter(
                "tier",
                MetadataFilterOperator::Eq,
                Some(json!("gold")),
                None,
                None,
            ),
            2,
        ),
        (
            "metadata membership",
            metadata_filter(
                "tier",
                MetadataFilterOperator::In,
                Some(json!(["gold", "silver"])),
                None,
                None,
            ),
            2,
        ),
        (
            "metadata array contains including missing-path SQL null",
            metadata_filter(
                "tags",
                MetadataFilterOperator::Contains,
                Some(json!("red")),
                None,
                None,
            ),
            3,
        ),
        (
            "metadata numeric range",
            metadata_filter(
                "score",
                MetadataFilterOperator::Range,
                None,
                Some(json!(5)),
                Some(json!(8)),
            ),
            1,
        ),
        (
            "metadata JSON null and missing values count as absent",
            metadata_filter(
                "nullable",
                MetadataFilterOperator::Exists,
                Some(json!(false)),
                None,
                None,
            ),
            3,
        ),
        (
            "metadata missing and JSON null are absent",
            metadata_filter(
                "tier",
                MetadataFilterOperator::Exists,
                Some(json!(false)),
                None,
                None,
            ),
            1,
        ),
        (
            "metadata missing equality preserves SQL null behavior",
            metadata_filter(
                "not.present",
                MetadataFilterOperator::Eq,
                Some(json!("x")),
                None,
                None,
            ),
            3,
        ),
    ] {
        let mut search_request =
            request(format!("filter{}needle", fixture.tag), &fixture.group_path);
        search_request.metadata_filters = vec![filter];
        cases.push(case(
            name,
            search_request,
            public_scope(),
            100,
            count,
            Some("chunk_phrase"),
        ));
    }
    cases
}
