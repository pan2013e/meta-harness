use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::{Mutex, RwLock};
use tracing::{debug, info};

use mh_core::{AdapterConfig, DiscoveredConfigOption, PermissionPolicy, PermissionRule};

use crate::AcpError;
use crate::broker::{BrokerCallbacks, MemberBrokerContext};
use crate::client::AcpClient;
use crate::platform::find_cli_binary;
use crate::protocol::{McpEnvVar, McpServerSpec};

#[derive(Debug, Clone)]
pub struct MemberSpawnSpec {
    pub session_id: String,
    pub member_handle: String,
    pub adapter_id: String,
    pub adapter: AdapterConfig,
    pub project_root: PathBuf,
    pub member_worktree: PathBuf,
    pub permission_policy: PermissionPolicy,
    pub existing_acp_session_id: Option<String>,
    pub extra_env: Vec<(String, String)>,
    pub mcp_bridge_spec: Option<McpServerSpec>,
    pub rules: Arc<RwLock<Vec<PermissionRule>>>,
}

pub struct ManagedMemberClient {
    pub client: Arc<AcpClient>,
    pub active_turn_id: Arc<RwLock<Option<String>>>,
    pub last_used: Mutex<Instant>,
    member_worktree: PathBuf,
    permission_policy: PermissionPolicy,
    adapter_id: String,
}

type ClientMap = HashMap<(String, String), Arc<ManagedMemberClient>>;

/// Manages lazy spawning, reuse, cancellation, and idle hibernation of ACP agent subprocesses
/// keyed by `(session_id, member_handle)`.
pub struct AcpSupervisor {
    clients: Arc<Mutex<ClientMap>>,
    closed_sessions: Arc<RwLock<HashSet<String>>>,
    idle_ttl: Duration,
}

