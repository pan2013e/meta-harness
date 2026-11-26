use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::RwLock;
use tokio::task::JoinSet;

use crate::AcpError;
use mh_core::{AdapterConfig, AppConfig, QuotaSnapshot, QuotaSource, QuotaWindow};

use crate::platform::{ProcessTreeGuard, find_cli_binary, prepare_isolated_command};
use crate::profile::{has_agy_keyring_entry, is_agy, profile_home};
use crate::supervisor::resolve_adapter_command;

const LIVE_QUOTA_CACHE_TTL_MS: i64 = 5 * 60 * 1000;
const RATE_LIMIT_CACHE_TTL_MS: i64 = 10 * 60 * 1000;

#[derive(Clone, Copy, PartialEq, Eq)]
enum QuotaLookup {
    Cached,
    Live,
    Refresh,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdapterProbeStatus {
    pub adapter_id: String,
    pub display_name: String,
    pub accent_color: String,
    pub installed: bool,
    pub resolved_command: Option<String>,
    pub auth_detected: bool,
    pub auth_hint: String,
    pub strengths: Vec<String>,
}

#[derive(Debug, Clone)]
struct TurnUsageEntry {
    timestamp_ms: i64,
    approx_tokens: u64,
}

#[derive(Debug, Clone, PartialEq)]
struct AdapterQuotaPrefs {
    plan_label: Option<String>,
    exact_probe_opt_in: bool,
    manual_5h_pct: Option<f32>,
    manual_weekly_pct: Option<f32>,
    rate_limited_until: Option<i64>,
    profile_dir: Option<PathBuf>,
    google_cli: Option<String>,
}

impl Default for AdapterQuotaPrefs {
    fn default() -> Self {
        Self {
            plan_label: None,
            exact_probe_opt_in: true,
            manual_5h_pct: None,
            manual_weekly_pct: None,
            rate_limited_until: None,
            profile_dir: None,
            google_cli: None,
        }
    }
}

#[derive(Debug, Clone)]
struct CachedLiveQuota {
    fetched_at_ms: i64,
    ttl_ms: i64,
    outcome: LiveProbeOutcome,
}

#[derive(Debug, Clone, Default)]
struct LiveProbeOutcome {
    plan_label: Option<String>,
    window_5h_remaining_pct: Option<f32>,
    window_5h_resets_at: Option<i64>,
    weekly_remaining_pct: Option<f32>,
    weekly_resets_at: Option<i64>,
    windows: Vec<QuotaWindow>,
    credits_label: Option<String>,
    status_note: Option<String>,
    source: Option<QuotaSource>,
    rate_limited_until: Option<i64>,
}

/// Layered adapter-level subscription quota tracker across Windows, macOS, and Linux.
/// Consults official/native local provider quota APIs (`codex app-server`, Claude OAuth usage,
/// Gemini API checks) with short TTL caching, plus reactive 429 cooldowns and manual overrides.
pub struct QuotaTracker {
    prefs: RwLock<HashMap<String, AdapterQuotaPrefs>>,
    turns: RwLock<HashMap<String, Vec<TurnUsageEntry>>>,
    live_cache: RwLock<HashMap<String, CachedLiveQuota>>,
    http: reqwest::Client,
}

impl QuotaTracker {
    /// Reads the shared CLI account for display only. It never feeds the adapter's quota ledger.
    pub async fn shared_antigravity_usage(
        &self,
        id: &str,
        adapter: &AdapterConfig,
    ) -> Result<Vec<QuotaWindow>, AcpError> {
        let enabled = self
            .prefs
            .read()
            .await
            .get(id)
            .map_or(adapter.exact_quota_probe_opt_in, |prefs| {
                prefs.exact_probe_opt_in
            });
        if !enabled {
            return Err(AcpError::PermissionDenied(
                "Live usage checks are disabled for this adapter.".into(),
            ));
        }
        let (command, _) = resolve_adapter_command(adapter)?;
        if adapter.quota_provider != "google_local"
            || adapter.profile_dir.is_none()
            || !is_agy(&command)
        {
            return Err(AcpError::InvalidProfile(
                "The shared usage view is available for a separate Antigravity profile.".into(),
            ));
        }
        let (outcome, _) = probe_google_live(
            &self.http,
            chrono::Utc::now().timestamp_millis(),
            None,
            Some(&command),
        )
        .await;
        if outcome.source == Some(QuotaSource::LiveProbe) && !outcome.windows.is_empty() {
            return Ok(outcome.windows);
        }
        Err(AcpError::Rpc {
            code: -32000,
            message: outcome
                .status_note
                .unwrap_or_else(|| "Shared Antigravity usage is unavailable.".into()),
            data: None,
        })
    }

    pub async fn configure_launch_profile(&self, adapter_id: &str, cfg: &AdapterConfig) {
        let mut prefs = self.prefs.write().await;
        let entry = prefs.entry(adapter_id.to_string()).or_default();
        entry.profile_dir = cfg.profile_dir.clone();
        entry.google_cli = if cfg.quota_provider == "google_local" {
            resolve_adapter_command(cfg)
                .ok()
                .map(|(command, _)| command)
        } else {
            None
        };
        drop(prefs);
        self.invalidate(adapter_id).await;
    }

