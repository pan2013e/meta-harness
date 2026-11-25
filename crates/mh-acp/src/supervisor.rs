use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::{Mutex, RwLock};
use tracing::info;

use mh_core::{AdapterConfig, DiscoveredConfigOption, PermissionPolicy, PermissionRule};

use crate::AcpError;
use crate::broker::{BrokerCallbacks, MemberBrokerContext};
use crate::client::AcpClient;
use crate::platform::probe_executable;
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
        {
            let guard = self.clients.lock().await;
            if let Some(existing) = guard.get(&key)
                && existing.client.is_alive().await
            {
                *existing.last_used.lock().await = Instant::now();
                return Ok(Arc::clone(existing));
            }
        }

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

        let init = client.initialize().await?;
        let mcp_servers = spec.mcp_bridge_spec.into_iter().collect::<Vec<_>>();

        let mut loaded = false;
        if init.agent_capabilities.load_session
            && let Some(prev_sid) = spec.existing_acp_session_id.as_deref()
            && client
                .load_session(prev_sid, &spec.member_worktree, mcp_servers.clone())
                .await
                .is_ok()
        {
            loaded = true;
        }
        if !loaded {
            client
                .new_session(&spec.member_worktree, mcp_servers)
                .await?;
        }

        let managed = Arc::new(ManagedMemberClient {
            client,
            active_turn_id,
            last_used: Mutex::new(Instant::now()),
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
    if !adapter.command.is_empty() && probe_executable(&adapter.command).is_some() {
        return Ok((adapter.command.clone(), adapter.args.clone()));
    }
    for fb in &adapter.fallback_commands {
        if probe_executable(&fb.command).is_some() {
            return Ok((fb.command.clone(), fb.args.clone()));
        }
    }
    if adapter.command.is_empty() {
        return Err(AcpError::CommandNotFound(adapter.display_name.clone()));
    }
    Ok((adapter.command.clone(), adapter.args.clone()))
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
