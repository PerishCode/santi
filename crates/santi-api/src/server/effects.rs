use axum::{
    Json,
    extract::{Path, State},
};
use santi_core::{EffectResolutionOutcome, EffectStatus, SantiError, SantiService};

use super::ApiError;

#[derive(Debug, serde::Deserialize, utoipa::ToSchema)]
pub struct ResolveEffectRequest {
    pub outcome: EffectResolutionOutcome,
    pub evidence: String,
}

#[utoipa::path(
    get,
    path = "/api/v1/effects/{effect_id}",
    params(("effect_id" = String, Path)),
    responses(
        (status = 200, body = EffectStatus),
        (status = 404, body = SantiError),
        (status = 500, body = SantiError)
    )
)]
pub async fn effect_status(
    State(service): State<SantiService>,
    Path(effect_id): Path<String>,
) -> Result<Json<EffectStatus>, ApiError> {
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
        (status = 200, body = EffectStatus),
        (status = 400, body = SantiError),
        (status = 404, body = SantiError),
        (status = 500, body = SantiError)
    )
)]
pub async fn resolve_effect(
    State(service): State<SantiService>,
    Path(effect_id): Path<String>,
    Json(request): Json<ResolveEffectRequest>,
) -> Result<Json<EffectStatus>, ApiError> {
    service
        .resolve_effect(&effect_id, request.outcome, &request.evidence)
        .map_err(ApiError::from_service)?
        .map(Json)
        .ok_or_else(|| ApiError::not_found("effect not found"))
}