impl AcpSupervisor {
    pub fn new(idle_process_ttl_secs: u64) -> Arc<Self> {
        let sup = Arc::new(Self {
            clients: Arc::new(Mutex::new(HashMap::new())),
            closed_sessions: Arc::new(RwLock::new(HashSet::new())),
            idle_ttl: Duration::from_secs(idle_process_ttl_secs.max(30)),
        });

        let weak = Arc::downgrade(&sup);
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(Duration::from_secs(60));
            loop {
                ticker.tick().await;
                let Some(this) = weak.upgrade() else {
                    break;
                };
                this.reap_idle_clients().await;
            }
        });

        sup
    }

    pub async fn get_or_spawn(
        &self,
        spec: MemberSpawnSpec,
        callbacks: Arc<dyn BrokerCallbacks>,
    ) -> Result<Arc<ManagedMemberClient>, AcpError> {
        if self.closed_sessions.read().await.contains(&spec.session_id) {
            return Err(AcpError::Cancelled);
        }
        let key = (spec.session_id.clone(), spec.member_handle.clone());
        let retired = {
            let mut clients = self.clients.lock().await;
            if let Some(existing) = clients.get(&key)
                && existing.member_worktree == spec.member_worktree
                && existing.permission_policy == spec.permission_policy
                && existing.adapter_id == spec.adapter_id
                && existing.client.is_alive().await
            {
                *existing.last_used.lock().await = Instant::now();
                existing
                    .client
                    .update_turn_context(callbacks, spec.rules.read().await.clone())
                    .await;
                return Ok(Arc::clone(existing));
            }
            clients.remove(&key)
        };
        if let Some(retired) = retired {
            retired.client.shutdown().await;
        }

        let started = Instant::now();
        let (command, args) = resolve_adapter_command(&spec.adapter)?;
        let active_turn_id = Arc::new(RwLock::new(None));
        let broker_ctx = MemberBrokerContext {
            session_id: spec.session_id.clone(),
            member_handle: spec.member_handle.clone(),
            project_root: spec.project_root.clone(),
            member_worktree: spec.member_worktree.clone(),
            permission_policy: spec.permission_policy,
            active_turn_id: Arc::clone(&active_turn_id),
            rules: spec.rules,
        };

        let client = AcpClient::spawn(
            &command,
            &args,
            &spec.member_worktree,
            &spec.extra_env,
            broker_ctx,
            callbacks,
        )
        .await?;

        let mcp_servers = spec.mcp_bridge_spec.into_iter().collect::<Vec<_>>();
        let setup = async {
            let init = client.initialize().await?;
            let initialize_ms = started.elapsed().as_millis();
            let resumed = if init.agent_capabilities.load_session
                && let Some(prev_sid) = spec.existing_acp_session_id.as_deref()
            {
                client
                    .load_session(prev_sid, &spec.member_worktree, mcp_servers.clone())
                    .await
                    .is_ok()
            } else {
                false
            };
            if !resumed {
                client
                    .new_session(&spec.member_worktree, mcp_servers)
                    .await?;
            }
            debug!(adapter = %spec.adapter_id, initialize_ms, ready_ms = started.elapsed().as_millis(), "ACP session ready");
            Ok::<_, AcpError>(())
        };
        let setup_result = tokio::time::timeout(Duration::from_secs(60), setup)
            .await
            .unwrap_or_else(|_| {
                Err(AcpError::Timeout(format!(
                    "{} did not initialize an ACP session within 60 seconds",
                    spec.adapter.display_name
                )))
            });
        if let Err(error) = setup_result {
            client.shutdown().await;
            return Err(error);
        }

        let managed = Arc::new(ManagedMemberClient {
            client,
            active_turn_id,
            last_used: Mutex::new(Instant::now()),
            member_worktree: spec.member_worktree,
            permission_policy: spec.permission_policy,
            adapter_id: spec.adapter_id,
        });

        let inserted = {
            let mut guard = self.clients.lock().await;
            if self.closed_sessions.read().await.contains(&spec.session_id) {
                false
            } else {
                guard.insert(key, Arc::clone(&managed));
                true
            }
        };
        if !inserted {
            managed.client.shutdown().await;
            return Err(AcpError::Cancelled);
        }
        Ok(managed)
    }

    pub async fn get_discovered_options(
        &self,
        session_id: &str,
        member_handle: &str,
    ) -> Vec<DiscoveredConfigOption> {
        let guard = self.clients.lock().await;
        if let Some(m) = guard.get(&(session_id.to_string(), member_handle.to_string())) {
            m.client.config_options.read().await.clone()
        } else {
            Vec::new()
        }
    }

    pub async fn cancel_session(&self, session_id: &str) {
        let targets: Vec<Arc<ManagedMemberClient>> = {
            let guard = self.clients.lock().await;
            guard
                .iter()
                .filter(|((sid, _), _)| sid == session_id)
                .map(|(_, m)| Arc::clone(m))
                .collect()
        };
        for m in targets {
            let _ = m.client.cancel().await;
        }
    }

    pub async fn shutdown_session(&self, session_id: &str) {
        self.closed_sessions
            .write()
            .await
            .insert(session_id.to_string());
        let removed: Vec<Arc<ManagedMemberClient>> = {
            let mut guard = self.clients.lock().await;
            let keys: Vec<(String, String)> = guard
                .keys()
                .filter(|(sid, _)| sid == session_id)
                .cloned()
                .collect();
            keys.into_iter().filter_map(|k| guard.remove(&k)).collect()
        };
        for m in removed {
            m.client.shutdown().await;
        }
    }

    pub async fn shutdown_all(&self) {
        let removed: Vec<Arc<ManagedMemberClient>> = {
            let mut guard = self.clients.lock().await;
            guard.drain().map(|(_, m)| m).collect()
        };
        for m in removed {
            m.client.shutdown().await;
        }
    }

    pub async fn active_process_count(&self) -> usize {
        self.clients.lock().await.len()
    }

    async fn reap_idle_clients(&self) {
        let mut to_shutdown = Vec::new();
        {
            let mut guard = self.clients.lock().await;
            let now = Instant::now();
            let mut remove_keys = Vec::new();
            for (k, m) in guard.iter() {
                let busy = m.active_turn_id.read().await.is_some();
                if !busy && now.duration_since(*m.last_used.lock().await) > self.idle_ttl {
                    remove_keys.push(k.clone());
                }
            }
            for k in remove_keys {
                if let Some(m) = guard.remove(&k) {
                    to_shutdown.push((k, m));
                }
            }
        }

        for ((sid, handle), m) in to_shutdown {
            info!("hibernating idle ACP client {sid}/{handle}");
            m.client.shutdown().await;
        }
    }
}

pub fn resolve_adapter_command(adapter: &AdapterConfig) -> Result<(String, Vec<String>), AcpError> {
    if !adapter.enabled {
        return Err(AcpError::CommandNotFound(format!(
            "{} is disabled",
            adapter.display_name
        )));
    }
    let mut missing = if adapter.command.is_empty() {
        adapter.display_name.clone()
    } else {
        adapter.command.clone()
    };
    if let Some((package, executable)) = default_npm_adapter(adapter) {
        if let Some(bin) = find_cli_binary(executable) {
            return Ok((bin.to_string_lossy().into_owned(), Vec::new()));
        }
        if executable == "claude-agent-acp" && find_cli_binary("claude").is_none() {
            missing = "Claude Code CLI or claude-agent-acp".into();
        } else {
            if let Some(bin) =
                npm_cache_dir().and_then(|cache| cached_npm_adapter(&cache, package, executable))
            {
                return Ok((bin.to_string_lossy().into_owned(), Vec::new()));
            }
            if let Some(npx) = find_cli_binary("npx") {
                return Ok((
                    npx.to_string_lossy().into_owned(),
                    vec!["--prefer-offline".into(), "-y".into(), package.into()],
                ));
            }
        }
    } else if !adapter.command.is_empty()
        && let Some(bin) = find_cli_binary(&adapter.command)
    {
        return Ok((bin.to_string_lossy().into_owned(), adapter.args.clone()));
    }
    for fb in &adapter.fallback_commands {
        if !fb.command.is_empty()
            && let Some(bin) = find_cli_binary(&fb.command)
        {
            return Ok((bin.to_string_lossy().into_owned(), fb.args.clone()));
        }
    }
    Err(AcpError::CommandNotFound(missing))
}