    pub fn new() -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(6))
            .build()
            .unwrap_or_default();
        Self {
            prefs: RwLock::new(HashMap::new()),
            turns: RwLock::new(HashMap::new()),
            live_cache: RwLock::new(HashMap::new()),
            http,
        }
    }

    pub async fn configure_adapter(
        &self,
        adapter_id: &str,
        plan_label: Option<String>,
        exact_probe_opt_in: bool,
        manual_5h_pct: Option<f32>,
        manual_weekly_pct: Option<f32>,
    ) {
        {
            let mut guard = self.prefs.write().await;
            let entry = guard.entry(adapter_id.to_string()).or_default();
            entry.plan_label = plan_label;
            entry.exact_probe_opt_in = exact_probe_opt_in;
            entry.manual_5h_pct = manual_5h_pct;
            entry.manual_weekly_pct = manual_weekly_pct;
        }
        self.invalidate(adapter_id).await;
    }

    pub async fn invalidate(&self, adapter_id: &str) {
        self.live_cache.write().await.remove(adapter_id);
    }

    pub async fn invalidate_if_stale(&self, adapter_id: &str, max_age_ms: i64) {
        let now = chrono::Utc::now().timestamp_millis();
        let mut cache = self.live_cache.write().await;
        if cache
            .get(adapter_id)
            .is_some_and(|c| now - c.fetched_at_ms >= max_age_ms)
        {
            cache.remove(adapter_id);
        }
    }

    pub async fn record_turn_tokens(&self, adapter_id: &str, approx_tokens: u64) {
        let now = chrono::Utc::now().timestamp_millis();
        let mut guard = self.turns.write().await;
        let list = guard.entry(adapter_id.to_string()).or_default();
        list.push(TurnUsageEntry {
            timestamp_ms: now,
            approx_tokens,
        });
        let seven_days_ago = now - 7 * 24 * 3600 * 1000;
        list.retain(|e| e.timestamp_ms >= seven_days_ago);
    }

    pub async fn mark_rate_limited(&self, adapter_id: &str, cooldown_secs: u64) {
        let until = chrono::Utc::now().timestamp_millis() + (cooldown_secs as i64 * 1000);
        let mut guard = self.prefs.write().await;
        let entry = guard.entry(adapter_id.to_string()).or_default();
        entry.rate_limited_until = Some(until);
    }

    pub async fn clear_rate_limit(&self, adapter_id: &str) {
        let mut guard = self.prefs.write().await;
        if let Some(entry) = guard.get_mut(adapter_id) {
            entry.rate_limited_until = None;
        }
    }

    pub async fn snapshot(&self, adapter_id: &str, quota_provider: &str) -> QuotaSnapshot {
        self.snapshot_inner(adapter_id, quota_provider, QuotaLookup::Live)
            .await
    }

    pub async fn refresh_all(
        self: &std::sync::Arc<Self>,
        config: &AppConfig,
    ) -> Vec<QuotaSnapshot> {
        self.collect_snapshots(config, QuotaLookup::Refresh).await
    }

    pub async fn all_snapshots(
        self: &std::sync::Arc<Self>,
        config: &AppConfig,
    ) -> Vec<QuotaSnapshot> {
        self.collect_snapshots(config, QuotaLookup::Live).await
    }

    pub async fn cached_snapshots(&self, config: &AppConfig) -> Vec<QuotaSnapshot> {
        let mut snapshots = Vec::new();
        for (id, adapter) in config
            .adapters
            .iter()
            .filter(|(_, adapter)| adapter.enabled)
        {
            snapshots.push(
                self.snapshot_inner(id, &adapter.quota_provider, QuotaLookup::Cached)
                    .await,
            );
        }
        snapshots
    }

    async fn collect_snapshots(
        self: &std::sync::Arc<Self>,
        config: &AppConfig,
        lookup: QuotaLookup,
    ) -> Vec<QuotaSnapshot> {
        let items: Vec<(String, String)> = config
            .adapters
            .iter()
            .filter(|(_, cfg)| cfg.enabled)
            .map(|(id, cfg)| (id.clone(), cfg.quota_provider.clone()))
            .collect();

        let mut set = JoinSet::new();
        for (idx, (id, provider)) in items.iter().cloned().enumerate() {
            let tracker = std::sync::Arc::clone(self);
            set.spawn(async move {
                let snap = tracker.snapshot_inner(&id, &provider, lookup).await;
                (idx, snap)
            });
        }

        let mut ordered: Vec<(usize, QuotaSnapshot)> = Vec::with_capacity(items.len());
        while let Some(res) = set.join_next().await {
            if let Ok(pair) = res {
                ordered.push(pair);
            }
        }
        ordered.sort_by_key(|(idx, _)| *idx);
        ordered.into_iter().map(|(_, s)| s).collect()
    }

    async fn snapshot_inner(
        &self,
        adapter_id: &str,
        quota_provider: &str,
        lookup: QuotaLookup,
    ) -> QuotaSnapshot {
        let now = chrono::Utc::now().timestamp_millis();
        let pref = self
            .prefs
            .read()
            .await
            .get(adapter_id)
            .cloned()
            .unwrap_or_default();

        let (mh_5h_tokens, mh_7d_tokens) = {
            let guard = self.turns.read().await;
            let five_h_ago = now - 5 * 3600 * 1000;
            let seven_d_ago = now - 7 * 24 * 3600 * 1000;
            let entries = guard.get(adapter_id).map(Vec::as_slice).unwrap_or(&[]);
            let t5: u64 = entries
                .iter()
                .filter(|e| e.timestamp_ms >= five_h_ago)
                .map(|e| e.approx_tokens)
                .sum();
            let t7: u64 = entries
                .iter()
                .filter(|e| e.timestamp_ms >= seven_d_ago)
                .map(|e| e.approx_tokens)
                .sum();
            (t5, t7)
        };

        if pref.rate_limited_until.is_some_and(|until| until > now) {
            return QuotaSnapshot {
                adapter_id: adapter_id.to_string(),
                plan_label: pref.plan_label,
                window_5h_remaining_pct: Some(0.0),
                window_5h_resets_at: pref.rate_limited_until,
                weekly_remaining_pct: Some(0.0),
                weekly_resets_at: pref.rate_limited_until,
                local_5h_tokens_est: mh_5h_tokens,
                rate_limited_until: pref.rate_limited_until,
                exact_probe_opt_in: pref.exact_probe_opt_in,
                source: QuotaSource::Cooldown429,
                windows: vec![QuotaWindow {
                    id: "cooldown".into(),
                    label: "429 Cooldown".into(),
                    remaining_pct: 0.0,
                    resets_at: pref.rate_limited_until,
                    window_mins: None,
                }],
                credits_label: None,
                status_note: Some("429 cooldown active".into()),
                updated_at: now,
            };
        }

        // Offline/deterministic test provider ("manual")
        if quota_provider == "manual" {
            return self
                .manual_snapshot(adapter_id, &pref, mh_5h_tokens, mh_7d_tokens, now)
                .await;
        }

        let cached = if pref.exact_probe_opt_in && lookup != QuotaLookup::Refresh {
            let guard = self.live_cache.read().await;
            guard
                .get(adapter_id)
                .filter(|c| now - c.fetched_at_ms < c.ttl_ms)
                .map(|c| c.outcome.clone())
        } else {
            None
        };

        let outcome = if !pref.exact_probe_opt_in {
            LiveProbeOutcome {
                status_note: Some("Live API probes are off".into()),
                ..Default::default()
            }
        } else if let Some(hit) = cached {
            hit
        } else if lookup == QuotaLookup::Cached {
            LiveProbeOutcome::default()
        } else {
            let (fresh, ttl_ms) = match quota_provider {
                "codex_local" => {
                    probe_codex_live(&self.http, now, pref.profile_dir.as_deref()).await
                }
                "claude_local" => {
                    probe_claude_live(&self.http, now, pref.profile_dir.as_deref()).await
                }
                "google_local" => {
                    probe_google_live(
                        &self.http,
                        now,
                        pref.profile_dir.as_deref(),
                        pref.google_cli.as_deref(),
                    )
                    .await
                }
                _ => (
                    LiveProbeOutcome {
                        status_note: Some("Custom adapter (no quota API)".into()),
                        ..Default::default()
                    },
                    LIVE_QUOTA_CACHE_TTL_MS,
                ),
            };
            {
                let current_prefs = self.prefs.read().await;
                if current_prefs
                    .get(adapter_id)
                    .map_or(pref == AdapterQuotaPrefs::default(), |current| {
                        *current == pref
                    })
                {
                    self.live_cache.write().await.insert(
                        adapter_id.to_string(),
                        CachedLiveQuota {
                            fetched_at_ms: now,
                            ttl_ms,
                            outcome: fresh.clone(),
                        },
                    );
                }
            }
            fresh
        };

        // If user entered manual calibration percentages and live API isn't returning percentages, honor them
        if outcome.source != Some(QuotaSource::LiveProbe)
            && outcome.rate_limited_until.is_none_or(|until| until <= now)
            && (pref.manual_5h_pct.is_some() || pref.manual_weekly_pct.is_some())
        {
            return self
                .manual_snapshot(adapter_id, &pref, mh_5h_tokens, mh_7d_tokens, now)
                .await;
        }

        let effective_rl = outcome.rate_limited_until.filter(|&u| u > now);
        QuotaSnapshot {
            adapter_id: adapter_id.to_string(),
            plan_label: outcome.plan_label.or(pref.plan_label),
            window_5h_remaining_pct: outcome.window_5h_remaining_pct,
            window_5h_resets_at: outcome.window_5h_resets_at,
            weekly_remaining_pct: outcome.weekly_remaining_pct,
            weekly_resets_at: outcome.weekly_resets_at,
            local_5h_tokens_est: mh_5h_tokens,
            rate_limited_until: effective_rl,
            exact_probe_opt_in: pref.exact_probe_opt_in,
            source: outcome.source.unwrap_or(QuotaSource::Unavailable),
            windows: outcome.windows,
            credits_label: outcome.credits_label,
            status_note: outcome.status_note,
            updated_at: now,
        }
    }

    async fn manual_snapshot(
        &self,
        adapter_id: &str,
        pref: &AdapterQuotaPrefs,
        mh_5h_tokens: u64,
        mh_7d_tokens: u64,
        now: i64,
    ) -> QuotaSnapshot {
        let (budget_5h, budget_7d) = plan_token_budgets(pref.plan_label.as_deref());
        let (w5, ww, source) = if pref.manual_5h_pct.is_some() || pref.manual_weekly_pct.is_some() {
            let used_5h_pct = (mh_5h_tokens as f32 / budget_5h as f32) * 100.0;
            let w5_val = pref
                .manual_5h_pct
                .map(|m| (m - used_5h_pct).clamp(0.0, 100.0));
            let ww_val = pref.manual_weekly_pct.map(|m| m.clamp(0.0, 100.0));
            (w5_val, ww_val, QuotaSource::UserCalibrated)
        } else {
            let used_5h_pct = ((mh_5h_tokens as f32 / budget_5h as f32) * 100.0).min(100.0);
            let used_7d_pct = ((mh_7d_tokens as f32 / budget_7d as f32) * 100.0).min(100.0);
            (
                Some((100.0 - used_5h_pct).clamp(0.0, 100.0)),
                Some((100.0 - used_7d_pct).clamp(0.0, 100.0)),
                QuotaSource::LocalLogEstimate,
            )
        };

        let mut windows = Vec::new();
        if let Some(p5) = w5 {
            windows.push(QuotaWindow {
                id: "5h".into(),
                label: "5h Window".into(),
                remaining_pct: p5,
                resets_at: None,
                window_mins: Some(300),
            });
        }
        if let Some(pw) = ww {
            windows.push(QuotaWindow {
                id: "7d".into(),
                label: "Weekly".into(),
                remaining_pct: pw,
                resets_at: None,
                window_mins: Some(10080),
            });
        }

        QuotaSnapshot {
            adapter_id: adapter_id.to_string(),
            plan_label: pref.plan_label.clone(),
            window_5h_remaining_pct: w5,
            window_5h_resets_at: None,
            weekly_remaining_pct: ww,
            weekly_resets_at: None,
            local_5h_tokens_est: mh_5h_tokens,
            rate_limited_until: None,
            exact_probe_opt_in: pref.exact_probe_opt_in,
            source,
            windows,
            credits_label: None,
            status_note: None,
            updated_at: now,
        }
    }
}

