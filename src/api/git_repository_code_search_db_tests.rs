//! Database-gated round trips for the bounded lexical code search (issue #681
//! phase 5D).
//!
//! The group, repository, and activated generation come from the shared Git
//! fixture through this phase's thin wrapper, so the suite adds no second
//! repository fixture: it supplies the chunk rows a search needs and then reads
//! them back through the same group-scoped lexical accessor the route calls.
//!
//! Every test skips unless `CONTEXT69_TEST_DATABASE_URL` names a migrated
//! scratch database and then removes the groups the fixture created. Only
//! synthetic paths, counts, statuses, and scores are reported: no stored text,
//! provider blob id, or secret value is printed or asserted.

use crate::api::git_repository_files_db_fixture::Fixture;
use crate::contracts::sources::GitCodeMatchKind;
use crate::db::StoredGitRepositorySource;

use super::super::git_repository_code_search::{SearchFilters, search_response};
use super::super::git_repository_files::serving_generation;
use super::fixture::{index_with_code, language_of, lexical, query};

/// The term every stored file of the fixture carries.
const TERM: &str = "alpha marker";

#[tokio::test]
async fn a_ready_generation_searches_and_reports_its_provenance() {
    let Some(fixture) = Fixture::setup().await else {
        return;
    };
    let generation_key = index_with_code(&fixture).await;
    let source: StoredGitRepositorySource = fixture.source().await;
    let generation = serving_generation(&fixture.db, &source)
        .await
        .expect("resolve the serving generation")
        .expect("the repository serves an active generation");
    assert_eq!(generation.generation_key, generation_key);

    let filters = SearchFilters::validated(&query(TERM, 20)).expect("a bounded search");
    let hits = lexical(
        &fixture,
        fixture.group_id,
        vec![fixture.group_id],
        &filters.query,
        filters.path_prefix.as_deref(),
        filters.language.as_deref(),
        filters.fetch_limit(),
    )
    .await;
    assert_eq!(hits.len(), 3, "all three stored files carry the term");
    let response = search_response(&source, &generation, hits, filters.limit);
    assert_eq!(response.generation_key, generation_key);
    assert_eq!(response.generation_number, generation.generation_number);
    assert_eq!(response.commit_sha, generation.commit_sha);
    assert_eq!(response.file_count, generation.file_count);
    assert!(!response.truncated, "three hits under a limit of 20");
    for hit in &response.hits {
        assert_eq!(hit.generation_key, generation_key);
        assert_eq!(hit.commit_sha, generation.commit_sha);
        assert!(!hit.text.is_empty(), "a hit carries its stored text");
        assert!(hit.start_line >= 1 && hit.end_line >= hit.start_line);
    }

    fixture.cleanup(&[fixture.group_id]).await;
}

#[tokio::test]
async fn the_path_prefix_and_language_filters_narrow_the_same_search() {
    let Some(fixture) = Fixture::setup().await else {
        return;
    };
    index_with_code(&fixture).await;
    let own = vec![fixture.group_id];

    // Every stored file carries the term, so each filter below visibly narrows
    // the same three matches instead of changing which term is searched.
    let unfiltered = lexical(
        &fixture,
        fixture.group_id,
        own.clone(),
        TERM,
        None,
        None,
        10,
    )
    .await;
    assert_eq!(unfiltered.len(), 3, "the unfiltered search is the control");

    // The path prefix is matched case-sensitively, the way Git paths are.
    let scoped = lexical(
        &fixture,
        fixture.group_id,
        own.clone(),
        TERM,
        Some("src/"),
        None,
        10,
    )
    .await;
    assert_eq!(scoped.len(), 1, "one stored file lives under src/");
    assert_eq!(scoped[0].path, "src/alpha.rs");
    let wrong_case = lexical(
        &fixture,
        fixture.group_id,
        own.clone(),
        TERM,
        Some("SRC/"),
        None,
        10,
    )
    .await;
    assert!(wrong_case.is_empty(), "a Git path prefix is case-sensitive");
    let nested = lexical(
        &fixture,
        fixture.group_id,
        own.clone(),
        TERM,
        Some("docs/"),
        None,
        10,
    )
    .await;
    assert_eq!(nested.len(), 1, "one stored file lives under docs/");
    assert_eq!(nested[0].path, "docs/guide.md");

    // The language filter uses the classified token the manifest stored.
    let rust = lexical(
        &fixture,
        fixture.group_id,
        own.clone(),
        TERM,
        None,
        Some(language_of("src/alpha.rs")),
        10,
    )
    .await;
    assert_eq!(rust.len(), 1, "one stored file is classified as rust");
    assert_eq!(rust[0].language, "rust");
    let markdown = lexical(
        &fixture,
        fixture.group_id,
        own,
        TERM,
        None,
        Some(language_of("README.md")),
        10,
    )
    .await;
    assert_eq!(markdown.len(), 2, "two stored files are markdown");
    assert!(markdown.iter().all(|hit| hit.language == "markdown"));

    fixture.cleanup(&[fixture.group_id]).await;
}

