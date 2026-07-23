use axum::{
    Json,
    extract::{Path, State},
};
use santi_core::Fault;
use santi_core::service::Service;

use super::ApiError;
use santi_core::effect;

#[derive(Debug, serde::Deserialize, utoipa::ToSchema)]
pub struct ResolveEffectRequest {
    pub outcome: effect::Outcome,
    pub evidence: String,
}

#[utoipa::path(
    get,
    path = "/api/v1/effects/{effect_id}",
    params(("effect_id" = String, Path)),
    responses(
        (status = 200, body = effect::Status),
        (status = 404, body = Fault),
        (status = 500, body = Fault)
    )
)]
pub async fn effect_status(
    State(service): State<Service>,
    Path(effect_id): Path<String>,
) -> Result<Json<effect::Status>, ApiError> {
    service
        .effect_status(&effect_id)
        .map_err(ApiError::from_service)?
        .map(Json)
        .ok_or_else(|| ApiError::not_found("effect not found"))
}

#[utoipa::path(
    post,
    path = "/api/v1/effects/{effect_id}/resolve",
    params(("effect_id" = String, Path)),
    request_body = ResolveEffectRequest,
    responses(
        (status = 200, body = effect::Status),
        (status = 400, body = Fault),
        (status = 404, body = Fault),
        (status = 500, body = Fault)
    )
)]
pub async fn resolve_effect(
    State(service): State<Service>,
    Path(effect_id): Path<String>,
    Json(request): Json<ResolveEffectRequest>,
) -> Result<Json<effect::Status>, ApiError> {
    service
        .resolve_effect(&effect_id, request.outcome, &request.evidence)
        .map_err(ApiError::from_service)?
        .map(Json)
        .ok_or_else(|| ApiError::not_found("effect not found"))
}