impl Default for QuotaTracker {
    fn default() -> Self {
        Self::new()
    }
}

fn plan_token_budgets(plan_label: Option<&str>) -> (u64, u64) {
    let lower = plan_label.unwrap_or("pro").to_ascii_lowercase();
    if lower.contains("20x") || lower.contains("ultra") {
        (4_000_000, 40_000_000)
    } else if lower.contains("5x") {
        (1_500_000, 15_000_000)
    } else if lower.contains("pro") || lower.contains("max") {
        (800_000, 8_000_000)
    } else {
        (400_000, 4_000_000)
    }
}

fn normalize_epoch_ms(ts: i64) -> i64 {
    if ts > 0 && ts < 1_000_000_000_000 {
        ts.saturating_mul(1000)
    } else {
        ts
    }
}

fn format_plan_label(raw: &str) -> String {
    match raw.to_ascii_lowercase().as_str() {
        "prolite" => "ProLite".into(),
        "plus" => "Plus".into(),
        "pro" => "Pro".into(),
        "team" => "Team".into(),
        "business" => "Business".into(),
        "enterprise" => "Enterprise".into(),
        "free" => "Free".into(),
        "go" => "Go".into(),
        "max_5x" | "max 5x" => "Max 5x".into(),
        "max_20x" | "max 20x" => "Max 20x".into(),
        other => {
            let mut chars = other.chars();
            match chars.next() {
                None => String::new(),
                Some(first) => format!("{}{}", first.to_uppercase(), chars.as_str()),
            }
        }
    }
}