#[tokio::test]
async fn a_literal_wildcard_term_matches_only_the_literal_text() {
    let Some(fixture) = Fixture::setup().await else {
        return;
    };
    index_with_code(&fixture).await;
    let own = vec![fixture.group_id];

    // A term made only of characters that cannot appear in this fixture's stored
    // text or paths can match nothing but itself: it has no identifier token to
    // fall back on, and the escaped phrase must find nothing instead of every
    // row. Each value here is absent from every stored path and chunk, `#`
    // deliberately so, because `README.md` really does start with `# manifest`
    // and a term that occurs in the corpus is a legitimate literal match. A term
    // that also carries identifier characters (`a%`, `%alpha`) legitimately
    // matches through the identifier-aware token fallback — `a` is in the stored
    // text — which is the accessor's documented behaviour, not a wildcard leak.
    for wildcard in ["%", "_", "*", "\\"] {
        let hits = lexical(
            &fixture,
            fixture.group_id,
            own.clone(),
            wildcard,
            None,
            None,
            10,
        )
        .await;
        assert!(hits.is_empty(), "{wildcard:?} must stay a literal term");
    }
    // The counterpart: the one metacharacter this fixture does contain matches
    // exactly the file that holds it, and no other.
    let present = lexical(&fixture, fixture.group_id, own.clone(), "#", None, None, 10).await;
    assert_eq!(present.len(), 1, "only the stored `# manifest` matches");
    assert_eq!(present[0].path, "README.md");

    // The real term still matches, so the fixture is not empty and the
    // no-match cases above prove the escaping rather than an empty index.
    assert!(
        !lexical(&fixture, fixture.group_id, own, TERM, None, None, 10)
            .await
            .is_empty()
    );

    fixture.cleanup(&[fixture.group_id]).await;
}

#[tokio::test]
async fn the_extra_row_reports_truncation_without_returning_it() {
    let Some(fixture) = Fixture::setup().await else {
        return;
    };
    index_with_code(&fixture).await;
    let source = fixture.source().await;
    let generation = serving_generation(&fixture.db, &source)
        .await
        .expect("resolve the serving generation")
        .expect("the repository serves an active generation");
    let own = vec![fixture.group_id];

    // One extra row is asked for, so a limit of one over two matching files
    // reports truncation and still returns a single hit.
    let hits = lexical(&fixture, fixture.group_id, own.clone(), TERM, None, None, 2).await;
    assert_eq!(hits.len(), 2, "the read asks for limit + 1 rows");
    let response = search_response(&source, &generation, hits, 1);
    assert!(response.truncated, "a dropped hit is reported");
    assert_eq!(response.hits.len(), 1, "the probe row is not returned");

    // A limit that covers every stored match is not truncation, even though the
    // read asked for one more row than the limit and got fewer back.
    let all = lexical(&fixture, fixture.group_id, own, TERM, None, None, 4).await;
    assert_eq!(all.len(), 3, "three stored files match");
    assert!(!search_response(&source, &generation, all, 3).truncated);

    fixture.cleanup(&[fixture.group_id]).await;
}

#[tokio::test]
async fn a_building_or_absent_generation_is_never_searched() {
    let Some(fixture) = Fixture::setup().await else {
        return;
    };
    // A building generation can hold chunk rows, but the route refuses to search
    // it: only the generation the repository advertises may answer.
    let building = fixture.start_building().await;
    fixture.store_manifest(building).await;
    let source = fixture.source().await;
    assert!(
        serving_generation(&fixture.db, &source)
            .await
            .expect("resolve the serving generation")
            .is_none(),
        "a building generation is never served: {building} is not activated"
    );

    // Once activated, the same generation is searchable.
    index_with_code(&fixture).await;
    let activated = fixture.source().await;
    assert!(
        serving_generation(&fixture.db, &activated)
            .await
            .expect("resolve the serving generation")
            .is_some(),
        "an activated generation is searchable"
    );

    fixture.cleanup(&[fixture.group_id]).await;
}

#[tokio::test]
async fn a_foreign_group_reads_no_hits() {
    let Some(fixture) = Fixture::setup().await else {
        return;
    };
    index_with_code(&fixture).await;
    let foreign_group_id = fixture.foreign_group().await;

    // The route answers an unresolvable repository before it would search, and the
    // accessor is group-scoped as well, so a foreign scope matches no row.
    assert!(
        fixture
            .db
            .get_git_repository_source(foreign_group_id, fixture.repository_key)
            .await
            .expect("read the source as a foreign group")
            .is_none()
    );
    let leaked = lexical(
        &fixture,
        foreign_group_id,
        vec![foreign_group_id],
        TERM,
        None,
        None,
        10,
    )
    .await;
    assert!(
        leaked.is_empty(),
        "a foreign group reads no hit ({} returned)",
        leaked.len()
    );
    // A scope that does not include the owning group sees nothing either, which
    // is how a private repository stays private to its members.
    let anonymous = lexical(&fixture, fixture.group_id, Vec::new(), TERM, None, None, 10).await;
    assert!(
        anonymous.is_empty(),
        "a scope without the owner sees no hit"
    );
    // The owning scope still reads its own hits.
    let owned = lexical(
        &fixture,
        fixture.group_id,
        vec![fixture.group_id],
        TERM,
        None,
        None,
        10,
    )
    .await;
    assert!(!owned.is_empty(), "the owning scope reads its own hits");
    assert!(
        owned
            .iter()
            .all(|hit| hit.matched != GitCodeMatchKind::PathExact)
    );

    fixture.cleanup(&[fixture.group_id, foreign_group_id]).await;
}
