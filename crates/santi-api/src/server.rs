use std::{convert::Infallible, env, fs, net::SocketAddr, path::PathBuf, sync::Arc};

use crate::{config, provider};
use axum::{
    Json, Router,
    extract::{Path, Request, State},
    http::StatusCode,
    middleware::{self, Next},
    response::{
        IntoResponse, Response, Sse,
        sse::{Event, KeepAlive},
    },
    routing::{get, post},
};
use futures_core::Stream;
use santi_core::{
    CreateSessionResponse, ErrorResponse, HealthResponse, MaterialRequest, SantiService,
    SantiServiceConfig, SantiStreamEvent, SantiStreamPayload, SendSessionAcceptedResponse,
    SendSessionRequest, Session, SessionDetail, SessionMaterial, SessionProfile,
    SessionRuntimeSnapshot, SessionSummary, SoulProfile, UpdateSessionRequest, prefixed_id,
    timestamp_now,
};
use tower_http::{
    cors::{Any, CorsLayer},
    trace::TraceLayer,
};
use utoipa::OpenApi;

pub fn export_openapi_json() -> Result<String, String> {
    serde_json::to_string_pretty(&ApiDoc::openapi()).map_err(|error| error.to_string())
}

pub async fn serve(config: config::ConfigService) -> Result<(), String> {
    let provider = provider::from_config(config.provider_config()?);
    // Defaults anchor on the santi home (`SANTI_HOME`, else `~/.santi`); explicit
    // env always overrides. The data dirs are created so a zero-config run works.
    let home = config::santi_home();
    let database_path = env::var("SANTI_DB")
        .unwrap_or_else(|_| home.join("runtime").join("db").display().to_string());
    let runtime_root = env::var("SANTI_RUNTIME_ROOT")
        .unwrap_or_else(|_| home.join("runtime").display().to_string());
    let execution_root = env::var("SANTI_EXECUTION_ROOT")
        .unwrap_or_else(|_| home.join("execution").display().to_string());
    if let Some(parent) = PathBuf::from(&database_path).parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    fs::create_dir_all(&runtime_root).map_err(|error| error.to_string())?;
    fs::create_dir_all(&execution_root).map_err(|error| error.to_string())?;
    let service = SantiService::open(
        SantiServiceConfig {
            database_path,
            runtime_root,
            execution_root,
            bind_addr: Some(bind_addr_string()),
        },
        provider,
    )?;
    let address: SocketAddr = bind_addr_string()
        .parse()
        .map_err(|_| "SANTI_HOST/SANTI_PORT did not form a valid socket address".to_string())?;
    let listener = tokio::net::TcpListener::bind(address)
        .await
        .map_err(|error| error.to_string())?;
    // Optional bearer auth: when SANTI_API_KEY is set, every endpoint except
    // /health requires `Authorization: Bearer <key>`. Unset = open (default).
    let api_key: Option<Arc<str>> = env::var("SANTI_API_KEY")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .map(Arc::from);
    if api_key.is_some() {
        println!("santi-api: bearer auth enabled");
    }
    println!("santi-api listening on http://{address}");
    axum::serve(listener, router(service, api_key))
        .await
        .map_err(|error| error.to_string())
}

fn bind_addr_string() -> String {
    let host = env::var("SANTI_HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
    let port = env::var("SANTI_PORT")
        .ok()
        .and_then(|value| value.parse::<u16>().ok())
        .unwrap_or(43307);
    format!("{host}:{port}")
}

fn router(service: SantiService, api_key: Option<Arc<str>>) -> Router {
    // Everything except /health is bearer-gated when a key is configured.
    let protected = Router::new()
        .route("/api/v1/openapi.json", get(openapi))
        .route("/api/v1/sessions", post(create_session).get(list_sessions))
        .route(
            "/api/v1/sessions/{session_id}",
            get(get_session).patch(update_session),
        )
        .route("/api/v1/sessions/{session_id}/messages", get(list_messages))
        .route(
            "/api/v1/sessions/{session_id}/materials",
            post(session_material),
        )
        .route("/api/v1/sessions/{session_id}/events", get(session_events))
        .route("/api/v1/sessions/{session_id}/send", post(send_session))
        .route(
            "/api/v1/sessions/{session_id}/runtime",
            get(runtime_snapshot),
        )
        .route(
            "/api/v1/bucket/{soul_id}/{session_id}/{*key}",
            get(crate::bucket::get_bucket_object),
        )
        .route_layer(middleware::from_fn_with_state(api_key, require_bearer));

    Router::new()
        .route("/api/v1/health", get(health))
        .merge(protected)
        .layer(TraceLayer::new_for_http())
        .layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods(Any)
                .allow_headers(Any),
        )
        .with_state(service)
}

/// Enforce `Authorization: Bearer <key>` when an API key is configured.
async fn require_bearer(
    State(expected): State<Option<Arc<str>>>,
    request: Request,
    next: Next,
) -> Result<Response, ApiError> {
    if let Some(expected) = expected.as_deref() {
        let presented = request
            .headers()
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("Bearer "));
        if presented != Some(expected) {
            return Err(ApiError::unauthorized("missing or invalid bearer token"));
        }
    }
    Ok(next.run(request).await)
}