async fn probe_codex_live(
    http: &reqwest::Client,
    now: i64,
    profile: Option<&Path>,
) -> (LiveProbeOutcome, i64) {
    // 1. Native `codex app-server --stdio` RPC (`account/rateLimits/read`)
    if let Some(codex_bin) = find_cli_binary("codex")
        && let Some(outcome) = probe_codex_app_server(&codex_bin, now, profile).await
    {
        return (outcome, LIVE_QUOTA_CACHE_TTL_MS);
    }

    // 2. Fallback: ChatGPT Codex OAuth usage endpoint from `~/.codex/auth.json`
    if let Some((token, account_id)) = read_codex_oauth_credentials(profile) {
        let mut req = http
            .get("https://chatgpt.com/backend-api/wham/usage")
            .bearer_auth(&token)
            .header(reqwest::header::ACCEPT, "application/json");
        if let Some(acc) = account_id.filter(|s| !s.is_empty()) {
            req = req.header("ChatGPT-Account-Id", acc);
        }
        if let Ok(resp) = req.send().await {
            if resp.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
                return (
                    LiveProbeOutcome {
                        status_note: Some("ChatGPT usage API rate-limited (429)".into()),
                        source: Some(QuotaSource::Unavailable),
                        ..Default::default()
                    },
                    RATE_LIMIT_CACHE_TTL_MS,
                );
            }
            if resp.status().is_success()
                && let Ok(json) = resp.json::<Value>().await
                && let Some(outcome) = parse_codex_wham_usage(&json)
            {
                return (outcome, LIVE_QUOTA_CACHE_TTL_MS);
            }
        }
    }

    if std::env::var_os("OPENAI_API_KEY").is_some_and(|v| !v.is_empty()) {
        return (
            LiveProbeOutcome {
                plan_label: Some("OpenAI API Key".into()),
                status_note: Some("Pay-per-token API key (no 5h/7d cap)".into()),
                source: Some(QuotaSource::Unavailable),
                ..Default::default()
            },
            LIVE_QUOTA_CACHE_TTL_MS,
        );
    }

    let note = if find_cli_binary("codex").is_some() {
        "Run `codex login` to load subscription quota"
    } else {
        "Codex CLI not installed"
    };
    (
        LiveProbeOutcome {
            status_note: Some(note.into()),
            source: Some(QuotaSource::Unavailable),
            ..Default::default()
        },
        LIVE_QUOTA_CACHE_TTL_MS,
    )
}

