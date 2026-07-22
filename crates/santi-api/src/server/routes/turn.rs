use super::*;

#[utoipa::path(
    get,
    path = "/api/v1/turn-events",
    params(
        ("since" = Option<i64>, Query),
        ("limit" = Option<usize>, Query)
    ),
    responses(
        (status = 200, body = Vec<TurnEventPage>),
        (status = 500, body = SantiError)
    )
)]
pub(super) async fn turn_events(
    State(service): State<Service>,
    Query(params): Query<TurnEventParams>,
) -> Result<Json<Vec<TurnEventPage>>, ApiError> {
    let since = params.since.unwrap_or(0);
    let limit = params.limit.unwrap_or(256).clamp(1, 1000);
    let pages = service
        .turn_events_since(since, limit)
        .map_err(ApiError::from_service)?
        .into_iter()
        .map(|(cursor, event)| TurnEventPage { cursor, event })
        .collect();
    Ok(Json(pages))
}

#[derive(serde::Deserialize)]
pub(super) struct TurnEventParams {
    since: Option<i64>,
    limit: Option<usize>,
}
