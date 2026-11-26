use std::{
    sync::OnceLock,
    time::{SystemTime, UNIX_EPOCH},
};

use axum::{
    extract::{Path, State},
    http::{Method, StatusCode, header},
    response::{IntoResponse, Response},
};
use ring::hmac;

use crate::{AppState, auth::generate_random_hex_token, routes::read_session_worktree_file};

const VALID_FOR_SECS: u64 = 30 * 60;
pub(crate) const GRANT_HEADER: &str = "x-meta-harness-preview";

pub(crate) fn issue(session_id: &str) -> String {
    let expires = now() + VALID_FOR_SECS;
    let tag = hmac::sign(key(), &signed_value(session_id, expires));
    format!("{expires}.{}", hex::encode(tag.as_ref()))
}

pub async fn read(
    State(state): State<AppState>,
    Path((session_id, token, file)): Path<(String, String, String)>,
    method: Method,
) -> Response {
    if !verify(&session_id, &token) {
        return (StatusCode::NOT_FOUND, "Preview expired. Reopen the file.").into_response();
    }

    let mut response = read_session_worktree_file(&state, &session_id, &file, true, method).await;
    let headers = response.headers_mut();
    headers.insert(
        header::REFERRER_POLICY,
        header::HeaderValue::from_static("no-referrer"),
    );
    headers.insert(
        header::CACHE_CONTROL,
        header::HeaderValue::from_static("private, no-store"),
    );
    // The opaque preview origin needs CORS for local ES modules and fonts.
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_ORIGIN,
        header::HeaderValue::from_static("*"),
    );
    response
}

fn verify(session_id: &str, token: &str) -> bool {
    let Some((expiry, tag)) = token.split_once('.') else {
        return false;
    };
    let Ok(expires) = expiry.parse::<u64>() else {
        return false;
    };
    if expires <= now() || tag.len() != 64 {
        return false;
    }
    hex::decode(tag).is_ok_and(|signature| {
        hmac::verify(key(), &signed_value(session_id, expires), &signature).is_ok()
    })
}

fn signed_value(session_id: &str, expires: u64) -> Vec<u8> {
    format!("{session_id}\n{expires}").into_bytes()
}

fn key() -> &'static hmac::Key {
    static KEY: OnceLock<hmac::Key> = OnceLock::new();
    KEY.get_or_init(|| hmac::Key::new(hmac::HMAC_SHA256, generate_random_hex_token().as_bytes()))
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_grant_is_bound_to_the_session_and_expiry() {
        let grant = issue("s_first");
        let (_, signature) = grant.split_once('.').unwrap();
        assert!(verify("s_first", &grant));
        assert!(!verify("s_other", &grant));
        assert!(!verify(
            "s_first",
            &format!("{}.{signature}", now() + VALID_FOR_SECS + 1)
        ));
        let expired = now().saturating_sub(1);
        let signature = hmac::sign(key(), &signed_value("s_first", expired));
        assert!(!verify(
            "s_first",
            &format!("{expired}.{}", hex::encode(signature.as_ref()))
        ));
    }
}
