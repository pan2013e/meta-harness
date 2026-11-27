use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use url::Url;

static PROXY: Mutex<Option<OutboundProxy>> = Mutex::new(None);

pub struct OutboundProxy {
    variables: Vec<(String, String)>,
    endpoint: Option<String>,
    host: Option<String>,
    force_direct: bool,
    health: Health,
    desktop_override: bool,
}

#[derive(Default)]
struct Health {
    checked_at: Option<u64>,
    reachable: bool,
    generation: u64,
}

#[derive(Serialize)]
pub struct OutboundProxyStatus {
    pub host: Option<String>,
    pub checked_at: Option<u64>,
    pub proxy_not_used: bool,
    pub desktop_override: bool,
    pub generation: u64,
}

pub fn endpoint() -> Option<String> {
    PROXY
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()?
        .endpoint
        .clone()
}

pub fn status() -> OutboundProxyStatus {
    let proxy = PROXY.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(proxy) = proxy.as_ref() {
        return OutboundProxyStatus {
            host: proxy.host.clone(),
            checked_at: proxy.health.checked_at,
            proxy_not_used: proxy.is_direct(),
            desktop_override: proxy.desktop_override,
            generation: proxy.health.generation,
        };
    }
    OutboundProxyStatus {
        host: None,
        checked_at: None,
        proxy_not_used: false,
        desktop_override: false,
        generation: 0,
    }
}

pub fn activate(mut next: OutboundProxy, reachable: Option<bool>, desktop_override: bool) {
    let mut current = PROXY.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(reachable) = reachable {
        next.record_health(reachable);
    }
    next.health.generation = current
        .as_ref()
        .map_or(1, |proxy| proxy.health.generation + 1);
    next.desktop_override = desktop_override;
    *current = Some(next);
}

pub fn record_health(reachable: bool) -> bool {
    let mut proxy = PROXY.lock().unwrap_or_else(|e| e.into_inner());
    proxy
        .as_mut()
        .is_some_and(|proxy| proxy.record_health(reachable))
}

pub fn environment() -> Vec<(String, String)> {
    PROXY
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .map_or_else(Vec::new, OutboundProxy::environment)
}

pub fn generation() -> u64 {
    PROXY
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .map_or(0, |proxy| proxy.health.generation)
}

impl OutboundProxy {
    pub fn endpoint(&self) -> Option<&str> {
        self.endpoint.as_deref()
    }

    fn record_health(&mut self, reachable: bool) -> bool {
        let health = &mut self.health;
        let changed = health.checked_at.is_none() || health.reachable != reachable;
        if changed {
            health.generation += 1;
        }
        health.reachable = reachable;
        health.checked_at = Some(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
        );
        changed
    }

    fn is_direct(&self) -> bool {
        self.force_direct || (self.endpoint.is_some() && !self.health.reachable)
    }

    fn environment(&self) -> Vec<(String, String)> {
        if self.is_direct() {
            return direct_variables();
        }
        self.variables.clone()
    }
}

pub fn resolve(override_url: Option<&str>) -> Result<OutboundProxy, &'static str> {
    let force_direct = override_url.is_none()
        && std::env::var_os("MH_OUTBOUND_PROXY_DIRECT").as_deref()
            == Some(std::ffi::OsStr::new("1"));
    let read = |key| {
        std::env::var(key)
            .ok()
            .filter(|value| !value.trim().is_empty())
    };
    let env = |upper, lower| read(upper).or_else(|| read(lower));
    let configured = read("MH_OUTBOUND_PROXY");
    let explicit = override_url
        .or(configured.as_deref())
        .filter(|_| !force_direct)
        .map(|value| {
            proxy_url(value)
                .ok_or("The outbound proxy must be an HTTP, HTTPS, or SOCKS5 proxy URL.")
        })
        .transpose()?;
    let all = env("ALL_PROXY", "all_proxy");
    #[cfg(windows)]
    let system = windows_proxy();
    #[cfg(not(windows))]
    let system: Option<(String, String)> = None;
    let proxy_for = |upper, lower, scheme| {
        explicit
            .clone()
            .or_else(|| env(upper, lower))
            .or_else(|| all.clone())
            .or_else(|| {
                system
                    .as_ref()
                    .and_then(|(settings, _)| system_proxy_for(settings, scheme))
            })
    };
    let http = proxy_for("HTTP_PROXY", "http_proxy", "http");
    let https = proxy_for("HTTPS_PROXY", "https_proxy", "https");
    let bypass = env("NO_PROXY", "no_proxy").unwrap_or_else(|| {
        system
            .as_ref()
            .map(|(_, bypass)| {
                bypass
                    .split(';')
                    .map(str::trim)
                    .filter(|name| !name.starts_with('<'))
                    .collect::<Vec<_>>()
                    .join(",")
            })
            .unwrap_or_default()
    });
    let host = explicit.as_deref().and_then(|value| {
        let url = Url::parse(value).ok()?;
        Some(match url.port() {
            Some(port) => format!("{}:{port}", url.host()?),
            None => url.host()?.to_string(),
        })
    });
    Ok(OutboundProxy {
        variables: proxy_variables(http, https, &bypass),
        endpoint: explicit,
        host,
        force_direct,
        health: Health::default(),
        desktop_override: false,
    })
}