#[utoipa::path(
    get,
    path = "/api/v1/health",
    responses((status = 200, body = HealthResponse))
)]
async fn health() -> Json<HealthResponse> {
    Json(HealthResponse {
        ok: true,
        service: "santi-api".to_string(),
    })
}

#[utoipa::path(
    post,
    path = "/api/v1/sessions",
    responses((status = 200, body = CreateSessionResponse), (status = 500, body = ErrorResponse))
)]
async fn create_session(
    State(service): State<SantiService>,
) -> Result<Json<CreateSessionResponse>, ApiError> {
    service
        .create_session()
        .map(Json)
        .map_err(ApiError::internal)
}

#[utoipa::path(
    get,
    path = "/api/v1/sessions",
    responses((status = 200, body = [SessionSummary]), (status = 500, body = ErrorResponse))
)]
async fn list_sessions(
    State(service): State<SantiService>,
) -> Result<Json<Vec<SessionSummary>>, ApiError> {
    service
        .list_sessions()
        .map(Json)
        .map_err(ApiError::internal)
}

#[utoipa::path(
    get,
    path = "/api/v1/sessions/{session_id}",
    params(("session_id" = String, Path)),
    responses(
        (status = 200, body = SessionDetail),
        (status = 404, body = ErrorResponse),
        (status = 500, body = ErrorResponse)
    )
)]
async fn get_session(
    State(service): State<SantiService>,
    Path(session_id): Path<String>,
) -> Result<Json<SessionDetail>, ApiError> {
    service
        .session(&session_id)
        .map_err(ApiError::internal)?
        .map(Json)
        .ok_or_else(|| ApiError::not_found("session not found"))
}

#[utoipa::path(
    patch,
    path = "/api/v1/sessions/{session_id}",
    params(("session_id" = String, Path)),
    request_body = UpdateSessionRequest,
    responses(
        (status = 200, body = SessionSummary),
        (status = 404, body = ErrorResponse),
        (status = 500, body = ErrorResponse)
    )
)]
async fn update_session(
    State(service): State<SantiService>,
    Path(session_id): Path<String>,
    Json(request): Json<UpdateSessionRequest>,
) -> Result<Json<SessionSummary>, ApiError> {
    service
        .update_session(&session_id, request)
        .map_err(ApiError::internal)?
        .map(Json)
        .ok_or_else(|| ApiError::not_found("session not found"))
}

#[utoipa::path(
    get,
    path = "/api/v1/sessions/{session_id}/messages",
    params(("session_id" = String, Path)),
    responses(
        (status = 200, body = [santi_core::SessionMessage]),
        (status = 404, body = ErrorResponse),
        (status = 500, body = ErrorResponse)
    )
)]
async fn list_messages(
    State(service): State<SantiService>,
    Path(session_id): Path<String>,
) -> Result<Json<Vec<santi_core::SessionMessage>>, ApiError> {
    service
        .session(&session_id)
        .map_err(ApiError::internal)?
        .map(|detail| Json(detail.messages))
        .ok_or_else(|| ApiError::not_found("session not found"))
}

#[utoipa::path(
    post,
    path = "/api/v1/sessions/{session_id}/materials",
    params(("session_id" = String, Path)),
    request_body = MaterialRequest,
    responses(
        (status = 200, body = SessionMaterial),
        (status = 404, body = ErrorResponse),
        (status = 500, body = ErrorResponse)
    )
)]
async fn session_material(
    State(service): State<SantiService>,
    Path(session_id): Path<String>,
    Json(request): Json<MaterialRequest>,
) -> Result<Json<SessionMaterial>, ApiError> {
    service
        .session_material(&session_id, request)
        .map(Json)
        .map_err(ApiError::from_service)
}

async fn session_events(
    State(service): State<SantiService>,
    Path(session_id): Path<String>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, ApiError> {
    let session = service
        .session(&session_id)
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::not_found("session not found"))?;
    drop(session);

    let mut receiver = service.subscribe_stream();
    let open_session_id = session_id.clone();
    let stream = async_stream::stream! {
        yield Ok(sse_event(SantiStreamEvent {
            event_id: prefixed_id("stream"),
            session_id: open_session_id,
            created_at: timestamp_now(),
            payload: SantiStreamPayload::StreamOpen,
        }));

        loop {
            match receiver.recv().await {
                Ok(event) if event.session_id == session_id => yield Ok(sse_event(event)),
                Ok(_) => {}
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    };
    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}

#[utoipa::path(
    post,
    path = "/api/v1/sessions/{session_id}/send",
    params(("session_id" = String, Path)),
    request_body = SendSessionRequest,
    responses(
        (status = 200, body = SendSessionAcceptedResponse),
        (status = 404, body = ErrorResponse),
        (status = 500, body = ErrorResponse)
    )
)]
async fn send_session(
    State(service): State<SantiService>,
    Path(session_id): Path<String>,
    Json(request): Json<SendSessionRequest>,
) -> Result<Json<SendSessionAcceptedResponse>, ApiError> {
    service
        .send_session(&session_id, request)
        .await
        .map(Json)
        .map_err(ApiError::internal)
}

