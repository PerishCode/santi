use super::*;

#[utoipa::path(
    get,
    path = "/api/v1/receipts/{inbox_id}",
    params(("inbox_id" = String, Path)),
    responses(
        (status = 200, body = ReceiptStatus),
        (status = 404, body = Fault),
        (status = 500, body = Fault)
    )
)]
pub async fn receipt_status(
    State(service): State<Service>,
    Path(inbox_id): Path<String>,
) -> Result<Json<ReceiptStatus>, ApiError> {
    let receipt = service
        .receipt_status(&inbox_id)
        .map_err(ApiError::from_service)?
        .ok_or_else(|| ApiError::not_found("receipt not found"))?;
    Ok(Json(receipt))
}

#[utoipa::path(
    post,
    path = "/api/v1/strands",
    responses((status = 200, body = CreateStrandResponse), (status = 500, body = Fault))
)]
pub(super) async fn create_strand(
    State(service): State<Service>,
) -> Result<Json<CreateStrandResponse>, ApiError> {
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
    request_body = CreateSoulRequest,
    responses((status = 200, body = Soul), (status = 500, body = Fault))
)]
pub(super) async fn create_soul(
    State(service): State<Service>,
    Json(request): Json<CreateSoulRequest>,
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
    path = "/api/v1/souls/{soul_id}",
    params(("soul_id" = String, Path)),
    responses(
        (status = 200, body = Soul),
        (status = 404, body = Fault),
        (status = 500, body = Fault)
    )
)]
pub(super) async fn get_soul(
    State(service): State<Service>,
    Path(soul_id): Path<String>,
) -> Result<Json<Soul>, ApiError> {
    match service.soul(&soul_id).map_err(ApiError::from_service)? {
        Some(soul) => Ok(Json(soul)),
        None => Err(ApiError::not_found("soul not found")),
    }
}

#[utoipa::path(
    post,
    path = "/api/v1/webhooks",
    request_body = CreateWebhookRequest,
    responses((status = 200, body = WebhookSubscription), (status = 500, body = Fault))
)]
pub(super) async fn create_webhook(
    State(service): State<Service>,
    Json(request): Json<CreateWebhookRequest>,
) -> Result<Json<WebhookSubscription>, ApiError> {
    service
        .create_webhook(request)
        .map(Json)
        .map_err(ApiError::from_service)
}

#[utoipa::path(
    get,
    path = "/api/v1/webhooks",
    responses((status = 200, body = [WebhookSubscription]), (status = 500, body = Fault))
)]
pub(super) async fn list_webhooks(
    State(service): State<Service>,
) -> Result<Json<Vec<WebhookSubscription>>, ApiError> {
    service
        .list_webhooks()
        .map(Json)
        .map_err(ApiError::from_service)
}

#[utoipa::path(
    get,
    path = "/api/v1/strands/{strand_id}",
    params(("strand_id" = String, Path)),
    responses(
        (status = 200, body = StrandDetail),
        (status = 404, body = Fault),
        (status = 500, body = Fault)
    )
)]
pub(super) async fn get_strand(
    State(service): State<Service>,
    Path(strand_id): Path<String>,
) -> Result<Json<StrandDetail>, ApiError> {
    service
        .strand(&strand_id)
        .map_err(ApiError::from_service)?
        .map(Json)
        .ok_or_else(|| ApiError::not_found("strand not found"))
}

#[utoipa::path(
    get,
    path = "/api/v1/strands/{strand_id}/messages",
    params(("strand_id" = String, Path)),
    responses(
        (status = 200, body = [santi_core::StrandMessage]),
        (status = 404, body = Fault),
        (status = 500, body = Fault)
    )
)]
pub(super) async fn list_messages(
    State(service): State<Service>,
    Path(strand_id): Path<String>,
) -> Result<Json<Vec<santi_core::StrandMessage>>, ApiError> {
    service
        .strand(&strand_id)
        .map_err(ApiError::from_service)?
        .map(|detail| Json(detail.messages))
        .ok_or_else(|| ApiError::not_found("strand not found"))
}

#[utoipa::path(
    post,
    path = "/api/v1/strands/{strand_id}/materials",
    params(("strand_id" = String, Path)),
    request_body = MaterialRequest,
    responses(
        (status = 200, body = StrandMaterial),
        (status = 404, body = Fault),
        (status = 500, body = Fault)
    )
)]
pub(super) async fn strand_material(
    State(service): State<Service>,
    Path(strand_id): Path<String>,
    Json(request): Json<MaterialRequest>,
) -> Result<Json<StrandMaterial>, ApiError> {
    service
        .strand_material(&strand_id, request)
        .map(Json)
        .map_err(ApiError::from_service)
}
