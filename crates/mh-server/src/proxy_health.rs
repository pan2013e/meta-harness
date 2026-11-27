use std::{io::Write, sync::OnceLock, time::Duration};

use axum::Json;
use mh_core::outbound_proxy::{self, OutboundProxyStatus};
use tokio::sync::Mutex;
use tokio::time::{Instant, MissedTickBehavior};

static DESKTOP_CHILD: OnceLock<bool> = OnceLock::new();
static LAST_CHECK: Mutex<Option<Instant>> = Mutex::const_new(None);
const CHECK_INTERVAL: Duration = Duration::from_secs(60);

pub async fn check_at_startup(desktop_child: bool) {
    let _ = DESKTOP_CHILD.set(desktop_child);
    let _ = check().await;
}

pub async fn run_periodically() {
    if outbound_proxy::endpoint().is_none() {
        return;
    }
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
    if let Some(proxy) = outbound_proxy::endpoint() {
        let mut last = LAST_CHECK.lock().await;
        if last.is_none_or(|checked| checked.elapsed() >= Duration::from_secs(2)) {
            let online = proxy_reaches_internet(proxy).await;
            *last = Some(Instant::now());
            if outbound_proxy::record_health(online) && DESKTOP_CHILD.get() == Some(&true) {
                let state = if online { "enabled" } else { "direct" };
                let mut stdout = std::io::stdout().lock();
                let _ = writeln!(stdout, "MH_PROXY {state}").and_then(|()| stdout.flush());
            }
        }
    }
    status().await
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
