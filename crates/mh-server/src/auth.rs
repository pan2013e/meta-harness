use std::{
    net::{IpAddr, SocketAddr},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use axum::{
    Json,
    extract::{
        Request, State,
        ws::{CloseFrame, Message, WebSocket, close_code},
    },
    http::{StatusCode, Uri, header, uri::Authority},
    middleware::Next,
    response::{IntoResponse, Response},
};
use rand::{RngCore, rngs::OsRng};
use ring::hmac;
use serde::{Deserialize, Serialize};
use serde_json::json;
use subtle::ConstantTimeEq;

use crate::AppState;

const SESSION_LIFETIME_SECS: u64 = 24 * 60 * 60;

#[derive(Clone, Copy)]
pub(crate) struct SessionAccess(Option<u64>);

impl SessionAccess {
    pub(crate) async fn expires(self) {
        if let Some(expires) = self.0 {
            tokio::time::sleep(Duration::from_secs(expires.saturating_sub(now()))).await;
        } else {
            std::future::pending::<()>().await;
        }
    }
}

pub(crate) async fn close_expired_socket(socket: &mut WebSocket) {
    let close = socket.send(Message::Close(Some(CloseFrame {
        code: close_code::POLICY,
        reason: "Authentication expired".into(),
    })));
    let _ = tokio::time::timeout(Duration::from_secs(1), close).await;
}

#[derive(Clone)]
pub struct AuthConfig {
    pub required: bool,
    pub token: String,
    tls: bool,
    cookie_name: String,
    cookie_key: hmac::Key,
}

impl AuthConfig {
    pub fn for_listener(
        address: SocketAddr,
        tls: bool,
        explicit_token: Option<String>,
    ) -> Result<Self, &'static str> {
        let is_loopback = address.ip().is_loopback();
        if !is_loopback && !tls {
            return Err(
                "LAN listeners require HTTPS. Set --tls-cert and --tls-key, or use an HTTPS reverse proxy with a loopback bind.",
            );
        }
        if explicit_token.as_ref().is_some_and(|token| {
            !(24..=256).contains(&token.len()) || !token.bytes().all(|byte| byte.is_ascii_graphic())
        }) {
            return Err(
                "Use an access token with 24 to 256 printable ASCII characters without spaces, or let Meta-Harness generate one.",
            );
        }
        let mut key = [0u8; 32];
        OsRng.fill_bytes(&mut key);
        Ok(Self {
            required: !is_loopback || explicit_token.is_some(),
            token: explicit_token.unwrap_or_else(generate_random_hex_token),
            tls,
            cookie_name: format!("mh_session_{}", address.port()),
            cookie_key: hmac::Key::new(hmac::HMAC_SHA256, &key),
        })
    }

    pub(crate) fn authenticated(&self, req: &Request) -> bool {
        self.access(req).is_some()
    }

    fn access(&self, req: &Request) -> Option<SessionAccess> {
        if !self.required {
            return Some(SessionAccess(None));
        }
        if req
            .headers()
            .get(header::AUTHORIZATION)
            .and_then(|header| header.to_str().ok())
            .and_then(|header| header.strip_prefix("Bearer "))
            .is_some_and(|candidate| tokens_equal(candidate, &self.token))
        {
            return Some(SessionAccess(None));
        }
        let authority = request_authority(req)?;
        let current = now();
        req.headers()
            .get_all(header::COOKIE)
            .iter()
            .filter_map(|header| header.to_str().ok())
            .flat_map(|header| header.split(';'))
            .filter_map(|part| part.trim().split_once('='))
            .filter(|(name, _)| *name == self.cookie_name)
            .find_map(|(_, value)| self.cookie_expiration(&authority, value, current))
            .map(|expires| SessionAccess(Some(expires)))
    }

    fn signed_cookie(&self, authority: &Authority, expires: u64) -> String {
        let signature = hmac::sign(&self.cookie_key, &cookie_scope(authority, expires));
        format!("{expires}.{}", hex::encode(signature.as_ref()))
    }

    fn cookie_expiration(&self, authority: &Authority, value: &str, current: u64) -> Option<u64> {
        let (expires, signature) = value.split_once('.')?;
        let expires = expires.parse::<u64>().ok()?;
        if expires <= current || signature.len() != 64 {
            return None;
        }
        let signature = hex::decode(signature).ok()?;
        hmac::verify(
            &self.cookie_key,
            &cookie_scope(authority, expires),
            &signature,
        )
        .ok()?;
        Some(expires)
    }
}