#[utoipa::path(
    get,
    path = "/api/v1/sessions/{session_id}/runtime",
    params(("session_id" = String, Path)),
    responses(
        (status = 200, body = SessionRuntimeSnapshot),
        (status = 404, body = ErrorResponse),
        (status = 500, body = ErrorResponse)
    )
)]
async fn runtime_snapshot(
    State(service): State<SantiService>,
    Path(session_id): Path<String>,
) -> Result<Json<SessionRuntimeSnapshot>, ApiError> {
    service
        .runtime_snapshot(&session_id)
        .map_err(ApiError::internal)?
        .map(Json)
        .ok_or_else(|| ApiError::not_found("session not found"))
}

async fn openapi() -> Json<utoipa::openapi::OpenApi> {
    Json(ApiDoc::openapi())
}

fn sse_event(event: SantiStreamEvent) -> Event {
    Event::default()
        .id(event.event_id.clone())
        .event(sse_event_name(&event.payload))
        .data(serde_json::to_string(&event).unwrap_or_else(|_| "{}".to_string()))
}

fn sse_event_name(payload: &SantiStreamPayload) -> &'static str {
    match payload {
        SantiStreamPayload::StreamOpen => "stream_open",
        SantiStreamPayload::MessageCreated { .. } => "message_created",
        SantiStreamPayload::MessageDelta { .. } => "message_delta",
        SantiStreamPayload::MessageCompleted { .. } => "message_completed",
        SantiStreamPayload::ToolCallCreated { .. } => "tool_call_created",
        SantiStreamPayload::ToolResultCreated { .. } => "tool_result_created",
        SantiStreamPayload::ThinkingCreated { .. } => "thinking_created",
        SantiStreamPayload::ThinkingUpdated { .. } => "thinking_updated",
        SantiStreamPayload::ThinkingCompleted { .. } => "thinking_completed",
        SantiStreamPayload::MaterialUpdated { .. } => "material_updated",
        SantiStreamPayload::TurnStarted { .. } => "turn_started",
        SantiStreamPayload::TurnActivity { .. } => "turn_activity",
        SantiStreamPayload::TurnFailed { .. } => "turn_failed",
    }
}

pub(crate) struct ApiError {
    status: StatusCode,
    code: &'static str,
    message: String,
}

impl ApiError {
    pub(crate) fn internal(message: String) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            code: "internal",
            message,
        }
    }

    pub(crate) fn not_found(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            code: "not-found",
            message: message.into(),
        }
    }

    pub(crate) fn bad_request(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            code: "bad-request",
            message: message.into(),
        }
    }

    pub(crate) fn unauthorized(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::UNAUTHORIZED,
            code: "unauthorized",
            message: message.into(),
        }
    }

    pub(crate) fn from_service(message: String) -> Self {
        match message.as_str() {
            "session not found" | "soul not found" => Self::not_found(message),
            _ if message.contains("object key")
                || message.contains("object uri")
                || message.contains("path segment")
                || message.contains("path separators") =>
            {
                Self::bad_request(message)
            }
            _ => Self::internal(message),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        (
            self.status,
            Json(ErrorResponse {
                code: self.code.to_string(),
                message: self.message,
            }),
        )
            .into_response()
    }
}

#[derive(OpenApi)]
#[openapi(
    paths(
        health,
        create_session,
        list_sessions,
        get_session,
        update_session,
        list_messages,
        session_material,
        send_session,
        runtime_snapshot,
        crate::bucket::get_bucket_object
    ),
    components(schemas(
        CreateSessionResponse,
        ErrorResponse,
        HealthResponse,
        MaterialRequest,
        SendSessionRequest,
        SendSessionAcceptedResponse,
        Session,
        SessionDetail,
        SessionMaterial,
        SessionProfile,
        SessionRuntimeSnapshot,
        SessionSummary,
        SoulProfile,
        UpdateSessionRequest,
        santi_core::ActorType,
        santi_core::Compact,
        santi_core::Message,
        santi_core::MessageContent,
        santi_core::MessagePart,
        santi_core::MessageState,
        santi_core::MaterialKind,
        santi_core::MaterialUpdated,
        santi_core::SessionEffect,
        santi_core::SessionMessage,
        santi_core::SessionMessageRef,
        santi_core::SoulSession,
        santi_core::ThinkingSpan,
        santi_core::ThinkingSpanState,
        santi_core::ToolCall,
        santi_core::ToolResult,
        santi_core::Turn,
        santi_core::TurnActivity,
        santi_core::TurnActivityState,
        santi_core::TurnStatus,
        santi_core::TurnTriggerType
    ))
)]
struct ApiDoc;
