use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode, header};
use santi_api::web_router;
use tower::util::ServiceExt;

async fn hit(path: &str) -> (StatusCode, Option<String>, Option<String>, String) {
    let app = web_router();
    let response = app
        .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let cache = response
        .headers()
        .get(header::CACHE_CONTROL)
        .map(|value| value.to_str().unwrap().to_string());
    let place = response
        .headers()
        .get(header::LOCATION)
        .map(|value| value.to_str().unwrap().to_string());
    let body = to_bytes(response.into_body(), 1_000_000).await.unwrap();
    (
        status,
        cache,
        place,
        String::from_utf8_lossy(&body).to_string(),
    )
}

async fn bundle() -> String {
    let (status, _, _, body) = hit("/").await;
    assert_eq!(status, StatusCode::OK);
    let start = body
        .find("./assets/")
        .expect("index references a hashed asset");
    let rest = &body[start + 2..];
    let end = rest.find('"').expect("asset reference closes");
    rest[..end].to_string()
}

#[tokio::test]
async fn root_serves_the_pane() {
    let (status, cache, _, body) = hit("/").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(cache.as_deref(), Some("no-cache"));
    assert!(body.contains("santi · window"));
}

#[tokio::test]
async fn hashed_assets_are_immutable_under_both_mounts() {
    let asset = bundle().await;
    for mount in ["", "/panel"] {
        let (status, cache, _, _) = hit(&format!("{mount}/{asset}")).await;
        assert_eq!(status, StatusCode::OK, "{mount}/{asset}");
        assert_eq!(
            cache.as_deref(),
            Some("public, max-age=31536000, immutable")
        );
    }
}

#[tokio::test]
async fn panel_redirects_before_assets_resolve() {
    let (status, _, place, _) = hit("/panel").await;
    assert_eq!(status, StatusCode::PERMANENT_REDIRECT);
    assert_eq!(place.as_deref(), Some("/panel/"));
}

#[tokio::test]
async fn panel_serves_the_same_pane() {
    let (status, cache, _, body) = hit("/panel/").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(cache.as_deref(), Some("no-cache"));
    assert!(body.contains("santi · window"));
}

#[tokio::test]
async fn unknown_api_paths_answer_json_404() {
    let (status, _, _, body) = hit("/api/nowhere").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(body.contains("unknown api path"));
    let (status, _, _, body) = hit("/api/v1/window/nowhere").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(body.contains("unknown api path"));
}

#[tokio::test]
async fn pathname_fallback_does_not_exist() {
    for path in [
        "/one/two",
        "/panel/one/two",
        "/nowhere.txt",
        "/panel/api/v1/health",
    ] {
        let (status, _, _, _) = hit(path).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{path}");
    }
}

#[tokio::test]
async fn traversal_never_reaches_the_bundle() {
    for path in ["/assets/../index.html", "/assets/%2e%2e/index.html"] {
        let (status, _, _, _) = hit(path).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{path}");
    }
}

#[tokio::test]
async fn the_bundle_is_self_contained() {
    let asset = bundle().await;
    let (_, _, _, script) = hit(&format!("/{asset}")).await;
    let stripped = script
        .replace("http://www.w3.org", "")
        .replace("https://react.dev/errors", "");
    assert!(!stripped.contains("http://"));
    assert!(!stripped.contains("https://"));
}
