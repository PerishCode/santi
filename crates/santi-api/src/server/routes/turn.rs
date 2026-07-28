use super::*;
use santi_core::event;
use santi_core::turn;

#[utoipa::path(
    get,
    path = "/api/v1/turn-events",
    security(("downstream_bearer" = [])),
    params(
        ("since" = Option<i64>, Query),
        ("limit" = Option<usize>, Query)
    ),
    responses(
        (status = 200, body = event::Batch),
        (status = 401, body = Fault),
        (status = 500, body = Fault)
    )
)]
pub(super) async fn turn_events(
    State(service): State<Service>,
    headers: axum::http::HeaderMap,
    Query(params): Query<TurnEventParams>,
) -> Result<Json<event::Batch>, ApiError> {
    let principal = service
        .principal(bearer(&headers))
        .map_err(ApiError::from_service)?
        .ok_or_else(|| ApiError::unauthorized("invalid or missing credential"))?;
    let since = params.since.unwrap_or(0).max(0);
    let limit = params.limit.unwrap_or(256).clamp(1, 1000);
    service
        .since(since, &principal.prefix, limit)
        .map(Json)
        .map_err(ApiError::from_service)
}

#[derive(serde::Deserialize)]
pub(super) struct TurnEventParams {
    since: Option<i64>,
    limit: Option<usize>,
}

#[utoipa::path(
    post,
    path = "/api/v1/turns/{turn}/stop",
    params(("turn" = String, Path)),
    responses(
        (status = 200, body = turn::Stop),
        (status = 404, body = Fault),
        (status = 500, body = Fault)
    )
)]
pub async fn stop(
    State(service): State<Service>,
    Path(turn): Path<String>,
) -> Result<Json<turn::Stop>, ApiError> {
    service
        .stop(&turn)
        .map_err(ApiError::from_service)?
        .map(Json)
        .ok_or_else(|| ApiError::not_found("turn not found"))
}
