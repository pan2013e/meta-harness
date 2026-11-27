use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::{AppPaths, CoreError};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    pub server: ServerConfig,
    pub switch: SwitchConfig,
    pub git: GitConfig,
    pub memory: MemoryConfig,
    pub adapters: BTreeMap<String, AdapterConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ServerConfig {
    pub bind: String,
    pub open_browser_on_start: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SwitchConfig {
    pub max_agent_hops_per_user_turn: u32,
    pub peer_consult_timeout_secs: u64,
    pub idle_process_ttl_secs: u64,
    pub default_permission_policy: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct GitConfig {
    pub isolation_mode: String,
    pub auto_merge_clean_turns: bool,
    pub sync_idle_worktrees_after_merge: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct MemoryConfig {
    pub curation_mode: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AdapterConfig {
    pub display_name: String,
    pub accent_color: String,
    pub enabled: bool,
    pub command: String,
    pub args: Vec<String>,
    pub fallback_commands: Vec<AdapterFallbackCommand>,
    pub quota_provider: String,
    pub exact_quota_probe_opt_in: bool,
    pub profile_dir: Option<std::path::PathBuf>,
    pub characteristics: AdapterCharacteristics,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdapterFallbackCommand {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct AdapterCharacteristics {
    pub strengths: Vec<String>,
    pub latency_tier: String,
    pub context_window_tier: String,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            bind: "127.0.0.1:4410".to_string(),
            open_browser_on_start: true,
        }
    }
}

impl Default for SwitchConfig {
    fn default() -> Self {
        Self {
            max_agent_hops_per_user_turn: 6,
            peer_consult_timeout_secs: 180,
            idle_process_ttl_secs: 900,
            default_permission_policy: "require_approval".to_string(),
        }
    }
}

impl Default for GitConfig {
    fn default() -> Self {
        Self {
            isolation_mode: "worktree_per_member".to_string(),
            auto_merge_clean_turns: true,
            sync_idle_worktrees_after_merge: true,
        }
    }
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            curation_mode: "orchestrator_llm".to_string(),
        }
    }
}

impl Default for AdapterConfig {
    fn default() -> Self {
        Self {
            display_name: "Custom Agent".to_string(),
            accent_color: "#6366f1".to_string(),
            enabled: true,
            command: String::new(),
            args: Vec::new(),
            fallback_commands: Vec::new(),
            quota_provider: "manual".to_string(),
            exact_quota_probe_opt_in: false,
            profile_dir: None,
            characteristics: AdapterCharacteristics::default(),
        }
    }
}

impl Default for AppConfig {
    fn default() -> Self {
        let mut adapters = BTreeMap::new();

        adapters.insert(
            "codex".to_string(),
            AdapterConfig {
                display_name: "Codex".to_string(),
                accent_color: "#10a37f".to_string(),
                enabled: true,
                command: "npx".to_string(),
                args: vec!["-y".into(), "@agentclientprotocol/codex-acp".into()],
                fallback_commands: Vec::new(),
                quota_provider: "codex_local".to_string(),
                exact_quota_probe_opt_in: true,
                profile_dir: None,
                characteristics: AdapterCharacteristics {
                    strengths: vec![
                        "backend-systems".into(),
                        "rust".into(),
                        "refactoring".into(),
                        "debugging".into(),
                        "test-generation".into(),
                        "precise-diffs".into(),
                    ],
                    latency_tier: "medium".into(),
                    context_window_tier: "large".into(),
                },
            },
        );

        adapters.insert(
            "claude".to_string(),
            AdapterConfig {
                display_name: "Claude Code".to_string(),
                accent_color: "#d97757".to_string(),
                enabled: true,
                command: "npx".to_string(),
                args: vec!["-y".into(), "@agentclientprotocol/claude-agent-acp".into()],
                fallback_commands: Vec::new(),
                quota_provider: "claude_local".to_string(),
                exact_quota_probe_opt_in: true,
                profile_dir: None,
                characteristics: AdapterCharacteristics {
                    strengths: vec![
                        "architecture".into(),
                        "frontend-ui".into(),
                        "nuanced-design".into(),
                        "code-review".into(),
                        "conflict-resolution".into(),
                    ],
                    latency_tier: "fast".into(),
                    context_window_tier: "large".into(),
                },
            },
        );

        adapters.insert(
            "antigravity".to_string(),
            AdapterConfig {
                display_name: "Antigravity".to_string(),
                accent_color: "#4285f4".to_string(),
                enabled: true,
                command: "agy".to_string(),
                args: vec!["acp".into()],
                fallback_commands: vec![AdapterFallbackCommand {
                    command: "gemini".to_string(),
                    args: vec!["--acp".into()],
                }],
                quota_provider: "google_local".to_string(),
                exact_quota_probe_opt_in: true,
                profile_dir: None,
                characteristics: AdapterCharacteristics {
                    strengths: vec![
                        "broad-codebase-research".into(),
                        "long-context".into(),
                        "orchestration".into(),
                        "fullstack-synthesis".into(),
                        "planning".into(),
                    ],
                    latency_tier: "fast".into(),
                    context_window_tier: "ultra".into(),
                },
            },
        );

        Self {
            server: ServerConfig::default(),
            switch: SwitchConfig::default(),
            git: GitConfig::default(),
            memory: MemoryConfig::default(),
            adapters,
        }
    }
}

impl AppConfig {
    pub fn load_or_default(
        explicit_path: Option<&Path>,
        app_paths: &AppPaths,
    ) -> Result<Self, CoreError> {
        let candidates: Vec<std::path::PathBuf> = if let Some(p) = explicit_path {
            vec![p.to_path_buf()]
        } else {
            vec![
                Path::new("configs/harnesses.local.toml").to_path_buf(),
                app_paths.config_dir.join("harnesses.toml"),
            ]
        };

        for candidate in candidates {
            if candidate.exists() {
                let raw = std::fs::read_to_string(&candidate)?;
                let mut cfg: AppConfig = toml::from_str(&raw)
                    .map_err(|e| CoreError::Config(format!("{}: {e}", candidate.display())))?;
                // Preserve built-in defaults for any adapter not overridden
                let defaults = AppConfig::default();
                for (k, v) in defaults.adapters {
                    cfg.adapters.entry(k).or_insert(v);
                }
                for adapter in cfg.adapters.values_mut() {
                    if let Some(dir) = adapter.profile_dir.as_mut()
                        && dir.is_relative()
                    {
                        *dir = app_paths.data_dir.join(&*dir);
                    }
                }
                return Ok(cfg);
            } else if explicit_path.is_some() {
                return Err(CoreError::Config(format!(
                    "config file not found: {}",
                    candidate.display()
                )));
            }
        }

        Ok(Self::default())
    }
}
