use std::net::{TcpStream, ToSocketAddrs};
use std::time::{Duration, Instant};
use url::Url;

/// Resolve outbound proxy variables before the server creates networking clients or children.
pub fn outbound_proxy_environment(
    override_url: Option<&str>,
) -> Result<Vec<(String, String)>, &'static str> {
    if override_url.is_none()
        && std::env::var_os("MH_OUTBOUND_PROXY_DIRECT").as_deref()
            == Some(std::ffi::OsStr::new("1"))
    {
        return Ok(direct_variables());
    }
    let read = |key| {
        std::env::var(key)
            .ok()
            .filter(|value| !value.trim().is_empty())
    };
    let env = |upper, lower| read(upper).or_else(|| read(lower));
    let configured = read("MH_OUTBOUND_PROXY");
    let explicit = override_url
        .or(configured.as_deref())
        .map(|value| {
            proxy_url(value)
                .ok_or("The outbound proxy must be an HTTP, HTTPS, or SOCKS5 proxy URL.")
        })
        .transpose()?;
    if explicit
        .as_deref()
        .is_some_and(|proxy| !proxy_reachable(proxy))
    {
        return Ok(direct_variables());
    }
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
    Ok(proxy_variables(http, https, &bypass))
}

fn proxy_reachable(proxy: &str) -> bool {
    let Ok(url) = Url::parse(proxy) else {
        return false;
    };
    let Some(host) = url.host_str() else {
        return false;
    };
    let port = url.port_or_known_default().unwrap_or(1080);
    let host = host.trim_matches(['[', ']']);
    let Ok(addresses) = (host, port).to_socket_addrs() else {
        return false;
    };
    let deadline = Instant::now() + Duration::from_secs(1);
    for address in addresses {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        if TcpStream::connect_timeout(&address, remaining).is_ok() {
            return true;
        }
    }
    false
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

fn proxy_url(value: &str) -> Option<String> {
    let value = value.trim();
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
        && url.path() == "/")
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
    }

    #[test]
    fn keeps_local_endpoints_and_custom_exclusions_direct() {
        let values = proxy_variables(None, Some("http://proxy.local:123".into()), ".example.test");
        assert!(values.contains(&(
            "NO_PROXY".into(),
            ".example.test,localhost,127.0.0.1,::1".into()
        )));
    }
}
