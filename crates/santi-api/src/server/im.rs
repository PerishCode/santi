use axum::{
    Json,
    extract::{Path, Query, State},
};
use santi_core::{
    ImInboxEntry, ImSendRequest, ImSendResponse, IngestOutcome, SantiError, SantiService,
};

use super::ApiError;

#[utoipa::path(
    post,
    path = "/api/v1/im/send",
    request_body = ImSendRequest,
    responses(
        (status = 200, body = ImSendResponse),
        (status = 423, body = SantiError),
        (status = 404, body = SantiError),
        (status = 500, body = SantiError)
    )
)]
pub(super) async fn send_im(
    State(service): State<SantiService>,
    Json(request): Json<ImSendRequest>,
) -> Result<Json<ImSendResponse>, ApiError> {
    let outcome = service
        .im_send(&request.soul_id, &request.participant_id, &request.content)
        .map_err(ApiError::from_service)?;
    match outcome {
        IngestOutcome::Accepted { receipt } => Ok(Json(ImSendResponse {
            participant_id: request.participant_id,
            receipt,
        })),
        IngestOutcome::Rejected { error } => Err(ApiError::from_santi(*error)),
    }
}

#[utoipa::path(
    get,
    path = "/api/v1/im/inbox/{participant_id}",
    params(
        ("participant_id" = String, Path),
        ("since" = Option<i64>, Query)
    ),
    responses(
        (status = 200, body = Vec<ImInboxEntry>),
        (status = 500, body = SantiError)
    )
)]
pub(super) async fn poll_im(
    State(service): State<SantiService>,
    Path(participant_id): Path<String>,
    Query(params): Query<ImPollParams>,
) -> Result<Json<Vec<ImInboxEntry>>, ApiError> {
    service
        .im_poll(&participant_id, params.since.unwrap_or(0))
        .map(Json)
        .map_err(ApiError::from_service)
}

#[derive(serde::Deserialize)]
pub(super) struct ImPollParams {
    since: Option<i64>,
}
