use axum::{
    Json,
    extract::{Path, Query, State},
};
use santi_core::service::Service;
use santi_core::{ErrorIncident, ErrorScope, SantiError};

use super::ApiError;

#[derive(serde::Deserialize)]
pub(super) struct ErrorQueryParams {
    limit: Option<i64>,
}

#[utoipa::path(
    get,
    path = "/api/v1/strands/{strand_id}/errors",
    params(
        ("strand_id" = String, Path),
        ("limit" = Option<i64>, Query)
    ),
    responses(
        (status = 200, body = Vec<ErrorIncident>),
        (status = 404, body = SantiError),
        (status = 500, body = SantiError)
    )
)]
pub(super) async fn strand_errors(
    State(service): State<Service>,
    Path(strand_id): Path<String>,
    Query(params): Query<ErrorQueryParams>,
) -> Result<Json<Vec<ErrorIncident>>, ApiError> {
    service
        .strand_errors(&strand_id, params.limit.unwrap_or(50))
        .map_err(ApiError::from_service)?
        .map(Json)
        .ok_or_else(|| ApiError::not_found("strand not found"))
}

#[utoipa::path(
    get,
    path = "/api/v1/errors/{scope_kind}/{scope_id}",
    params(
        ("scope_kind" = String, Path),
        ("scope_id" = String, Path),
        ("limit" = Option<i64>, Query)
    ),
    responses(
        (status = 200, body = Vec<ErrorIncident>),
        (status = 500, body = SantiError)
    )
)]
pub(super) async fn errors(
    State(service): State<Service>,
    Path((scope_kind, scope_id)): Path<(String, String)>,
    Query(params): Query<ErrorQueryParams>,
) -> Result<Json<Vec<ErrorIncident>>, ApiError> {
    service
        .errors(
            &ErrorScope::new(scope_kind, scope_id),
            params.limit.unwrap_or(50),
        )
        .map(Json)
        .map_err(ApiError::from_service)
}