fn direct_variables() -> Vec<(String, String)> {
    let mut values = proxy_variables(Some(String::new()), Some(String::new()), "*");
    values.push(("ALL_PROXY".into(), String::new()));
    #[cfg(not(windows))]
    values.push(("all_proxy".into(), String::new()));
    values
}

fn proxy_variables(
    http: Option<String>,
    https: Option<String>,
    bypass: &str,
) -> Vec<(String, String)> {
    if http.is_none() && https.is_none() {
        return Vec::new();
    }
    let mut values = Vec::new();
    let mut set = |key: &str, value: String| {
        #[cfg(not(windows))]
        values.push((key.to_ascii_lowercase(), value.clone()));
        values.push((key.to_owned(), value));
    };
    if let Some(proxy) = http {
        set("HTTP_PROXY", proxy);
    }
    if let Some(proxy) = https {
        set("HTTPS_PROXY", proxy);
    }
    let mut excluded = bypass
        .split(',')
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .collect::<Vec<_>>();
    for loopback in ["localhost", "127.0.0.1", "::1"] {
        if !excluded.contains(&loopback) {
            excluded.push(loopback);
        }
    }
    set("NO_PROXY", excluded.join(","));
    values
}

pub fn proxy_url(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() || value.len() > 2048 || value.contains(char::is_whitespace) {
        return None;
    }
    let url = if value.contains("://") {
        Url::parse(value)
    } else {
        Url::parse(&format!("http://{value}"))
    }
    .ok()?;
    (matches!(url.scheme(), "http" | "https" | "socks5" | "socks5h")
        && url.has_host()
        && url.query().is_none()
        && url.fragment().is_none()
        && matches!(url.path(), "" | "/"))
    .then(|| url.to_string())
}

fn system_proxy_for(settings: &str, scheme: &str) -> Option<String> {
    if settings.contains('=') {
        settings
            .split(';')
            .filter_map(|entry| entry.split_once('='))
            .find(|(entry_scheme, _)| entry_scheme.trim().eq_ignore_ascii_case(scheme))
            .and_then(|(_, proxy)| proxy_url(proxy))
    } else {
        proxy_url(settings.split(';').next()?)
    }
}

#[cfg(windows)]
fn windows_proxy() -> Option<(String, String)> {
    use windows_sys::Win32::{
        Foundation::GlobalFree,
        Networking::WinHttp::{
            WINHTTP_CURRENT_USER_IE_PROXY_CONFIG, WinHttpGetIEProxyConfigForCurrentUser,
        },
    };
    fn take_wide(ptr: *mut u16) -> String {
        if ptr.is_null() {
            return String::new();
        }
        // SAFETY: the Windows API returns an owned, NUL-terminated UTF-16 allocation.
        unsafe {
            let len = (0..).take_while(|&i| *ptr.add(i) != 0).count();
            let value = String::from_utf16_lossy(std::slice::from_raw_parts(ptr, len));
            GlobalFree(ptr.cast());
            value
        }
    }
    // SAFETY: the WinHTTP input/output struct contains only raw pointers and a BOOL.
    let mut config: WINHTTP_CURRENT_USER_IE_PROXY_CONFIG = unsafe { std::mem::zeroed() };
    let result = unsafe { WinHttpGetIEProxyConfigForCurrentUser(&mut config) };
    let proxy = take_wide(config.lpszProxy);
    let bypass = take_wide(config.lpszProxyBypass);
    let _ = take_wide(config.lpszAutoConfigUrl);
    (result != 0 && !proxy.is_empty()).then_some((proxy, bypass))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selects_the_manual_windows_proxy_for_https() {
        assert_eq!(
            system_proxy_for("127.0.0.1:7890", "https").as_deref(),
            Some("http://127.0.0.1:7890/")
        );
        assert_eq!(
            system_proxy_for("http=proxy.local:123;https=proxy.local:456", "https").as_deref(),
            Some("http://proxy.local:456/")
        );
        assert_eq!(system_proxy_for("http=proxy.local:123", "https"), None);
        assert_eq!(proxy_url("ftp://proxy.local:123"), None);
        assert_eq!(proxy_url("http://proxy.local:8080/path"), None);
        assert_eq!(proxy_url(""), None);
        assert_eq!(
            proxy_url("socks5h://proxy.local:1080").as_deref(),
            Some("socks5h://proxy.local:1080")
        );
    }

    #[test]
    fn keeps_local_endpoints_and_custom_exclusions_direct() {
        let values = proxy_variables(None, Some("http://proxy.local:123".into()), ".example.test");
        assert!(values.contains(&(
            "NO_PROXY".into(),
            ".example.test,localhost,127.0.0.1,::1".into()
        )));
    }

    #[test]
    fn route_falls_back_on_failure_and_recovers_on_success() {
        let mut proxy = resolve(Some("http://account:secret@proxy.local:8080")).unwrap();
        assert_eq!(proxy.host.as_deref(), Some("proxy.local:8080"));
        let has_https_proxy = |proxy: &OutboundProxy| {
            proxy
                .environment()
                .iter()
                .any(|(key, value)| key == "HTTPS_PROXY" && value.contains("@proxy.local:8080"))
        };
        assert!(proxy.record_health(true));
        assert!(has_https_proxy(&proxy));
        assert!(!proxy.record_health(true));
        assert!(proxy.record_health(false));
        assert!(!has_https_proxy(&proxy));
        assert!(proxy.record_health(true));
        assert!(has_https_proxy(&proxy));
        assert_eq!(proxy.health.generation, 3);
    }
}