async fn probe_codex_app_server(
    codex_bin: &Path,
    now: i64,
    profile: Option<&Path>,
) -> Option<LiveProbeOutcome> {
    let mut cmd = Command::new(codex_bin);
    if let Some(path) = profile {
        cmd.env("CODEX_HOME", dunce::simplified(path));
    }
    cmd.args(["app-server", "--stdio"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    prepare_isolated_command(&mut cmd);

    let mut child = cmd.spawn().ok()?;
    let mut guard = ProcessTreeGuard::attach(&child);
    let mut stdin = child.stdin.take()?;
    let stdout = child.stdout.take()?;

    let rpc_fut = async {
        stdin
            .write_all(
                br#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"clientInfo":{"name":"meta-harness","version":"0.1.0"},"capabilities":{"experimentalApi":true}}}
"#,
            )
            .await
            .ok()?;
        stdin.flush().await.ok()?;

        let mut lines = BufReader::new(stdout).lines();
        for _ in 0..8 {
            let line = lines.next_line().await.ok().flatten()?;
            if let Ok(val) = serde_json::from_str::<Value>(&line)
                && val.get("id").and_then(Value::as_i64) == Some(1)
            {
                break;
            }
        }

        stdin
            .write_all(
                br#"{"jsonrpc":"2.0","method":"initialized"}
{"jsonrpc":"2.0","id":2,"method":"account/rateLimits/read","params":{"excludeResetCreditDetails":true}}
"#,
            )
            .await
            .ok()?;
        stdin.flush().await.ok()?;

        for _ in 0..24 {
            let line = lines.next_line().await.ok().flatten()?;
            if let Ok(val) = serde_json::from_str::<Value>(&line)
                && val.get("id").and_then(Value::as_i64) == Some(2)
            {
                return val
                    .get("result")
                    .and_then(|res| parse_codex_app_server_result(res, now));
            }
        }
        None
    };

    let outcome = tokio::time::timeout(Duration::from_secs(6), rpc_fut)
        .await
        .ok()
        .flatten();

    guard.kill_tree();
    let _ = tokio::time::timeout(Duration::from_millis(200), child.wait()).await;
    outcome
}

fn parse_codex_app_server_result(result: &Value, _now: i64) -> Option<LiveProbeOutcome> {
    let rl = result
        .get("rateLimitsByLimitId")
        .and_then(|m| m.get("codex"))
        .or_else(|| result.get("rateLimits"))?;

    let ordinary_allowed = result.get("ordinaryUsageAllowed").and_then(Value::as_bool);
    let plan_label = rl
        .get("planType")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty() && *s != "unknown")
        .map(format_plan_label);

    let mut out = LiveProbeOutcome {
        plan_label,
        source: Some(QuotaSource::LiveProbe),
        status_note: Some("Live · Codex app-server".into()),
        ..Default::default()
    };

    for slot in ["primary", "secondary"] {
        let Some(w) = rl.get(slot).filter(|v| v.is_object()) else {
            continue;
        };
        let used = w.get("usedPercent").and_then(Value::as_f64).unwrap_or(0.0) as f32;
        let rem = if ordinary_allowed == Some(false) && used >= 99.5 {
            0.0
        } else {
            (100.0 - used).clamp(0.0, 100.0)
        };
        let resets_at = w
            .get("resetsAt")
            .and_then(Value::as_i64)
            .map(normalize_epoch_ms);
        let mins = w.get("windowDurationMins").and_then(Value::as_i64);

        let (id, label) = classify_window(mins, slot == "primary");
        if id == "5h" || mins.is_some_and(|m| m <= 360) {
            out.window_5h_remaining_pct = Some(rem);
            out.window_5h_resets_at = resets_at;
        } else if id == "7d" || mins.is_some_and(|m| m >= 1440) {
            out.weekly_remaining_pct = Some(rem);
            out.weekly_resets_at = resets_at;
        }

        out.windows.push(QuotaWindow {
            id,
            label,
            remaining_pct: rem,
            resets_at,
            window_mins: mins,
        });
    }

    if let Some(credits) = rl.get("credits").filter(|v| v.is_object()) {
        if credits.get("unlimited").and_then(Value::as_bool) == Some(true) {
            out.credits_label = Some("Unlimited credits".into());
        } else if credits.get("hasCredits").and_then(Value::as_bool) == Some(true)
            && let Some(bal) = credits.get("balance").and_then(|v| {
                v.as_str()
                    .and_then(|s| s.trim().parse::<f64>().ok())
                    .or_else(|| v.as_f64())
            })
        {
            out.credits_label = Some(format!("Credits: {}", bal.round() as i64));
        }
    }

    if out.windows.is_empty() && rl.get("rateLimitReachedType").is_some() {
        out.status_note = Some("Usage limit reached · Codex app-server".into());
    }

    Some(out)
}

fn parse_codex_wham_usage(json: &Value) -> Option<LiveProbeOutcome> {
    let rl = json.get("rate_limit")?;
    let plan_label = json
        .get("plan_type")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(format_plan_label);

    let mut out = LiveProbeOutcome {
        plan_label,
        source: Some(QuotaSource::LiveProbe),
        status_note: Some("Live · ChatGPT Usage API".into()),
        ..Default::default()
    };

    for (key, is_primary) in [("primary_window", true), ("secondary_window", false)] {
        let Some(w) = rl.get(key).filter(|v| v.is_object()) else {
            continue;
        };
        let used = w.get("used_percent").and_then(Value::as_f64).unwrap_or(0.0) as f32;
        let rem = (100.0 - used).clamp(0.0, 100.0);
        let resets_at = w
            .get("reset_at")
            .and_then(Value::as_i64)
            .map(normalize_epoch_ms);
        let mins = w
            .get("limit_window_seconds")
            .and_then(Value::as_i64)
            .map(|s| s / 60);

        let (id, label) = classify_window(mins, is_primary);
        if id == "5h" || mins.is_some_and(|m| m <= 360) {
            out.window_5h_remaining_pct = Some(rem);
            out.window_5h_resets_at = resets_at;
        } else if id == "7d" || mins.is_some_and(|m| m >= 1440) {
            out.weekly_remaining_pct = Some(rem);
            out.weekly_resets_at = resets_at;
        }

        out.windows.push(QuotaWindow {
            id,
            label,
            remaining_pct: rem,
            resets_at,
            window_mins: mins,
        });
    }

    Some(out)
}

fn classify_window(mins: Option<i64>, is_primary: bool) -> (String, String) {
    match mins {
        Some(m) if m <= 360 => ("5h".into(), format!("{}h Window", (m / 60).max(1))),
        Some(m) if m >= 1440 => {
            let days = m / 1440;
            if days == 7 {
                ("7d".into(), "Weekly".into())
            } else {
                ("7d".into(), format!("{days}d Window"))
            }
        }
        Some(m) if m % 60 == 0 => ("custom".into(), format!("{}h Window", m / 60)),
        Some(m) => ("custom".into(), format!("{m}m Window")),
        None if is_primary => ("5h".into(), "5h Window".into()),
        None => ("7d".into(), "Weekly".into()),
    }
}

fn read_codex_oauth_credentials(profile: Option<&Path>) -> Option<(String, Option<String>)> {
    let codex_dir = profile_home(profile, "CODEX_HOME", ".codex")?;
    let raw = std::fs::read_to_string(codex_dir.join("auth.json")).ok()?;
    let json: Value = serde_json::from_str(&raw).ok()?;
    let token = json
        .get("tokens")
        .and_then(|t| t.get("access_token"))
        .and_then(Value::as_str)
        .or_else(|| json.get("access_token").and_then(Value::as_str))?
        .trim()
        .to_string();
    if token.is_empty() {
        return None;
    }
    let account_id = json
        .get("tokens")
        .and_then(|t| t.get("account_id"))
        .and_then(Value::as_str)
        .map(str::to_string);
    Some((token, account_id))
}

async fn probe_claude_live(
    http: &reqwest::Client,
    _now: i64,
    profile: Option<&Path>,
) -> (LiveProbeOutcome, i64) {
    if let Some((token, sub_type)) = read_claude_oauth_credentials(profile).await {
        let resp = http
            .get("https://api.anthropic.com/api/oauth/usage")
            .bearer_auth(&token)
            .header("anthropic-beta", "oauth-2025-04-20")
            .header(reqwest::header::ACCEPT, "application/json")
            .send()
            .await;

        if let Ok(r) = resp {
            let status = r.status();
            if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
                return (
                    LiveProbeOutcome {
                        plan_label: sub_type.map(|s| format_plan_label(&s)),
                        status_note: Some("Anthropic usage API rate-limited (cached 10m)".into()),
                        source: Some(QuotaSource::Unavailable),
                        ..Default::default()
                    },
                    RATE_LIMIT_CACHE_TTL_MS,
                );
            }
            if status == reqwest::StatusCode::UNAUTHORIZED
                || status == reqwest::StatusCode::FORBIDDEN
            {
                return (
                    LiveProbeOutcome {
                        plan_label: sub_type.map(|s| format_plan_label(&s)),
                        status_note: Some("Claude OAuth expired — run `claude login`".into()),
                        source: Some(QuotaSource::Unavailable),
                        ..Default::default()
                    },
                    LIVE_QUOTA_CACHE_TTL_MS,
                );
            }
            if status.is_success()
                && let Ok(json) = r.json::<Value>().await
            {
                return (
                    parse_claude_usage_json(&json, sub_type.as_deref()),
                    LIVE_QUOTA_CACHE_TTL_MS,
                );
            }
        }
    }

    if std::env::var_os("ANTHROPIC_API_KEY").is_some_and(|v| !v.is_empty()) {
        return (
            LiveProbeOutcome {
                plan_label: Some("Anthropic API Key".into()),
                status_note: Some("Pay-per-token API key (no 5h/7d cap)".into()),
                source: Some(QuotaSource::Unavailable),
                ..Default::default()
            },
            LIVE_QUOTA_CACHE_TTL_MS,
        );
    }

    let note = if find_cli_binary("claude").is_some() {
        "Run `claude login` to load subscription quota"
    } else {
        "Claude Code CLI not installed"
    };
    (
        LiveProbeOutcome {
            status_note: Some(note.into()),
            source: Some(QuotaSource::Unavailable),
            ..Default::default()
        },
        LIVE_QUOTA_CACHE_TTL_MS,
    )
}

