//! Viewer-gated bounded lexical code search (issue #681 phase 5D).
//!
//! This is the first HTTP search over the already-indexed repository content. It
//! reuses the guard chain the other Git reads share — group, the Viewer floor,
//! the group-scoped repository, the active ready generation — and then asks the
//! existing group-scoped lexical accessor for hits from that generation. No new
//! SQL, persistence, provider call, MCP tool, symbol index, or diff work is
//! involved: the matching, escaping, visibility filter, ordering, and the
//! active-generation guard all already live in the storage layer, and this route
//! only bounds what a caller may ask for and what comes back.
//!
//! The response is a dedicated HTTP contract, not an MCP projection: it carries
//! the verbatim stored chunk text with the repository, generation, ref, commit,
//! file, path, and inclusive line range a caller needs to check the hit, plus
//! the source's index status, commit checkpoint, and coverage counters. Raw
//! acquisition blobs, provider blob ids, secret references, and connection state
//! never cross it.

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use uuid::Uuid;

use crate::{
    contracts::sources::{
        GIT_CODE_SEARCH_LANGUAGE_MAX_CHARS, GIT_CODE_SEARCH_LIMIT_MAX, GIT_CODE_SEARCH_LIMIT_MIN,
        GIT_CODE_SEARCH_PATH_PREFIX_MAX_CHARS, GIT_CODE_SEARCH_QUERY_MAX_CHARS, GitCodeLexicalHit,
        GitCodeSearchHit, GitCodeSearchQuery, GitCodeSearchResponse,
    },
    db::{GitLexicalCodeSearch, StoredGitRepositoryGeneration, StoredGitRepositorySource},
    domain_errors::DomainError,
    services::git_repository::SafeTreePath,
};

use super::{
    ApiState,
    auth::CurrentUser,
    errors::library_management_error_response,
    git_repository_files::{
        generation_not_ready, repository_not_found, require_manifest_read, serving_generation,
    },
    group_access::{group_access_error_response, group_for_user},
};

#[utoipa::path(
    get,
    path = "/v1/groups/by-path/{group_path}/git-repositories/{repository_key}/code-search",
    params(
        ("group_path" = String, Path, description = "URL-encoded group path"),
        ("repository_key" = Uuid, Path, description = "Git repository source key"),
        GitCodeSearchQuery
    ),
    responses(
        (status = 200, description = "Bounded lexical hits from the active generation", body = GitCodeSearchResponse),
        (status = 400, description = "Invalid search term, path prefix, language, or limit", body = crate::contracts::ApiErrorResponse),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Group or repository not found"),
        (status = 409, description = "No active ready index generation to search", body = crate::contracts::ApiErrorResponse)
    )
)]
pub(crate) async fn search_git_repository_code(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path((group_path, repository_key)): Path<(String, Uuid)>,
    Query(query): Query<GitCodeSearchQuery>,
) -> Response {
    let group = match group_for_user(&state, session.user.id, &group_path).await {
        Ok(group) => group,
        Err(error) => return group_access_error_response(error),
    };
    if let Err(error) = require_manifest_read(&group) {
        return group_access_error_response(error);
    }
    // The search is validated before any storage work, so an invalid term never
    // reaches the query and the refusal never echoes it.
    let filters = match SearchFilters::validated(&query) {
        Ok(filters) => filters,
        Err(error) => return library_management_error_response(error),
    };
    let source = match found_repository(
        state
            .app
            .db
            .get_git_repository_source(group.id, repository_key)
            .await,
    ) {
        Ok(source) => source,
        Err(response) => return *response,
    };
    let generation = match found_generation(serving_generation(&state.app.db, &source).await) {
        Ok(generation) => generation,
        Err(response) => return *response,
    };
    // The visible groups come from the caller's resolved access scope, never
    // from the request, so a private repository is searched only for a caller who
    // can already read it.
    let scope = match state
        .app
        .auth
        .access_scope(Some(session.user.id), Some(group_path.clone()))
        .await
    {
        Ok(scope) => scope,
        Err(error) => return group_access_error_response(error),
    };
    let hits = match state
        .app
        .db
        .lexical_search_git_generation_chunks(
            group.id,
            &GitLexicalCodeSearch {
                repository_key: source.repository_key,
                query: filters.query.clone(),
                path_prefix: filters.path_prefix.clone(),
                language: filters.language.clone(),
                visible_group_ids: scope.private_group_ids,
                limit: filters.fetch_limit(),
            },
        )
        .await
    {
        Ok(hits) => hits,
        Err(error) => return library_management_error_response(error),
    };
    (
        StatusCode::OK,
        Json(search_response(&source, &generation, hits, filters.limit)),
    )
        .into_response()
}

