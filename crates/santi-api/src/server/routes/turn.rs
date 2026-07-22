use super::*;

#[utoipa::path(
    get,
    path = "/api/v1/turn-events",
    security(("downstream_bearer" = [])),
    params(
        ("since" = Option<i64>, Query),
        ("limit" = Option<usize>, Query)
    ),
    responses(
        (status = 200, body = TurnEventBatch),
        (status = 401, body = SantiError),
        (status = 500, body = SantiError)
    )
)]
pub(super) async fn turn_events(
    State(service): State<Service>,
    headers: axum::http::HeaderMap,
    Query(params): Query<TurnEventParams>,
) -> Result<Json<TurnEventBatch>, ApiError> {
    let principal = service
        .principal(bearer(&headers))
        .map_err(ApiError::from_service)?
        .ok_or_else(|| ApiError::unauthorized("invalid or missing credential"))?;
    let since = params.since.unwrap_or(0).max(0);
    let limit = params.limit.unwrap_or(256).clamp(1, 1000);
    service
        .turn_events_since(since, &principal.label_prefix, limit)
        .map(Json)
        .map_err(ApiError::from_service)
}

#[derive(serde::Deserialize)]
pub(super) struct TurnEventParams {
    since: Option<i64>,
    limit: Option<usize>,
}