fn parse_claude_usage_json(json: &Value, sub_type: Option<&str>) -> LiveProbeOutcome {
    let mut out = LiveProbeOutcome {
        plan_label: sub_type.map(format_plan_label),
        source: Some(QuotaSource::LiveProbe),
        status_note: Some("Live · Anthropic Usage API".into()),
        ..Default::default()
    };

    let defs = [
        ("five_hour", "5h", "5h Window", Some(300i64)),
        ("seven_day", "7d", "Weekly (All)", Some(10080)),
        ("seven_day_sonnet", "7d_sonnet", "Sonnet 7d", Some(10080)),
        ("seven_day_opus", "7d_opus", "Opus 7d", Some(10080)),
    ];

    for (key, id, label, mins) in defs {
        let Some(w) = json.get(key).filter(|v| v.is_object()) else {
            continue;
        };
        let Some(util) = w.get("utilization").and_then(Value::as_f64) else {
            continue;
        };
        let rem = (100.0 - util as f32).clamp(0.0, 100.0);
        let resets_at = w
            .get("resets_at")
            .and_then(Value::as_str)
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .map(|dt| dt.timestamp_millis());

        if id == "5h" {
            out.window_5h_remaining_pct = Some(rem);
            out.window_5h_resets_at = resets_at;
        } else if id == "7d" {
            out.weekly_remaining_pct = Some(rem);
            out.weekly_resets_at = resets_at;
        }

        out.windows.push(QuotaWindow {
            id: id.into(),
            label: label.into(),
            remaining_pct: rem,
            resets_at,
            window_mins: mins,
        });
    }

    if let Some(extra) = json.get("extra_usage").filter(|v| v.is_object())
        && extra.get("is_enabled").and_then(Value::as_bool) == Some(true)
    {
        let used = extra.get("used_credits").and_then(Value::as_f64);
        let limit = extra.get("monthly_limit").and_then(Value::as_f64);
        if let (Some(u), Some(l)) = (used, limit) {
            out.credits_label = Some(format!("Extra usage: {u:.0}/{l:.0}"));
        } else {
            out.credits_label = Some("Extra usage enabled".into());
        }
    }

    out
}

async fn read_claude_oauth_credentials(profile: Option<&Path>) -> Option<(String, Option<String>)> {
    let claude_dir = profile_home(profile, "CLAUDE_CONFIG_DIR", ".claude");

    if let Some(dir) = claude_dir {
        let cred_path = dir.join(".credentials.json");
        if let Ok(raw) = std::fs::read_to_string(&cred_path)
            && let Some(parsed) = extract_claude_oauth_from_json(&raw)
        {
            return Some(parsed);
        }
    }

    #[cfg(target_os = "macos")]
    if profile.is_none() {
        if let Ok(output) = tokio::time::timeout(
            Duration::from_secs(2),
            Command::new("security")
                .args([
                    "find-generic-password",
                    "-s",
                    "Claude Code-credentials",
                    "-w",
                ])
                .output(),
        )
        .await
            && let Ok(out) = output
            && out.status.success()
        {
            let raw = String::from_utf8_lossy(&out.stdout);
            if let Some(parsed) = extract_claude_oauth_from_json(&raw) {
                return Some(parsed);
            }
        }
    }

    None
}

fn extract_claude_oauth_from_json(raw: &str) -> Option<(String, Option<String>)> {
    let json: Value = serde_json::from_str(raw.trim()).ok()?;
    let oauth = json.get("claudeAiOauth").unwrap_or(&json);
    let token = oauth
        .get("accessToken")
        .and_then(Value::as_str)?
        .trim()
        .to_string();
    if token.is_empty() {
        return None;
    }
    let sub = oauth
        .get("subscriptionType")
        .or_else(|| oauth.get("rateLimitTier"))
        .and_then(Value::as_str)
        .map(str::to_string);
    Some((token, sub))
}