/// The route's single decision about the group-scoped repository lookup.
///
/// A repository the caller's group does not own, and a repository key that names
/// nothing, both read as `None` and answer with the very same bounded `404` the
/// other Git reads return: same status, same body, no repository or provider
/// detail. Keeping that mapping in one named place is what makes it testable
/// without a database and impossible to drift into a distinguishable shape that
/// would let this read enumerate repository keys. The response travels boxed so
/// this stays a plain `Result`.
fn found_repository(
    found: anyhow::Result<Option<StoredGitRepositorySource>>,
) -> Result<StoredGitRepositorySource, Box<Response>> {
    match found {
        Ok(Some(source)) => Ok(source),
        Ok(None) => Err(Box::new(repository_not_found())),
        Err(error) => Err(Box::new(library_management_error_response(error))),
    }
}

/// The route's single decision about the serving generation.
///
/// A repository with no active ready generation is a bounded `409`, never an
/// empty hit list: an empty result would read as "this repository has no such
/// code" instead of "nothing is indexed yet", and a search must not become a
/// second, weaker way to probe index state.
fn found_generation(
    found: anyhow::Result<Option<StoredGitRepositoryGeneration>>,
) -> Result<StoredGitRepositoryGeneration, Box<Response>> {
    match found {
        Ok(Some(generation)) => Ok(generation),
        Ok(None) => Err(Box::new(generation_not_ready())),
        Err(error) => Err(Box::new(library_management_error_response(error))),
    }
}

/// The validated bounds of one search.
#[derive(Debug, Clone, PartialEq, Eq)]
struct SearchFilters {
    query: String,
    path_prefix: Option<String>,
    language: Option<String>,
    limit: i64,
}

impl SearchFilters {
    /// Validate the wire query, or return a bounded invalid-argument error.
    ///
    /// Every refusal names the rule and never the value, so a rejected term,
    /// prefix, or language cannot be echoed back into a log or a response.
    fn validated(query: &GitCodeSearchQuery) -> anyhow::Result<Self> {
        let term = query.query.trim();
        if term.is_empty() || term.chars().count() > GIT_CODE_SEARCH_QUERY_MAX_CHARS {
            return Err(DomainError::invalid_argument("git_code_search_query_invalid").into());
        }
        if !(GIT_CODE_SEARCH_LIMIT_MIN..=GIT_CODE_SEARCH_LIMIT_MAX).contains(&query.limit) {
            return Err(
                DomainError::invalid_argument("git_code_search_limit_out_of_bounds").into(),
            );
        }
        let path_prefix = match query
            .path_prefix
            .as_deref()
            .map(|prefix| prefix.trim_end_matches('/'))
        {
            Some(prefix) if !prefix.is_empty() => {
                if prefix.chars().count() > GIT_CODE_SEARCH_PATH_PREFIX_MAX_CHARS
                    || SafeTreePath::parse(prefix).is_err()
                {
                    return Err(DomainError::invalid_argument(
                        "git_code_search_path_prefix_invalid",
                    )
                    .into());
                }
                Some(prefix.to_string())
            }
            // An absent, empty, or separator-only prefix narrows nothing rather
            // than matching nothing, so a caller cannot turn a search into an
            // empty result by passing a blank value, and the trailing separator of
            // `src/` cannot smuggle an empty segment past the path rules.
            _ => None,
        };
        let language = match query.language.as_deref() {
            Some(language) if !language.is_empty() => {
                if !is_language_token(language) {
                    return Err(
                        DomainError::invalid_argument("git_code_search_language_invalid").into(),
                    );
                }
                Some(language.to_string())
            }
            _ => None,
        };
        Ok(Self {
            query: term.to_string(),
            path_prefix,
            language,
            limit: i64::from(query.limit),
        })
    }