fn cookie_scope(authority: &Authority, expires: u64) -> Vec<u8> {
    format!("{}\n{expires}", authority.as_str().to_ascii_lowercase()).into_bytes()
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub(crate) fn tokens_equal(candidate: &str, expected: &str) -> bool {
    bool::from(candidate.as_bytes().ct_eq(expected.as_bytes()))
}

pub fn generate_random_hex_token() -> String {
    let mut bytes = [0u8; 24];
    OsRng.fill_bytes(&mut bytes);
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
    Json(AuthStatusResponse {
        required: state.auth.required,
        authenticated: state.auth.authenticated(&req),
    })
}

pub async fn create_auth_session(
    State(state): State<AppState>,
    request_headers: header::HeaderMap,
    uri: Uri,
    Json(body): Json<AuthSessionRequest>,
) -> Response {
    if !state.auth.required {
        return Json(json!({ "authenticated": true })).into_response();
    }
    if !tokens_equal(&body.token, &state.auth.token) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "error": "invalid access token" })),
        )
            .into_response();
    }
    let Some(authority) = authority_from(&uri, &request_headers) else {
        return forbidden_origin();
    };
    // A trusted HTTPS reverse proxy can retain the browser's HTTPS Origin on a loopback backend.
    let secure = state.auth.tls
        || request_headers
            .get(header::ORIGIN)
            .and_then(|origin| origin.to_str().ok())
            .and_then(|origin| origin.parse::<Uri>().ok())
            .is_some_and(|origin| origin.scheme_str() == Some("https"));
    let session = state
        .auth
        .signed_cookie(&authority, now() + SESSION_LIFETIME_SECS);
    let cookie = format!(
        "{}={session}; Path=/api/; HttpOnly; SameSite=Strict{}",
        state.auth.cookie_name,
        if secure { "; Secure" } else { "" }
    );
    (
        [(header::SET_COOKIE, cookie)],
        Json(json!({ "authenticated": true })),
    )
        .into_response()
}

pub async fn require_user_auth(
    State(state): State<AppState>,
    mut req: Request,
    next: Next,
) -> Response {
    if !request_matches_app_origin(&req, state.auth.required, state.auth.tls) {
        return forbidden_origin();
    }
    let path = req.uri().path();
    if matches!(
        path,
        "/api/health" | "/api/auth/status" | "/api/auth/session"
    ) {
        return next.run(req).await;
    }
    if let Some(access) = state.auth.access(&req) {
        req.extensions_mut().insert(access);
        return next.run(req).await;
    }
    (
        StatusCode::UNAUTHORIZED,
        Json(json!({ "error": "authentication required" })),
    )
        .into_response()
}

fn forbidden_origin() -> Response {
    (
        StatusCode::FORBIDDEN,
        Json(json!({ "error": "request origin does not match Meta-Harness" })),
    )
        .into_response()
}

fn request_authority(req: &Request) -> Option<Authority> {
    authority_from(req.uri(), req.headers())
}

fn authority_from(uri: &Uri, headers: &header::HeaderMap) -> Option<Authority> {
    let mut hosts = headers.get_all(header::HOST).iter();
    let supplied = match hosts.next() {
        Some(value) => Some(value.to_str().ok()?.parse::<Authority>().ok()?),
        None => None,
    };
    if hosts.next().is_some() {
        return None;
    }
    match (uri.authority(), supplied) {
        (Some(authority), Some(host))
            if !authority.as_str().eq_ignore_ascii_case(host.as_str()) =>
        {
            None
        }
        (Some(authority), _) => Some(authority.clone()),
        (None, supplied) => supplied,
    }
    .filter(|authority| !authority.as_str().contains('@'))
}

fn request_matches_app_origin(req: &Request, auth_required: bool, tls: bool) -> bool {
    let Some(host) = request_authority(req) else {
        return false;
    };
    let hostname = host.host().trim_matches(['[', ']']);
    if !auth_required
        && !hostname.eq_ignore_ascii_case("localhost")
        && !hostname.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback())
    {
        return false;
    }
    if req
        .headers()
        .get("sec-fetch-site")
        .is_some_and(|site| site == "cross-site" || site == "same-site")
    {
        return false;
    }
    let mut origins = req.headers().get_all(header::ORIGIN).iter();
    let Some(value) = origins.next() else {
        return true;
    };
    if origins.next().is_some() {
        return false;
    }
    let origin = value
        .to_str()
        .ok()
        .and_then(|value| value.parse::<Uri>().ok());
    origin.is_some_and(|origin| {
        let default_port = match origin.scheme_str() {
            Some("https") => 443,
            Some("http") if !tls => 80,
            _ => return false,
        };
        origin
            .path_and_query()
            .is_none_or(|part| part.as_str() == "/")
            && origin.authority().is_some_and(|other| {
                !other.as_str().contains('@')
                    && other.host().eq_ignore_ascii_case(host.host())
                    && other.port_u16().unwrap_or(default_port)
                        == host.port_u16().unwrap_or(default_port)
            })
    })
}