async fn probe_google_live(
    http: &reqwest::Client,
    now: i64,
    profile: Option<&Path>,
    configured_cli: Option<&str>,
) -> (LiveProbeOutcome, i64) {
    let executable = configured_cli.map(str::to_string).or_else(|| {
        find_cli_binary("agy")
            .or_else(|| find_cli_binary("gemini"))
            .map(|p| p.to_string_lossy().into_owned())
    });
    if let Some(command) = executable.as_deref().filter(|command| is_agy(command)) {
        let credentials = has_google_cli_auth(profile, true);
        if profile.is_some() {
            return (LiveProbeOutcome {
                plan_label: credentials.then(|| "Google Account".into()),
                status_note: Some("Antigravity's usage command reads the shared CLI profile; private-profile quota is unavailable".into()),
                source: Some(QuotaSource::Unavailable),
                ..Default::default()
            }, LIVE_QUOTA_CACHE_TTL_MS);
        }
        let windows = if credentials {
            crate::antigravity::run_agy(command, None, &["-p", "/usage", "--print-timeout", "12s"])
                .await
                .map_or_else(Vec::new, |output| {
                    crate::antigravity::quota_windows(&output)
                })
        } else {
            Vec::new()
        };
        if !windows.is_empty() {
            let limiting = |minutes| {
                windows
                    .iter()
                    .filter(|window| window.window_mins == Some(minutes))
                    .min_by(|a, b| a.remaining_pct.total_cmp(&b.remaining_pct))
            };
            let five = limiting(300);
            let week = limiting(10_080);
            return (
                LiveProbeOutcome {
                    plan_label: Some("Google Account".into()),
                    window_5h_remaining_pct: five.map(|window| window.remaining_pct),
                    window_5h_resets_at: five.and_then(|window| window.resets_at),
                    weekly_remaining_pct: week.map(|window| window.remaining_pct),
                    weekly_resets_at: week.and_then(|window| window.resets_at),
                    windows,
                    source: Some(QuotaSource::LiveProbe),
                    status_note: Some("Live · Antigravity CLI".into()),
                    ..Default::default()
                },
                LIVE_QUOTA_CACHE_TTL_MS,
            );
        }
        return (
            LiveProbeOutcome {
                plan_label: credentials.then(|| "Google Account".into()),
                status_note: Some(
                    if credentials {
                        "Antigravity CLI usage is unavailable"
                    } else {
                        "Antigravity sign-in status unavailable"
                    }
                    .into(),
                ),
                source: Some(QuotaSource::Unavailable),
                ..Default::default()
            },
            30_000,
        );
    }
    let api_key = std::env::var("GEMINI_API_KEY")
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| {
            std::env::var("GOOGLE_API_KEY")
                .ok()
                .filter(|s| !s.is_empty())
        });

    if let Some(key) = api_key {
        let resp = http
            .get("https://generativelanguage.googleapis.com/v1beta/models?pageSize=1")
            .header("x-goog-api-key", key)
            .send()
            .await;

        if let Ok(r) = resp {
            if r.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
                return (
                    LiveProbeOutcome {
                        plan_label: Some("Gemini API Key".into()),
                        window_5h_remaining_pct: Some(0.0),
                        status_note: Some("Gemini API rate limited (429)".into()),
                        source: Some(QuotaSource::Cooldown429),
                        rate_limited_until: Some(now + 60_000),
                        ..Default::default()
                    },
                    60_000,
                );
            }
            if r.status().is_success() {
                return (
                    LiveProbeOutcome {
                        plan_label: Some("Gemini API Key".into()),
                        status_note: Some("API key verified · metered RPM/RPD".into()),
                        source: Some(QuotaSource::Unavailable),
                        ..Default::default()
                    },
                    LIVE_QUOTA_CACHE_TTL_MS,
                );
            }
        }

        return (
            LiveProbeOutcome {
                plan_label: Some("Gemini API Key".into()),
                status_note: Some("API key present (check failed)".into()),
                source: Some(QuotaSource::Unavailable),
                ..Default::default()
            },
            LIVE_QUOTA_CACHE_TTL_MS,
        );
    }

    let harness = executable.as_ref().map(|_| false);

    let (plan, note) = if harness.is_some_and(|agy| has_google_cli_auth(profile, agy)) {
        (
            Some("Google Account".into()),
            "Credentials detected · no public OAuth quota percentage",
        )
    } else if harness.is_some() {
        (None, "CLI found · local sign-in status unavailable")
    } else {
        (None, "CLI (`agy` / `gemini`) not installed")
    };

    (
        LiveProbeOutcome {
            plan_label: plan,
            status_note: Some(note.into()),
            source: Some(QuotaSource::Unavailable),
            ..Default::default()
        },
        LIVE_QUOTA_CACHE_TTL_MS,
    )
}

pub(crate) fn has_google_cli_auth(profile: Option<&Path>, agy: bool) -> bool {
    if agy {
        return profile_home(profile, "GEMINI_HOME", ".gemini").is_some_and(|root| {
            ["antigravity-cli", "antigravity-acp"]
                .iter()
                .any(|dir| root.join(dir).join("antigravity-oauth-token").is_file())
        }) || has_agy_keyring_entry();
    }
    let home = profile_home(profile, "GEMINI_CLI_HOME", "");
    home.is_some_and(|root| root.join(".gemini/oauth_creds.json").is_file())
}

