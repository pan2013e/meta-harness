use std::{
    path::Path as FilePath,
    sync::OnceLock,
    time::{SystemTime, UNIX_EPOCH},
};

use axum::{
    extract::{Path, State},
    http::{Method, StatusCode, header},
    response::{IntoResponse, Response},
};
use ring::hmac;

use crate::{
    AppState,
    auth::generate_random_hex_token,
    routes::{FileAccess, read_session_worktree_file},
};

const VALID_FOR_SECS: u64 = 30 * 60;
pub(crate) const GRANT_HEADER: &str = "x-meta-harness-preview";

pub(crate) fn issue(session_id: &str, entry: &str) -> String {
    let expires = now() + VALID_FOR_SECS;
    let entry = hex::encode(entry.as_bytes());
    let tag = hmac::sign(key(), &signed_value(session_id, expires, &entry));
    format!("{expires}.{entry}.{}", hex::encode(tag.as_ref()))
}

pub async fn read(
    State(state): State<AppState>,
    Path((session_id, token, file)): Path<(String, String, String)>,
    request_headers: header::HeaderMap,
    method: Method,
) -> Response {
    let Some(entry) = verify(&session_id, &token) else {
        return (StatusCode::NOT_FOUND, "Preview expired. Reopen the file.").into_response();
    };

    let mut response = read_session_worktree_file(
        &state,
        &session_id,
        &file,
        FileAccess::SharedPreview { entry: &entry },
        method,
    )
    .await;
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
    if request_headers
        .get(header::ORIGIN)
        .is_some_and(|origin| origin == "null")
    {
        headers.insert(
            header::ACCESS_CONTROL_ALLOW_ORIGIN,
            header::HeaderValue::from_static("null"),
        );
    }
    response
}

pub(crate) fn permits_file(entry: &FilePath, file: &FilePath, mime: &str) -> bool {
    if file == entry && mime.starts_with("text/html") {
        return true;
    }
    if file
        .components()
        .any(|part| part.as_os_str().to_string_lossy().starts_with('.'))
    {
        return false;
    }
    if mime.starts_with("text/html") {
        return entry
            .parent()
            .is_some_and(|directory| file.starts_with(directory));
    }
    [
        "text/css",
        "application/javascript",
        "image/",
        "font/",
        "audio/",
        "video/",
        "application/wasm",
    ]
    .iter()
    .any(|prefix| mime.starts_with(prefix))
}

fn verify(session_id: &str, token: &str) -> Option<String> {
    let (expiry, rest) = token.split_once('.')?;
    let (entry, tag) = rest.split_once('.')?;
    let expires = expiry.parse::<u64>().ok()?;
    if expires <= now() || tag.len() != 64 || entry.is_empty() || entry.len() > 8192 {
        return None;
    }
    let signature = hex::decode(tag).ok()?;
    hmac::verify(key(), &signed_value(session_id, expires, entry), &signature).ok()?;
    String::from_utf8(hex::decode(entry).ok()?).ok()
}

fn signed_value(session_id: &str, expires: u64, entry: &str) -> Vec<u8> {
    format!("{session_id}\n{expires}\n{entry}").into_bytes()
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
        let entry = hex::encode("site/index.html");
        let grant = issue("s_first", "site/index.html");
        let (_, tail) = grant.split_once('.').unwrap();
        assert_eq!(
            verify("s_first", &grant).as_deref(),
            Some("site/index.html")
        );
        assert!(verify("s_other", &grant).is_none());
        assert!(verify("s_first", &format!("{}.{tail}", now() + VALID_FOR_SECS + 1)).is_none());
        let expired = now().saturating_sub(1);
        let signature = hmac::sign(key(), &signed_value("s_first", expired, &entry));
        assert!(
            verify(
                "s_first",
                &format!("{expired}.{entry}.{}", hex::encode(signature.as_ref()))
            )
            .is_none()
        );
    }

    #[test]
    fn preview_scope_keeps_static_assets_and_excludes_private_files() {
        let entry = FilePath::new("site/index.html");
        for (path, mime, allowed) in [
            ("site/index.html", "text/html", true),
            ("site/about.html", "text/html", true),
            ("styles/site.css", "text/css", true),
            ("private/notes.html", "text/html", false),
            (".private/config.js", "application/javascript", false),
            (".env", "application/octet-stream", false),
            ("site/settings.json", "application/json", false),
        ] {
            assert_eq!(permits_file(entry, FilePath::new(path), mime), allowed);
        }
    }
}
