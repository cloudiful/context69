use chrono::{Duration, TimeZone, Utc};
use context69::domain::AccessScope;

use super::super::{SearchCase, SeededFixture, public_scope, request};
use super::case;

pub fn search_cases(fixture: &SeededFixture) -> Vec<SearchCase> {
    let tag = &fixture.tag;
    vec![
        case(
            "title phrase expands all document chunks",
            request(format!("expansion{tag} phrase{tag}"), &fixture.group_path),
            public_scope(),
            100,
            2,
            Some("title_phrase"),
        ),
        case(
            "chunk phrase preserves chunk scoring",
            request(format!("needle{tag} phrase{tag}"), &fixture.group_path),
            public_scope(),
            100,
            1,
            Some("chunk_phrase"),
        ),
        case(
            "all distinct terms may span title and chunk",
            request(
                format!("alpha{tag} beta{tag} gamma{tag}"),
                &fixture.group_path,
            ),
            public_scope(),
            100,
            1,
            Some("all_terms"),
        ),
        case(
            "Unicode whitespace splits terms without phrase normalization",
            request(
                format!("alpha{tag}\u{3000}beta{tag} gamma{tag}"),
                &fixture.group_path,
            ),
            public_scope(),
            100,
            1,
            Some("all_terms"),
        ),
        case(
            "duplicate query terms retain matching semantics",
            request(format!("repeat{tag} repeat{tag}"), &fixture.group_path),
            public_scope(),
            100,
            1,
            Some("all_terms"),
        ),
        case(
            "terms can be split across title and chunk",
            request(format!("left{tag} right{tag}"), &fixture.group_path),
            public_scope(),
            100,
            1,
            Some("all_terms"),
        ),
        case(
            "wildcard percent remains raw LIKE syntax",
            request(format!("wild{tag}%card"), &fixture.group_path),
            public_scope(),
            100,
            1,
            Some("chunk_phrase"),
        ),
        case(
            "wildcard underscore remains raw LIKE syntax",
            request(format!("under{tag}_score"), &fixture.group_path),
            public_scope(),
            100,
            1,
            Some("chunk_phrase"),
        ),
        case(
            "backslash retains PostgreSQL LIKE escape behavior",
            request(format!(r"slash{tag}\word"), &fixture.group_path),
            public_scope(),
            100,
            1,
            Some("chunk_phrase"),
        ),
        case(
            "multi-character CJK phrase",
            request("中文", &fixture.group_path),
            public_scope(),
            100,
            2,
            Some("title_phrase"),
        ),
        case(
            "one-character CJK phrase",
            request("中", &fixture.group_path),
            public_scope(),
            100,
            2,
            Some("title_phrase"),
        ),
        case(
            "punctuation-only request has no terms",
            request("!!!", &fixture.group_path),
            public_scope(),
            100,
            0,
            None,
        ),
        case(
            "empty request returns no rows",
            request(" \t ", &fixture.group_path),
            public_scope(),
            100,
            0,
            None,
        ),
        case(
            "source filter selects matching source",
            {
                let mut request = request(format!("source{tag}needle"), &fixture.group_path);
                request.source_key = Some("other-source".to_owned());
                request
            },
            public_scope(),
            100,
            1,
            Some("chunk_phrase"),
        ),
        case(
            "source filter rejects nonmatching source",
            {
                let mut request = request(format!("source{tag}needle"), &fixture.group_path);
                request.source_key = Some("fixture-source".to_owned());
                request
            },
            public_scope(),
            100,
            0,
            None,
        ),
        case(
            "group path filter remains exact",
            request(
                format!("needle{tag} phrase{tag}"),
                "issue62/non-matching/path",
            ),
            public_scope(),
            100,
            0,
            None,
        ),
        case(
            "published range includes both boundaries",
            {
                let mut request = request(format!("date{tag}needle"), &fixture.group_path);
                let start = Utc.with_ymd_and_hms(2025, 1, 1, 0, 0, 0).unwrap();
                request.published_after = Some(start);
                request.published_before = Some(start + Duration::seconds(1));
                request
            },
            public_scope(),
            100,
            2,
            Some("chunk_phrase"),
        ),
        case(
            "private group is hidden from public-only scope",
            request(format!("secret{tag}needle"), &fixture.private_group_path),
            public_scope(),
            100,
            0,
            None,
        ),
        case(
            "private group is visible to authorized scope",
            request(format!("secret{tag}needle"), &fixture.private_group_path),
            AccessScope {
                private_group_ids: vec![fixture.private_group_id],
                ..public_scope()
            },
            100,
            1,
            Some("chunk_phrase"),
        ),
    ]
}