pub fn probe_installed_adapters(config: &AppConfig) -> Vec<AdapterProbeStatus> {
    let mut results = Vec::new();

    for (id, cfg) in &config.adapters {
        let command = resolve_adapter_command(cfg);
        let agy = command.as_ref().is_ok_and(|(command, _)| is_agy(command));
        let profile = cfg.profile_dir.as_deref();
        let (auth_detected, auth_hint) = match cfg.quota_provider.as_str() {
            "codex_local" => {
                let codex_dir = profile_home(profile, "CODEX_HOME", ".codex");
                let has_oauth = codex_dir
                    .as_ref()
                    .is_some_and(|d| d.join("auth.json").is_file());
                let has_key = std::env::var_os("OPENAI_API_KEY").is_some_and(|v| !v.is_empty());
                (
                    has_oauth || has_key,
                    if has_oauth || has_key {
                        "Codex credentials detected"
                    } else {
                        "Sign in with Codex CLI"
                    },
                )
            }
            "claude_local" => {
                let dir = profile_home(profile, "CLAUDE_CONFIG_DIR", ".claude");
                let has_creds = dir
                    .as_ref()
                    .is_some_and(|d| d.join(".credentials.json").is_file());
                let has_key = std::env::var_os("ANTHROPIC_API_KEY").is_some_and(|v| !v.is_empty());
                (
                    has_creds || has_key,
                    if has_creds || has_key {
                        "Claude credentials detected"
                    } else {
                        "Sign in with Claude Code"
                    },
                )
            }
            "google_local" => {
                let signed_in = has_google_cli_auth(profile, agy);
                let has_key = std::env::var_os("GEMINI_API_KEY").is_some_and(|v| !v.is_empty())
                    || std::env::var_os("GOOGLE_API_KEY").is_some_and(|v| !v.is_empty());
                let hint = match (signed_in, agy, has_key) {
                    (true, true, _) => "Antigravity sign-in detected",
                    (true, false, _) => "Gemini CLI credentials detected",
                    (_, _, true) => "Gemini API key detected",
                    _ => "CLI sign-in status unavailable",
                };
                (signed_in || has_key, hint)
            }
            _ => (true, "Custom ACP adapter"),
        };
        let (resolved, auth_hint) = match command {
            Ok((command, _)) => (Some(command), auth_hint.into()),
            Err(error) => (None, error.to_string()),
        };
        results.push(AdapterProbeStatus {
            adapter_id: id.clone(),
            display_name: cfg.display_name.clone(),
            accent_color: cfg.accent_color.clone(),
            installed: resolved.is_some(),
            resolved_command: resolved,
            auth_detected,
            auth_hint,
            strengths: cfg.characteristics.strengths.clone(),
        });
    }

    results
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn shared_usage_respects_adapter_opt_out() {
        let tracker = QuotaTracker::new();
        tracker
            .configure_adapter("antigravity", None, false, None, None)
            .await;
        let adapter = AdapterConfig {
            quota_provider: "google_local".into(),
            exact_quota_probe_opt_in: true,
            profile_dir: Some("profiles/antigravity".into()),
            ..AdapterConfig::default()
        };
        let error = tracker
            .shared_antigravity_usage("antigravity", &adapter)
            .await
            .unwrap_err();
        assert!(matches!(error, AcpError::PermissionDenied(_)));
    }
    use serde_json::json;

    #[test]
    fn parses_codex_app_server_prolite_single_weekly_window_honestly() {
        let payload = json!({
            "ordinaryUsageAllowed": false,
            "rateLimits": {
                "limitId": "codex",
                "planType": "prolite",
                "primary": {
                    "resetsAt": 1776000000i64,
                    "usedPercent": 100,
                    "windowDurationMins": 10080
                },
                "secondary": null,
                "credits": { "hasCredits": false, "unlimited": false, "balance": null }
            }
        });
        let parsed = parse_codex_app_server_result(&payload, 1000).unwrap();
        assert_eq!(parsed.plan_label.as_deref(), Some("ProLite"));
        // 10080 mins is weekly -> should NOT populate a fake 5h %!
        assert_eq!(parsed.window_5h_remaining_pct, None);
        assert_eq!(parsed.weekly_remaining_pct, Some(0.0));
        assert_eq!(parsed.windows.len(), 1);
        assert_eq!(parsed.windows[0].label, "Weekly");
        assert_eq!(parsed.windows[0].resets_at, Some(1776000000i64 * 1000));
    }

    #[test]
    fn rounds_codex_credit_balance_to_integer() {
        let payload = json!({
            "rateLimits": {
                "limitId": "codex",
                "planType": "prolite",
                "primary": { "resetsAt": 1776000000i64, "usedPercent": 0, "windowDurationMins": 10080 },
                "credits": { "hasCredits": true, "unlimited": false, "balance": "60002.0775370000" }
            }
        });
        let parsed = parse_codex_app_server_result(&payload, 1000).unwrap();
        assert_eq!(parsed.credits_label.as_deref(), Some("Credits: 60002"));
    }

    #[test]
    fn parses_claude_oauth_usage_windows() {
        let payload = json!({
            "five_hour": { "utilization": 22.0, "resets_at": "2026-11-26T04:00:00Z" },
            "seven_day": { "utilization": 65.5, "resets_at": "2026-11-30T00:00:00Z" },
            "seven_day_sonnet": { "utilization": 10.0, "resets_at": "2026-11-30T00:00:00Z" }
        });
        let parsed = parse_claude_usage_json(&payload, Some("max_5x"));
        assert_eq!(parsed.plan_label.as_deref(), Some("Max 5x"));
        assert_eq!(parsed.window_5h_remaining_pct, Some(78.0));
        assert_eq!(parsed.weekly_remaining_pct, Some(34.5));
        assert_eq!(parsed.windows.len(), 3);
        assert!(parsed.window_5h_resets_at.is_some());
    }

    #[tokio::test]
    async fn manual_provider_is_deterministic_offline() {
        let tracker = QuotaTracker::new();
        tracker
            .configure_adapter("mock", Some("pro".into()), true, None, None)
            .await;
        let snap = tracker.snapshot("mock", "manual").await;
        assert_eq!(snap.source, QuotaSource::LocalLogEstimate);
        assert_eq!(snap.window_5h_remaining_pct, Some(100.0));
        assert_eq!(snap.windows.len(), 2);
    }

    #[tokio::test]
    async fn saved_opt_out_ignores_cached_live_data_and_keeps_weekly_calibration() {
        let tracker = QuotaTracker::new();
        let adapter = "codex";
        tracker
            .configure_adapter(adapter, None, false, None, Some(64.0))
            .await;
        tracker.live_cache.write().await.insert(
            adapter.into(),
            CachedLiveQuota {
                fetched_at_ms: chrono::Utc::now().timestamp_millis(),
                ttl_ms: LIVE_QUOTA_CACHE_TTL_MS,
                outcome: LiveProbeOutcome {
                    source: Some(QuotaSource::LiveProbe),
                    window_5h_remaining_pct: Some(90.0),
                    ..Default::default()
                },
            },
        );

        let snapshot = tracker.snapshot(adapter, "codex_local").await;
        assert!(!snapshot.exact_probe_opt_in);
        assert_eq!(snapshot.source, QuotaSource::UserCalibrated);
        assert_eq!(snapshot.window_5h_remaining_pct, None);
        assert_eq!(snapshot.weekly_remaining_pct, Some(64.0));
        assert_eq!(snapshot.weekly_resets_at, None);
    }
}
