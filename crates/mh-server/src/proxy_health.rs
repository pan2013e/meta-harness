use std::{io::Write, path::PathBuf, sync::OnceLock, time::Duration};

use axum::Json;
use axum::http::StatusCode;
use mh_core::outbound_proxy::{self, OutboundProxyStatus};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use tokio::time::{Instant, MissedTickBehavior};

static DESKTOP_CHILD: OnceLock<bool> = OnceLock::new();
static SETTINGS_PATH: OnceLock<PathBuf> = OnceLock::new();
static LAST_CHECK: Mutex<Option<Instant>> = Mutex::const_new(None);
const CHECK_INTERVAL: Duration = Duration::from_secs(60);

#[derive(Deserialize, Serialize)]
struct NetworkSettings {
    proxy: Option<String>,
}

pub fn initialize(
    cli_proxy: Option<&str>,
    settings_path: Option<PathBuf>,
) -> Result<(), &'static str> {
    let saved = settings_path.as_ref().and_then(|path| {
        let data = std::fs::read(path).ok()?;
        let settings: NetworkSettings = serde_json::from_slice(&data).ok()?;
        outbound_proxy::proxy_url(settings.proxy.as_deref()?)
    });
    let config = outbound_proxy::resolve(cli_proxy.or(saved.as_deref()))?;
    if let Some(path) = settings_path {
        let _ = SETTINGS_PATH.set(path);
    }
    outbound_proxy::activate(config, None, saved.is_some());
    Ok(())
}

pub async fn check_at_startup(desktop_child: bool) {
    let _ = DESKTOP_CHILD.set(desktop_child);
    if outbound_proxy::endpoint().is_some() {
        let _ = check().await;
    } else {
        announce_route();
    }
}

pub async fn run_periodically() {
    let mut timer = tokio::time::interval_at(Instant::now() + CHECK_INTERVAL, CHECK_INTERVAL);
    timer.set_missed_tick_behavior(MissedTickBehavior::Skip);
    loop {
        timer.tick().await;
        let _ = check().await;
    }
}

pub async fn status() -> Json<OutboundProxyStatus> {
    Json(outbound_proxy::status())
}

pub async fn check() -> Json<OutboundProxyStatus> {
    let mut last = LAST_CHECK.lock().await;
    if let Some(proxy) = outbound_proxy::endpoint()
        && last.is_none_or(|checked| checked.elapsed() >= Duration::from_secs(2))
    {
        let online = proxy_reaches_internet(&proxy).await;
        *last = Some(Instant::now());
        if outbound_proxy::record_health(online) {
            announce_route();
        }
    }
    status().await
}

#[derive(Deserialize)]
pub struct UpdateProxy {
    proxy: Option<String>,
    #[serde(default)]
    allow_unreachable: bool,
}

#[derive(Serialize)]
pub struct ProxyUpdateResult {
    saved: bool,
    status: OutboundProxyStatus,
}

pub async fn update(
    Json(request): Json<UpdateProxy>,
) -> crate::routes::ApiResult<ProxyUpdateResult> {
    let path = SETTINGS_PATH.get().ok_or_else(|| {
        crate::routes::err_resp(
            StatusCode::METHOD_NOT_ALLOWED,
            "Set the standalone server proxy with --proxy.",
        )
    })?;
    let proxy = request
        .proxy
        .as_deref()
        .map(|url| {
            outbound_proxy::proxy_url(url).ok_or_else(|| {
                crate::routes::err_resp(
                    StatusCode::UNPROCESSABLE_ENTITY,
                    "Enter a valid HTTP, HTTPS, or SOCKS5 proxy URL without a path.",
                )
            })
        })
        .transpose()?;
    let candidate = outbound_proxy::resolve(proxy.as_deref())
        .map_err(|message| crate::routes::err_resp(StatusCode::UNPROCESSABLE_ENTITY, message))?;
    let mut last = LAST_CHECK.lock().await;
    let reachable = match candidate.endpoint() {
        Some(url) => Some(proxy_reaches_internet(url).await),
        None => None,
    };
    if proxy.is_some() && reachable == Some(false) && !request.allow_unreachable {
        return Ok(Json(ProxyUpdateResult {
            saved: false,
            status: outbound_proxy::status(),
        }));
    }
    let desktop_override = proxy.is_some();
    save_settings(path, NetworkSettings { proxy }).map_err(|_| {
        crate::routes::err_resp(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Could not save the desktop proxy settings.",
        )
    })?;
    outbound_proxy::activate(candidate, reachable, desktop_override);
    *last = reachable.map(|_| Instant::now());
    announce_route();
    Ok(Json(ProxyUpdateResult {
        saved: true,
        status: outbound_proxy::status(),
    }))
}

fn save_settings(path: &std::path::Path, settings: NetworkSettings) -> std::io::Result<()> {
    let parent = path.parent().ok_or(std::io::ErrorKind::InvalidInput)?;
    std::fs::create_dir_all(parent)?;
    let mut staged = tempfile::NamedTempFile::new_in(parent)?;
    serde_json::to_writer(staged.as_file_mut(), &settings)?;
    staged.persist(path)?;
    Ok(())
}

fn announce_route() {
    if DESKTOP_CHILD.get() != Some(&true) {
        return;
    }
    let state = outbound_proxy::status();
    let route = if state.proxy_not_used {
        "direct"
    } else if state.host.is_some() {
        "enabled"
    } else {
        "automatic"
    };
    let mut stdout = std::io::stdout().lock();
    let _ = writeln!(stdout, "MH_PROXY {route}").and_then(|()| stdout.flush());
}

pub async fn proxy_reaches_internet(proxy: &str) -> bool {
    let Ok(proxy) = reqwest::Proxy::all(proxy) else {
        return false;
    };
    let Ok(client) = reqwest::Client::builder()
        .proxy(proxy)
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(4))
        .build()
    else {
        return false;
    };
    client
        .get("https://www.gstatic.com/generate_204")
        .send()
        .await
        .is_ok_and(|response| response.status() == reqwest::StatusCode::NO_CONTENT)
}
