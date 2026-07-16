use axum::{
    Router,
    extract::Path,
    http::{StatusCode, header},
    response::{IntoResponse, Redirect, Response},
    routing::get,
};

#[derive(rust_embed::Embed)]
#[folder = "../../web/dist"]
struct Dist;

pub(super) fn router<S: Clone + Send + Sync + 'static>() -> Router<S> {
    Router::new()
        .route("/", get(index))
        .route("/assets/{*rest}", get(asset))
        .route("/panel", get(panel))
        .route("/panel/", get(index))
        .route("/panel/assets/{*rest}", get(asset))
}

async fn index() -> Response {
    match Dist::get("index.html") {
        Some(file) => (
            [
                (header::CONTENT_TYPE, "text/html; charset=utf-8"),
                (header::CACHE_CONTROL, "no-cache"),
            ],
            file.data,
        )
            .into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn panel() -> Redirect {
    Redirect::permanent("/panel/")
}

async fn asset(Path(rest): Path<String>) -> Response {
    if rest
        .split('/')
        .any(|piece| piece.is_empty() || piece == "." || piece == "..")
    {
        return StatusCode::NOT_FOUND.into_response();
    }
    match Dist::get(&format!("assets/{rest}")) {
        Some(file) => (
            [
                (header::CONTENT_TYPE, kind(&rest)),
                (header::CACHE_CONTROL, "public, max-age=31536000, immutable"),
            ],
            file.data,
        )
            .into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

fn kind(path: &str) -> &'static str {
    match path.rsplit('.').next() {
        Some("js") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("svg") => "image/svg+xml",
        Some("map") | Some("json") => "application/json; charset=utf-8",
        Some("woff2") => "font/woff2",
        _ => "application/octet-stream",
    }
}
