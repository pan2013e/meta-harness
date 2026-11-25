use axum::{
    Json,
    extract::{Request, State},
    http::{StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response},
};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::AppState;

#[derive(Debug, Clone)]
pub struct AuthConfig {
    pub required: bool,
    pub token: String,
}

impl AuthConfig {
    pub fn from_bind_and_flag(bind: &str, explicit_token: Option<String>) -> Self {
        let host = bind
            .rsplit_once(':')
            .map(|(h, _)| h)
            .unwrap_or(bind)
            .trim_matches(['[', ']']);
        let is_loopback = matches!(host, "127.0.0.1" | "::1" | "localhost");
        let required = !is_loopback || explicit_token.is_some();

        let token = explicit_token.unwrap_or_else(generate_random_hex_token);
        Self { required, token }
    }
}

pub fn generate_random_hex_token() -> String {
    let mut bytes = [0u8; 24];
    rand::thread_rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}

#[derive(Debug, Serialize)]
pub struct AuthStatusResponse {
    pub required: bool,
    pub authenticated: bool,
}

#[derive(Debug, Deserialize)]
pub struct AuthSessionRequest {
    pub token: String,
}

pub async fn auth_status(State(state): State<AppState>, req: Request) -> Json<AuthStatusResponse> {
    if !state.auth.required {
        return Json(AuthStatusResponse {
            required: false,
            authenticated: true,
        });
    }
    let ok = is_authorized(&req, &state.auth.token);
    Json(AuthStatusResponse {
        required: true,
        authenticated: ok,
    })
}

pub async fn create_auth_session(
    State(state): State<AppState>,
    Json(body): Json<AuthSessionRequest>,
) -> Response {
    if state.auth.required && body.token != state.auth.token {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "error": "invalid access token" })),
        )
            .into_response();
    }

    let cookie = format!(
        "mh_auth={}; Path=/; HttpOnly; SameSite=Lax",
        state.auth.token
    );
    (
        StatusCode::OK,
        [(header::SET_COOKIE, cookie)],
        Json(json!({ "authenticated": true })),
    )
        .into_response()
}

pub async fn require_user_auth(
    State(state): State<AppState>,
    req: Request,
    next: Next,
) -> Response {
    if !state.auth.required {
        return next.run(req).await;
    }

    let path = req.uri().path();
    if path == "/api/health" || path == "/api/auth/status" || path == "/api/auth/session" {
        return next.run(req).await;
    }

    if is_authorized(&req, &state.auth.token) {
        next.run(req).await
    } else {
        (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "error": "authentication required" })),
        )
            .into_response()
    }
}

pub fn is_authorized(req: &Request, expected: &str) -> bool {
    if let Some(auth_hdr) = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        && let Some(bearer) = auth_hdr.strip_prefix("Bearer ")
        && bearer == expected
    {
        return true;
    }

    if let Some(cookie_hdr) = req
        .headers()
        .get(header::COOKIE)
        .and_then(|v| v.to_str().ok())
    {
        for part in cookie_hdr.split(';') {
            let trimmed = part.trim();
            if let Some(val) = trimmed.strip_prefix("mh_auth=")
                && val == expected
            {
                return true;
            }
        }
    }

    if let Some(q) = req.uri().query() {
        for pair in q.split('&') {
            if let Some((k, v)) = pair.split_once('=')
                && k == "token"
                && v == expected
            {
                return true;
            }
        }
    }

    false
}
