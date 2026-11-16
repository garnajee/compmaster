use crate::AppState;
use axum::{
    extract::{Request, State},
    http::{HeaderValue, Method, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};
use std::sync::Arc;

pub fn new_csrf_token() -> String {
    format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}

pub async fn guard(State(state): State<Arc<AppState>>, request: Request, next: Next) -> Response {
    let expected = state.config.public_origin.trim_end_matches('/');
    let authority = expected
        .split_once("://")
        .map(|(_, h)| h)
        .unwrap_or_default();
    let host = request
        .headers()
        .get("host")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    if host != authority {
        return (StatusCode::FORBIDDEN, "Host non autorisé").into_response();
    }
    if let Some(origin) = request
        .headers()
        .get("origin")
        .and_then(|v| v.to_str().ok())
    {
        if origin != expected {
            return (StatusCode::FORBIDDEN, "Origin non autorisée").into_response();
        }
    }
    if !matches!(
        *request.method(),
        Method::GET | Method::HEAD | Method::OPTIONS
    ) {
        if request
            .headers()
            .get("x-csrf-token")
            .and_then(|v| v.to_str().ok())
            != Some(&state.csrf_token)
        {
            return (StatusCode::FORBIDDEN, "Jeton CSRF manquant ou invalide").into_response();
        }
    }
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    headers.insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    headers.insert("referrer-policy", HeaderValue::from_static("no-referrer"));
    headers.insert("content-security-policy", HeaderValue::from_static("default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; connect-src 'self'; img-src 'self' data:; frame-ancestors 'none'; base-uri 'none'; form-action 'self'"));
    headers.insert("cache-control", HeaderValue::from_static("no-store"));
    response
}
