use super::*;
use santi_core::environ;

#[utoipa::path(
    get,
    path = "/api/v1/souls/{soul}/environment",
    params(("soul" = String, Path)),
    responses(
        (status = 200, body = [environ::Variable]),
        (status = 404, body = Fault),
        (status = 500, body = Fault)
    )
)]
pub async fn soul_environs(
    State(service): State<Service>,
    Path(owner): Path<String>,
) -> Result<Json<Vec<environ::Variable>>, ApiError> {
    list(service, environ::Scope::Soul, owner).await
}

#[utoipa::path(
    post,
    path = "/api/v1/souls/{soul}/environment",
    params(("soul" = String, Path)),
    request_body = environ::Draft,
    responses(
        (status = 200, body = environ::Variable),
        (status = 400, body = Fault),
        (status = 404, body = Fault),
        (status = 500, body = Fault)
    )
)]
pub async fn set_soul_environ(
    State(service): State<Service>,
    Path(owner): Path<String>,
    Json(request): Json<environ::Draft>,
) -> Result<Json<environ::Variable>, ApiError> {
    set(service, environ::Scope::Soul, owner, request).await
}

#[utoipa::path(
    delete,
    path = "/api/v1/souls/{soul}/environment/{name}",
    params(("soul" = String, Path), ("name" = String, Path)),
    responses(
        (status = 204),
        (status = 400, body = Fault),
        (status = 404, body = Fault),
        (status = 500, body = Fault)
    )
)]
pub async fn end_soul_environ(
    State(service): State<Service>,
    Path((owner, name)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    end(service, environ::Scope::Soul, owner, name).await
}

#[utoipa::path(
    get,
    path = "/api/v1/strands/{strand}/environment",
    params(("strand" = String, Path)),
    responses(
        (status = 200, body = [environ::Variable]),
        (status = 404, body = Fault),
        (status = 500, body = Fault)
    )
)]
pub async fn strand_environs(
    State(service): State<Service>,
    Path(owner): Path<String>,
) -> Result<Json<Vec<environ::Variable>>, ApiError> {
    list(service, environ::Scope::Strand, owner).await
}

#[utoipa::path(
    post,
    path = "/api/v1/strands/{strand}/environment",
    params(("strand" = String, Path)),
    request_body = environ::Draft,
    responses(
        (status = 200, body = environ::Variable),
        (status = 400, body = Fault),
        (status = 404, body = Fault),
        (status = 500, body = Fault)
    )
)]
pub async fn set_strand_environ(
    State(service): State<Service>,
    Path(owner): Path<String>,
    Json(request): Json<environ::Draft>,
) -> Result<Json<environ::Variable>, ApiError> {
    set(service, environ::Scope::Strand, owner, request).await
}

#[utoipa::path(
    delete,
    path = "/api/v1/strands/{strand}/environment/{name}",
    params(("strand" = String, Path), ("name" = String, Path)),
    responses(
        (status = 204),
        (status = 400, body = Fault),
        (status = 404, body = Fault),
        (status = 500, body = Fault)
    )
)]
pub async fn end_strand_environ(
    State(service): State<Service>,
    Path((owner, name)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    end(service, environ::Scope::Strand, owner, name).await
}

async fn list(
    service: Service,
    scope: environ::Scope,
    owner: String,
) -> Result<Json<Vec<environ::Variable>>, ApiError> {
    service
        .environs(scope, &owner)
        .await
        .map(Json)
        .map_err(ApiError::from_service)
}

async fn set(
    service: Service,
    scope: environ::Scope,
    owner: String,
    request: environ::Draft,
) -> Result<Json<environ::Variable>, ApiError> {
    service
        .set_environ(scope, &owner, request)
        .await
        .map(Json)
        .map_err(ApiError::from_service)
}

async fn end(
    service: Service,
    scope: environ::Scope,
    owner: String,
    name: String,
) -> Result<StatusCode, ApiError> {
    service
        .end_environ(scope, &owner, &name)
        .await
        .map(|_| StatusCode::NO_CONTENT)
        .map_err(ApiError::from_service)
}
