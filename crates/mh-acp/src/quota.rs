use std::collections::HashMap;
use std::path::PathBuf;
use std::time::SystemTime;

use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

use mh_core::{AppConfig, QuotaSnapshot, QuotaSource};

use crate::platform::probe_executable;

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

#[derive(Debug, Clone, Default)]
struct AdapterQuotaPrefs {
    plan_label: Option<String>,
    exact_probe_opt_in: bool,
    manual_5h_pct: Option<f32>,
    manual_weekly_pct: Option<f32>,
    rate_limited_until: Option<i64>,
}

/// Layered adapter-level subscription quota tracker across Windows, macOS, and Linux.
/// Tracks shared usage per `adapter_id` (so Orchestrator + Executor personas share one pool).
pub struct QuotaTracker {
    prefs: RwLock<HashMap<String, AdapterQuotaPrefs>>,
    turns: RwLock<HashMap<String, Vec<TurnUsageEntry>>>,
}

impl QuotaTracker {
    pub fn new() -> Self {
        Self {
            prefs: RwLock::new(HashMap::new()),
            turns: RwLock::new(HashMap::new()),
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
        let mut guard = self.prefs.write().await;
        let entry = guard.entry(adapter_id.to_string()).or_default();
        entry.plan_label = plan_label;
        entry.exact_probe_opt_in = exact_probe_opt_in;
        entry.manual_5h_pct = manual_5h_pct;
        entry.manual_weekly_pct = manual_weekly_pct;
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

        let local_cli_5h_est = scan_local_cli_5h_tokens(quota_provider);
        let total_5h_tokens = mh_5h_tokens + local_cli_5h_est;

        let (budget_5h, budget_7d) = plan_token_budgets(pref.plan_label.as_deref());

        let is_rate_limited = pref.rate_limited_until.is_some_and(|until| until > now);

        let (window_5h_remaining_pct, weekly_remaining_pct, source) = if is_rate_limited {
            (Some(0.0), Some(0.0), QuotaSource::Cooldown429)
        } else if let Some(manual5) = pref.manual_5h_pct {
            let decay = ((mh_5h_tokens as f32 / budget_5h as f32) * 100.0).min(manual5);
            let w5 = (manual5 - decay).clamp(0.0, 100.0);
            let ww = pref.manual_weekly_pct.map(|m| m.clamp(0.0, 100.0));
            (Some(w5), ww, QuotaSource::UserCalibrated)
        } else {
            let used_5h_pct = ((total_5h_tokens as f32 / budget_5h as f32) * 100.0).min(100.0);
            let used_7d_pct = ((mh_7d_tokens as f32 / budget_7d as f32) * 100.0).min(100.0);
            (
                Some((100.0 - used_5h_pct).clamp(0.0, 100.0)),
                Some((100.0 - used_7d_pct).clamp(0.0, 100.0)),
                QuotaSource::LocalLogEstimate,
            )
        };

        QuotaSnapshot {
            adapter_id: adapter_id.to_string(),
            plan_label: pref.plan_label,
            window_5h_remaining_pct,
            window_5h_resets_at: Some(now + 5 * 3600 * 1000),
            weekly_remaining_pct,
            weekly_resets_at: Some(now + 7 * 24 * 3600 * 1000),
            local_5h_tokens_est: total_5h_tokens,
            rate_limited_until: pref.rate_limited_until.filter(|&u| u > now),
            exact_probe_opt_in: pref.exact_probe_opt_in,
            source,
            updated_at: now,
        }
    }

    pub async fn all_snapshots(&self, config: &AppConfig) -> Vec<QuotaSnapshot> {
        let mut out = Vec::with_capacity(config.adapters.len());
        for (id, cfg) in &config.adapters {
            if cfg.enabled {
                out.push(self.snapshot(id, &cfg.quota_provider).await);
            }
        }
        out
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

fn scan_local_cli_5h_tokens(quota_provider: &str) -> u64 {
    let Some(home) = dirs::home_dir() else {
        return 0;
    };
    let dir: PathBuf = match quota_provider {
        "codex_local" => std::env::var_os("CODEX_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".codex"))
            .join("sessions"),
        "claude_local" => std::env::var_os("CLAUDE_CONFIG_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".claude"))
            .join("projects"),
        "google_local" => home.join(".gemini"),
        _ => return 0,
    };

    if !dir.exists() {
        return 0;
    }

    let cutoff = SystemTime::now()
        .checked_sub(std::time::Duration::from_secs(5 * 3600))
        .unwrap_or(SystemTime::UNIX_EPOCH);

    let mut bytes_recent = 0u64;
    collect_recent_log_bytes(&dir, cutoff, 0, &mut bytes_recent);
    // Conservative approximation: ~12 bytes of verbose JSONL session log per 1 model token
    bytes_recent / 12
}

fn collect_recent_log_bytes(dir: &PathBuf, cutoff: SystemTime, depth: u8, acc: &mut u64) {
    if depth > 4 {
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in rd.flatten().take(200) {
        let p = entry.path();
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        if meta.is_dir() {
            collect_recent_log_bytes(&p, cutoff, depth + 1, acc);
        } else if meta.is_file()
            && let Ok(mtime) = meta.modified()
            && mtime >= cutoff
        {
            *acc = acc.saturating_add(meta.len().min(4_000_000));
        }
    }
}

pub fn probe_installed_adapters(config: &AppConfig) -> Vec<AdapterProbeStatus> {
    let home = dirs::home_dir();
    let mut results = Vec::new();

    for (id, cfg) in &config.adapters {
        let mut resolved = if !cfg.command.is_empty() {
            probe_executable(&cfg.command).map(|p| p.to_string_lossy().into_owned())
        } else {
            None
        };
        if resolved.is_none() {
            for fb in &cfg.fallback_commands {
                if let Some(p) = probe_executable(&fb.command) {
                    resolved = Some(p.to_string_lossy().into_owned());
                    break;
                }
            }
        }

        // For `npx`, check if underlying CLI or npm is present; specifically for codex/claude check npx + CLI
        let (auth_detected, auth_hint) = match cfg.quota_provider.as_str() {
            "codex_local" => {
                let codex_dir = std::env::var_os("CODEX_HOME")
                    .map(PathBuf::from)
                    .or_else(|| home.as_ref().map(|h| h.join(".codex")));
                let ok = codex_dir
                    .as_ref()
                    .is_some_and(|d| d.join("auth.json").exists());
                (
                    ok,
                    if ok {
                        "Codex CLI credentials found (~/.codex)".into()
                    } else {
                        "Run `codex login` to connect ChatGPT/Codex subscription".into()
                    },
                )
            }
            "claude_local" => {
                let claude_dir = std::env::var_os("CLAUDE_CONFIG_DIR")
                    .map(PathBuf::from)
                    .or_else(|| home.as_ref().map(|h| h.join(".claude")));
                let ok = claude_dir.as_ref().is_some_and(|d| d.exists());
                (
                    ok,
                    if ok {
                        "Claude CLI directory detected (~/.claude)".into()
                    } else {
                        "Install Claude Code CLI and run `claude login`".into()
                    },
                )
            }
            "google_local" => {
                let gem_dir = home.as_ref().map(|h| h.join(".gemini"));
                let ok = gem_dir.as_ref().is_some_and(|d| d.exists());
                (
                    ok,
                    if ok {
                        "Google/Antigravity directory detected (~/.gemini)".into()
                    } else {
                        "Install `agy` or `gemini` CLI and sign in".into()
                    },
                )
            }
            _ => (true, "Custom ACP adapter".into()),
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