pub(crate) async fn browser_response_headers(req: Request, next: Next) -> Response {
    let sensitive = req.uri().path().starts_with("/api/") || req.uri().path() == "/";
    let mut response = next.run(req).await;
    let headers = response.headers_mut();
    headers.insert(
        header::REFERRER_POLICY,
        header::HeaderValue::from_static("no-referrer"),
    );
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        header::HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        header::X_FRAME_OPTIONS,
        header::HeaderValue::from_static("SAMEORIGIN"),
    );
    if sensitive {
        headers.insert(
            header::CACHE_CONTROL,
            header::HeaderValue::from_static("private, no-store"),
        );
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lan_requires_tls_and_explicit_tokens_cannot_be_weak_or_empty() {
        let lan = "0.0.0.0:4410".parse().unwrap();
        assert!(AuthConfig::for_listener(lan, false, None).is_err());
        assert!(AuthConfig::for_listener(lan, true, Some(String::new())).is_err());
        assert!(AuthConfig::for_listener(lan, true, Some("short-token".into())).is_err());
        assert!(AuthConfig::for_listener(lan, true, None).unwrap().required);
    }

    #[test]
    fn browser_origins_must_match_scheme_and_authority() {
        for (host, origin, auth_required, tls, allowed) in [
            (
                "localhost:5173",
                Some("http://localhost:5173"),
                false,
                false,
                true,
            ),
            ("127.0.0.1:4410", None, false, false, true),
            ("[::1]:4410", Some("http://[::1]:4410"), false, false, true),
            (
                "127.0.0.1:4410",
                Some("http://example.invalid"),
                false,
                false,
                false,
            ),
            ("127.0.0.1:4410", Some("null"), false, false, false),
            ("example.invalid", None, false, false, false),
            (
                "harness.example",
                Some("https://harness.example"),
                true,
                true,
                true,
            ),
            (
                "harness.example:443",
                Some("https://harness.example"),
                true,
                true,
                true,
            ),
            (
                "harness.example",
                Some("http://harness.example"),
                true,
                true,
                false,
            ),
            (
                "harness.example",
                Some("https://example.invalid"),
                true,
                true,
                false,
            ),
        ] {
            let mut req = Request::builder().uri("/api/ws").header(header::HOST, host);
            if let Some(origin) = origin {
                req = req.header(header::ORIGIN, origin);
            }
            assert_eq!(
                request_matches_app_origin(
                    &req.body(axum::body::Body::empty()).unwrap(),
                    auth_required,
                    tls
                ),
                allowed
            );
        }
    }

    #[test]
    fn sessions_are_expiring_origin_scoped_credentials() {
        let config = AuthConfig::for_listener(
            "127.0.0.1:4410".parse().unwrap(),
            false,
            Some(generate_random_hex_token()),
        )
        .unwrap();
        let host = "example.test:4410".parse().unwrap();
        let cookie = config.signed_cookie(&host, 120);
        assert!(!cookie.contains(&config.token));
        assert_eq!(config.cookie_expiration(&host, &cookie, 119), Some(120));
        assert_eq!(config.cookie_expiration(&host, &cookie, 120), None);
        assert_eq!(
            config.cookie_expiration(&"example.test:5000".parse().unwrap(), &cookie, 119),
            None
        );
    }

    #[tokio::test]
    async fn cookie_authentication_carries_a_deadline_for_upgraded_connections() {
        let config = AuthConfig::for_listener(
            "127.0.0.1:4410".parse().unwrap(),
            false,
            Some(generate_random_hex_token()),
        )
        .unwrap();
        let host: Authority = "127.0.0.1:4410".parse().unwrap();
        let expires = now() + 2;
        let cookie = config.signed_cookie(&host, expires);
        let request = Request::builder()
            .uri("/api/ws")
            .header(header::HOST, host.as_str())
            .header(header::COOKIE, format!("{}={cookie}", config.cookie_name))
            .body(axum::body::Body::empty())
            .unwrap();
        let access = config.access(&request).unwrap();
        assert_eq!(access.0, Some(expires));
        tokio::time::timeout(Duration::from_secs(3), access.expires())
            .await
            .unwrap();
        assert!(!config.authenticated(&request));
    }
}