fn default_npm_adapter(adapter: &AdapterConfig) -> Option<(&'static str, &'static str)> {
    let (package, executable) = match adapter.quota_provider.as_str() {
        "codex_local" => ("@agentclientprotocol/codex-acp", "codex-acp"),
        "claude_local" => ("@agentclientprotocol/claude-agent-acp", "claude-agent-acp"),
        _ => return None,
    };
    (adapter.command == "npx" && adapter.args == ["-y", package]).then_some((package, executable))
}

fn npm_cache_dir() -> Option<PathBuf> {
    std::env::var_os("npm_config_cache")
        .map(PathBuf::from)
        .or_else(|| {
            if cfg!(windows) {
                dirs::cache_dir().map(|path| path.join("npm-cache"))
            } else {
                dirs::home_dir().map(|path| path.join(".npm"))
            }
        })
}

fn cached_npm_adapter(cache: &Path, package: &str, executable: &str) -> Option<PathBuf> {
    let shim_name = if cfg!(windows) {
        format!("{executable}.cmd")
    } else {
        executable.into()
    };
    std::fs::read_dir(cache.join("_npx"))
        .ok()?
        .flatten()
        .filter_map(|entry| {
            let manifest = entry.path().join("package.json");
            let data: serde_json::Value =
                serde_json::from_slice(&std::fs::read(&manifest).ok()?).ok()?;
            let packages = data.pointer("/_npx/packages")?.as_array()?;
            if packages.len() != 1 || packages[0].as_str() != Some(package) {
                return None;
            }
            let modules = entry.path().join("node_modules");
            let shim = modules.join(".bin").join(&shim_name);
            (shim.is_file() && modules.join(package).join("package.json").is_file())
                .then(|| (manifest.metadata().and_then(|m| m.modified()).ok(), shim))
        })
        .max()
        .map(|(_, shim)| shim)
}

pub fn build_mcp_bridge_spec(
    self_exe: &Path,
    server_mcp_url: &str,
    session_id: &str,
    caller_handle: &str,
    bridge_token: &str,
) -> McpServerSpec {
    McpServerSpec {
        name: "meta-harness-switch".into(),
        command: dunce::simplified(self_exe).to_string_lossy().into_owned(),
        args: vec![
            "mcp-bridge".into(),
            "--url".into(),
            server_mcp_url.into(),
            "--session".into(),
            session_id.into(),
            "--caller".into(),
            caller_handle.into(),
        ],
        env: vec![McpEnvVar {
            name: "MH_BRIDGE_TOKEN".into(),
            value: bridge_token.into(),
        }],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cached_codex_launch_uses_only_a_matching_npm_exec_installation() {
        let root = tempfile::tempdir().unwrap();
        let cache = root.path();
        let package = "@agentclientprotocol/codex-acp";
        let executable = "codex-acp";
        let app = cache.join("_npx/sample");
        let shims = app.join("node_modules/.bin");
        std::fs::create_dir_all(&shims).unwrap();
        let shim = shims.join(if cfg!(windows) {
            "codex-acp.cmd"
        } else {
            "codex-acp"
        });
        std::fs::write(&shim, "cached shim").unwrap();
        let installed = app.join("node_modules").join(package);
        std::fs::create_dir_all(&installed).unwrap();
        std::fs::write(installed.join("package.json"), "{}").unwrap();
        std::fs::write(
            app.join("package.json"),
            r#"{"_npx":{"packages":["@agentclientprotocol/codex-acp@1.1.13"]}}"#,
        )
        .unwrap();
        assert_eq!(cached_npm_adapter(cache, package, executable), None);
        std::fs::write(
            app.join("package.json"),
            r#"{"_npx":{"packages":["@agentclientprotocol/codex-acp"]}}"#,
        )
        .unwrap();
        assert_eq!(cached_npm_adapter(cache, package, executable), Some(shim));
    }
}
