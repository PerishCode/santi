use super::*;
use santi_core::{material, receipt, soul, strand, webhook};

#[utoipa::path(
    get,
    path = "/api/v1/receipts/{inbox}",
    params(("inbox" = String, Path)),
    responses(
        (status = 200, body = receipt::Status),
        (status = 404, body = Fault),
        (status = 500, body = Fault)
    )
)]
pub async fn receipt_status(
    State(service): State<Service>,
    Path(inbox): Path<String>,
) -> Result<Json<receipt::Status>, ApiError> {
    let receipt = service
        .receipt_status(&inbox)
        .map_err(ApiError::from_service)?
        .ok_or_else(|| ApiError::not_found("receipt not found"))?;
    Ok(Json(receipt))
}

#[utoipa::path(
    post,
    path = "/api/v1/strands",
    responses((status = 200, body = strand::Created), (status = 500, body = Fault))
)]
pub(super) async fn create_strand(
    State(service): State<Service>,
) -> Result<Json<strand::Created>, ApiError> {
    service
        .create_strand()
        .map(Json)
        .map_err(ApiError::from_service)
}

#[utoipa::path(
    get,
    path = "/api/v1/strands",
    responses((status = 200, body = [Strand]), (status = 500, body = Fault))
)]
pub(super) async fn list_strands(
    State(service): State<Service>,
) -> Result<Json<Vec<Strand>>, ApiError> {
    service
        .list_strands()
        .map(Json)
        .map_err(ApiError::from_service)
}

#[utoipa::path(
    post,
    path = "/api/v1/souls",
    request_body = soul::Draft,
    responses((status = 200, body = Soul), (status = 500, body = Fault))
)]
pub(super) async fn create_soul(
    State(service): State<Service>,
    Json(request): Json<soul::Draft>,
) -> Result<Json<Soul>, ApiError> {
    service
        .create_soul(request)
        .map(Json)
        .map_err(ApiError::from_service)
}

#[utoipa::path(
    get,
    path = "/api/v1/souls",
    responses((status = 200, body = [Soul]), (status = 500, body = Fault))
)]
pub(super) async fn list_souls(
    State(service): State<Service>,
) -> Result<Json<Vec<Soul>>, ApiError> {
    service
        .list_souls()
        .map(Json)
        .map_err(ApiError::from_service)
}

#[utoipa::path(
    get,
    path = "/api/v1/souls/{soul}",
    params(("soul" = String, Path)),
    responses(
        (status = 200, body = Soul),
        (status = 404, body = Fault),
        (status = 500, body = Fault)
    )
)]
pub(super) async fn get_soul(
    State(service): State<Service>,
    Path(soul): Path<String>,
) -> Result<Json<Soul>, ApiError> {
    match service.soul(&soul).map_err(ApiError::from_service)? {
        Some(soul) => Ok(Json(soul)),
        None => Err(ApiError::not_found("soul not found")),
    }
}

#[utoipa::path(
    post,
    path = "/api/v1/webhooks",
    request_body = webhook::Draft,
    responses((status = 200, body = webhook::Subscription), (status = 500, body = Fault))
)]
pub(super) async fn create_webhook(
    State(service): State<Service>,
    Json(request): Json<webhook::Draft>,
) -> Result<Json<webhook::Subscription>, ApiError> {
    service
        .create_webhook(request)
        .map(Json)
        .map_err(ApiError::from_service)
}

#[utoipa::path(
    get,
    path = "/api/v1/webhooks",
    responses((status = 200, body = [webhook::Subscription]), (status = 500, body = Fault))
)]
pub(super) async fn list_webhooks(
    State(service): State<Service>,
) -> Result<Json<Vec<webhook::Subscription>>, ApiError> {
    service
        .list_webhooks()
        .map(Json)
        .map_err(ApiError::from_service)
}

#[utoipa::path(
    get,
    path = "/api/v1/strands/{strand}",
    params(("strand" = String, Path)),
    responses(
        (status = 200, body = strand::Detail),
        (status = 404, body = Fault),
        (status = 500, body = Fault)
    )
)]
pub(super) async fn get_strand(
    State(service): State<Service>,
    Path(strand): Path<String>,
) -> Result<Json<strand::Detail>, ApiError> {
    service
        .strand(&strand)
        .map_err(ApiError::from_service)?
        .map(Json)
        .ok_or_else(|| ApiError::not_found("strand not found"))
}

#[utoipa::path(
    get,
    path = "/api/v1/strands/{strand}/messages",
    params(("strand" = String, Path)),
    responses(
        (status = 200, body = [santi_core::message::Placed]),
        (status = 404, body = Fault),
        (status = 500, body = Fault)
    )
)]
pub(super) async fn list_messages(
    State(service): State<Service>,
    Path(strand): Path<String>,
) -> Result<Json<Vec<santi_core::message::Placed>>, ApiError> {
    service
        .strand(&strand)
        .map_err(ApiError::from_service)?
        .map(|detail| Json(detail.messages))
        .ok_or_else(|| ApiError::not_found("strand not found"))
}

#[utoipa::path(
    post,
    path = "/api/v1/strands/{strand}/materials",
    params(("strand" = String, Path)),
    request_body = material::Request,
    responses(
        (status = 200, body = material::Material),
        (status = 404, body = Fault),
        (status = 500, body = Fault)
    )
)]
pub(super) async fn strand_material(
    State(service): State<Service>,
    Path(strand): Path<String>,
    Json(request): Json<material::Request>,
) -> Result<Json<material::Material>, ApiError> {
    service
        .strand_material(&strand, request)
        .map(Json)
        .map_err(ApiError::from_service)
}
