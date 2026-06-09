use axum::{
    Json,
    extract::{Query, State},
    http::StatusCode,
    response::IntoResponse,
};
use serde::Deserialize;

use crate::contracts::{ApiErrorResponse, UserDirectoryEntryResponse};

use super::{ApiState, auth::CurrentUser, errors::internal_error_response};

#[derive(Debug, Deserialize)]
pub(crate) struct UserDirectoryQuery {
    #[serde(default)]
    query: String,
    #[serde(default = "default_limit")]
    limit: usize,
}

const fn default_limit() -> usize {
    10
}

#[utoipa::path(
    get,
    path = "/v1/user-directory",
    params(
        ("query" = Option<String>, Query, description = "Search login_name or display_name"),
        ("limit" = Option<usize>, Query, description = "Max entries to return")
    ),
    responses(
        (status = 200, description = "Matching active users", body = [UserDirectoryEntryResponse]),
        (status = 401, description = "Missing or invalid bearer token", body = ApiErrorResponse)
    )
)]
pub(crate) async fn search_user_directory(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Query(query): Query<UserDirectoryQuery>,
) -> impl IntoResponse {
    match state
        .app
        .auth
        .search_user_directory(&session.user, &query.query, query.limit)
        .await
    {
        Ok(users) => (
            StatusCode::OK,
            Json(
                users
                    .into_iter()
                    .map(|user| UserDirectoryEntryResponse {
                        user_id: user.id,
                        login_name: user.login_name,
                        display_name: user.display_name,
                    })
                    .collect::<Vec<_>>(),
            ),
        )
            .into_response(),
        Err(error) => internal_error_response(error),
    }
}