    /// Rows to read: one more than the caller asked for, so the extra row is the
    /// only evidence of truncation.
    fn fetch_limit(&self) -> i64 {
        self.limit + 1
    }
}

/// The classified language token shape the manifest stores:
/// `^[a-z0-9][a-z0-9_+#-]{0,31}$`.
fn is_language_token(value: &str) -> bool {
    let mut characters = value.chars();
    match characters.next() {
        Some(first) if first.is_ascii_lowercase() || first.is_ascii_digit() => {}
        _ => return false,
    }
    value.chars().count() <= GIT_CODE_SEARCH_LANGUAGE_MAX_CHARS
        && value.chars().all(|character| {
            character.is_ascii_lowercase()
                || character.is_ascii_digit()
                || matches!(character, '_' | '+' | '#' | '-')
        })
}

/// Assemble the response from the resolved generation and the hits read with one
/// extra row.
///
/// The extra hit is kept only as the truncation probe: the response never returns
/// more than the requested limit, and `truncated` says whether the storage layer
/// had more. A search with no matches is a truthful empty result with the same
/// provenance and coverage, never an error.
fn search_response(
    source: &StoredGitRepositorySource,
    generation: &StoredGitRepositoryGeneration,
    hits: Vec<GitCodeLexicalHit>,
    limit: i64,
) -> GitCodeSearchResponse {
    let truncated = hits.len() as i64 > limit;
    let kept: Vec<GitCodeSearchHit> = hits.iter().take(limit as usize).map(search_hit).collect();
    GitCodeSearchResponse {
        repository_key: source.repository_key,
        generation_key: generation.generation_key,
        generation_number: generation.generation_number,
        ref_name: generation.ref_name.clone(),
        commit_sha: generation.commit_sha.clone(),
        index_status: source.index_status,
        checkpoint: source.checkpoint.clone(),
        file_count: generation.file_count,
        excluded_file_count: generation.excluded_file_count,
        total_bytes: generation.total_bytes,
        hits: kept,
        truncated,
    }
}

/// Project one stored hit onto the HTTP contract, verbatim.
fn search_hit(hit: &GitCodeLexicalHit) -> GitCodeSearchHit {
    GitCodeSearchHit {
        repository_key: hit.repository_key,
        generation_key: hit.generation_key,
        generation_number: hit.generation_number,
        ref_name: hit.ref_name.clone(),
        commit_sha: hit.commit_sha.clone(),
        visibility: hit.visibility,
        file_key: hit.file_key,
        path: hit.path.clone(),
        language: hit.language.clone(),
        chunk_key: hit.chunk_key,
        chunk_index: hit.chunk_index,
        start_line: hit.start_line,
        end_line: hit.end_line,
        text: hit.text.clone(),
        score: hit.score,
        matched: hit.matched,
    }
}

#[cfg(test)]
#[path = "git_repository_code_search_tests.rs"]
mod tests;

/// Disposable-search helpers shared by the database-gated round trips, over the
/// shared Git fixture.
#[cfg(test)]
#[path = "git_repository_code_search_db_fixture.rs"]
mod fixture;

/// Database-gated round trips for the same route, over the shared disposable
/// Git fixture.
#[cfg(test)]
#[path = "git_repository_code_search_db_tests.rs"]
mod db_tests;
