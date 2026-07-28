use axum::{
    Json,
    extract::{Path, State},
};
use santi_core::Fault;
use santi_core::service::Service;

use super::ApiError;
use santi_core::{effect, trace};

#[derive(Debug, serde::Deserialize, utoipa::ToSchema)]
pub struct ResolveEffectRequest {
    pub outcome: effect::Outcome,
    pub evidence: String,
}

#[utoipa::path(
    get,
    path = "/api/v1/effects/{effect}",
    params(("effect_id" = String, Path)),
    responses(
        (status = 200, body = effect::Status),
        (status = 404, body = Fault),
        (status = 500, body = Fault)
    )
)]
pub async fn effect(
    State(service): State<Service>,
    Path(effect): Path<String>,
) -> Result<Json<effect::Status>, ApiError> {
    service
        .effect(&effect)
        .await
        .map_err(ApiError::from_service)?
        .map(Json)
        .ok_or_else(|| ApiError::not_found("effect not found"))
}

#[utoipa::path(
    get,
    path = "/api/v1/effects/{effect}/trace",
    params(("effect_id" = String, Path)),
    responses(
        (status = 200, body = Vec<trace::Record>),
        (status = 500, body = Fault)
    )
)]
pub async fn trail(
    State(service): State<Service>,
    Path(effect): Path<String>,
) -> Result<Json<Vec<trace::Record>>, ApiError> {
    service
        .trail(&effect)
        .await
        .map(Json)
        .map_err(ApiError::from_service)
}

#[utoipa::path(
    post,
    path = "/api/v1/effects/{effect}/resolve",
    params(("effect_id" = String, Path)),
    request_body = ResolveEffectRequest,
    responses(
        (status = 200, body = effect::Status),
        (status = 400, body = Fault),
        (status = 404, body = Fault),
        (status = 500, body = Fault)
    )
)]
pub async fn settle(
    State(service): State<Service>,
    Path(effect): Path<String>,
    Json(request): Json<ResolveEffectRequest>,
) -> Result<Json<effect::Status>, ApiError> {
    service
        .settle(&effect, request.outcome, &request.evidence)
        .await
        .map_err(ApiError::from_service)?
        .map(Json)
        .ok_or_else(|| ApiError::not_found("effect not found"))
}
