use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use mh_core::{
    AdapterConfig, AdapterModelCatalog, ConfigOptionChoice, DiscoveredConfigOption,
    ModelChoiceDetail, PendingPermissionView, PermissionPolicy, PermissionRule,
    TrajectoryEventKind,
};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::process::{ChildStdout, Command};
use tokio::sync::{Mutex, RwLock};
use tracing::{debug, info};

use crate::AcpError;
use crate::broker::{BrokerCallbacks, MemberBrokerContext, PermissionResolution};
use crate::client::{AcpClient, TurnStreamListener};
use crate::platform::{ProcessTreeGuard, find_cli_binary, prepare_isolated_command};
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
    profile_dir: Option<PathBuf>,
}

type ClientMap = HashMap<(String, String), Arc<ManagedMemberClient>>;

/// Manages lazy spawning, reuse, cancellation, and idle hibernation of ACP agent subprocesses
/// keyed by `(session_id, member_handle)`.
pub struct AcpSupervisor {
    clients: Arc<Mutex<ClientMap>>,
    closed_sessions: Arc<RwLock<HashSet<String>>>,
    catalog_cache: Mutex<HashMap<String, (Instant, AdapterModelCatalog)>>,
    adapter_activity: Mutex<HashMap<String, Arc<RwLock<()>>>>,
    idle_ttl: Duration,
}

impl AcpSupervisor {
    pub fn new(idle_process_ttl_secs: u64) -> Arc<Self> {
        let sup = Arc::new(Self {
            clients: Arc::new(Mutex::new(HashMap::new())),
            closed_sessions: Arc::new(RwLock::new(HashSet::new())),
            catalog_cache: Mutex::new(HashMap::new()),
            adapter_activity: Mutex::new(HashMap::new()),
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
                && existing.profile_dir == spec.adapter.profile_dir
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
        let (command, mut args) = resolve_adapter_command(&spec.adapter)?;
        let mut extra_env = spec.extra_env;
        crate::profile::apply_profile(&spec.adapter, &command, &mut args, &mut extra_env).await?;
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
            &extra_env,
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
            profile_dir: spec.adapter.profile_dir,
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

    pub async fn get_latest_usage(
        &self,
        session_id: &str,
        member_handle: &str,
    ) -> Option<(u64, u64)> {
        let guard = self.clients.lock().await;
        if let Some(m) = guard.get(&(session_id.to_string(), member_handle.to_string())) {
            *m.client.latest_usage.read().await
        } else {
            None
        }
    }

    pub async fn adapter_activity(&self, id: &str) -> Arc<RwLock<()>> {
        Arc::clone(
            self.adapter_activity
                .lock()
                .await
                .entry(id.to_string())
                .or_default(),
        )
    }

    pub async fn retire_member(&self, session_id: &str, handle: &str) {
        let removed = self
            .clients
            .lock()
            .await
            .remove(&(session_id.into(), handle.into()));
        if let Some(member) = removed {
            member.client.shutdown().await;
        }
    }

    pub async fn retire_adapter(&self, adapter_id: &str) {
        let removed = {
            let mut clients = self.clients.lock().await;
            let keys: Vec<_> = clients
                .iter()
                .filter(|(_, member)| member.adapter_id == adapter_id)
                .map(|(key, _)| key.clone())
                .collect();
            keys.into_iter()
                .filter_map(|key| clients.remove(&key))
                .collect::<Vec<_>>()
        };
        for member in removed {
            member.client.shutdown().await;
        }
    }

    pub async fn invalidate_catalog(&self, adapter_id: &str) {
        self.catalog_cache.lock().await.remove(adapter_id);
    }

    pub async fn model_default_effort(&self, adapter_id: &str, model: &str) -> Option<String> {
        let lookup = |catalog: &AdapterModelCatalog| {
            catalog
                .model_details
                .iter()
                .find(|entry| entry.value.eq_ignore_ascii_case(model))
                .and_then(|entry| entry.default_effort.clone())
        };
        let cache = self.catalog_cache.lock().await;
        if let Some((_, catalog)) = cache.get(adapter_id) {
            lookup(catalog)
        } else {
            lookup(&builtin_adapter_catalog(adapter_id))
        }
    }

    /// Runs a short-lived ephemeral ACP turn (used for low-effort utility passes such as
    /// automatic session title generation) without disturbing active conversation clients.
    pub async fn run_ephemeral_prompt(
        &self,
        adapter: &AdapterConfig,
        cwd: &Path,
        effort: Option<&str>,
        prompt_text: &str,
        timeout_dur: Duration,
    ) -> Result<String, AcpError> {
        let (command, mut args) = resolve_adapter_command(adapter)?;
        let mut extra_env = Vec::new();
        crate::profile::apply_profile(adapter, &command, &mut args, &mut extra_env).await?;
        let active_turn_id = Arc::new(RwLock::new(Some("ephemeral_title".to_string())));
        let broker_ctx = MemberBrokerContext {
            session_id: "ephemeral".into(),
            member_handle: "orchestrator".into(),
            project_root: cwd.to_path_buf(),
            member_worktree: cwd.to_path_buf(),
            permission_policy: PermissionPolicy::Strict,
            active_turn_id,
            rules: Arc::new(RwLock::new(Vec::new())),
        };
        let callbacks = Arc::new(EphemeralCallbacks);
        let client =
            AcpClient::spawn(&command, &args, cwd, &extra_env, broker_ctx, callbacks).await?;
        let listener = Arc::new(NoopStreamListener);
        let res = tokio::time::timeout(timeout_dur, async {
            client.initialize().await?;
            client.new_session(cwd, Vec::new()).await?;
            if effort.is_some() {
                let _ = client.apply_model_and_effort(None, effort, None).await;
            }
            let (_, out) = client
                .prompt("ephemeral_title", prompt_text, listener)
                .await?;
            Ok::<String, AcpError>(out.final_text)
        })
        .await;
        client.shutdown().await;
        match res {
            Ok(inner) => inner,
            Err(_) => Err(AcpError::Timeout("ephemeral prompt timed out".into())),
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
        self.reset_session_clients(session_id).await;
    }

    pub async fn reopen_session(&self, session_id: &str) {
        self.closed_sessions.write().await.remove(session_id);
    }

    /// Shuts down active ACP child processes for `session_id` without marking the
    /// session permanently closed (used for context compaction).
    pub async fn reset_session_clients(&self, session_id: &str) {
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

    /// Resolves live model + reasoning effort metadata for an adapter (querying
    /// `codex app-server --stdio` `model/list` on Codex and caching for 10 minutes),
    /// falling back cleanly to `builtin_adapter_catalog`.
    pub async fn resolve_adapter_catalog(
        &self,
        adapter_id: &str,
        adapter: &AdapterConfig,
        refresh: bool,
    ) -> AdapterModelCatalog {
        {
            let cache = self.catalog_cache.lock().await;
            if !refresh
                && let Some((ts, cat)) = cache.get(adapter_id)
                && ts.elapsed() < Duration::from_secs(600)
            {
                return cat.clone();
            }
        }

        let mut catalog = builtin_adapter_catalog(adapter_id);
        let Ok(_activity) = self.adapter_activity(adapter_id).await.try_read_owned() else {
            return catalog;
        };
        if adapter.quota_provider == "codex_local" && adapter.enabled {
            if let Some(live) = probe_codex_models(adapter).await {
                catalog = live;
                catalog.adapter_id = adapter_id.to_string();
            }
            if let Some(cfg_model) = read_codex_configured_model(adapter.profile_dir.as_deref())
                && !catalog
                    .models
                    .iter()
                    .any(|m| m.value.eq_ignore_ascii_case(&cfg_model))
            {
                catalog.models.insert(
                    0,
                    ConfigOptionChoice {
                        value: cfg_model.clone(),
                        label: cfg_model.clone(),
                    },
                );
                catalog.model_details.insert(
                    0,
                    ModelChoiceDetail {
                        value: cfg_model.clone(),
                        label: cfg_model,
                        description: "Configured in the Codex profile".into(),
                        is_default: false,
                        default_effort: None,
                        supported_efforts: catalog.efforts.clone(),
                    },
                );
            }
        }
        if adapter.quota_provider == "google_local"
            && let Ok((command, _)) = resolve_adapter_command(adapter)
            && crate::profile::is_agy(&command)
        {
            let live = if adapter.profile_dir.is_some() {
                probe_agy_profile_catalog(adapter, adapter_id).await
            } else {
                crate::antigravity::run_agy(&command, None, &["models"])
                    .await
                    .and_then(|output| crate::antigravity::model_catalog(&output, adapter_id))
            };
            if let Some(live) = live {
                catalog = live;
            }
        }

        self.catalog_cache
            .lock()
            .await
            .insert(adapter_id.to_string(), (Instant::now(), catalog.clone()));
        catalog
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

async fn probe_agy_profile_catalog(
    adapter: &AdapterConfig,
    adapter_id: &str,
) -> Option<AdapterModelCatalog> {
    if !crate::quota::has_google_cli_auth(adapter.profile_dir.as_deref(), true) {
        return None;
    }
    let temp = tempfile::tempdir().ok()?;
    let (command, mut args) = resolve_adapter_command(adapter).ok()?;
    let mut env = vec![("AGY_CLI_DISABLE_AUTO_UPDATE".into(), "true".into())];
    crate::profile::apply_profile(adapter, &command, &mut args, &mut env)
        .await
        .ok()?;
    let cwd = temp.path();
    let context = MemberBrokerContext {
        session_id: "catalog".into(),
        member_handle: adapter_id.into(),
        project_root: cwd.to_path_buf(),
        member_worktree: cwd.to_path_buf(),
        permission_policy: PermissionPolicy::Strict,
        active_turn_id: Arc::new(RwLock::new(None)),
        rules: Arc::new(RwLock::new(Vec::new())),
    };
    let client = AcpClient::spawn(
        &command,
        &args,
        cwd,
        &env,
        context,
        Arc::new(EphemeralCallbacks),
    )
    .await
    .ok()?;
    let result = tokio::time::timeout(Duration::from_secs(10), async {
        client.initialize().await.ok()?;
        let response = client.new_session(cwd, Vec::new()).await.ok()?;
        let models = response.models?;
        let rows = models
            .get("availableModels")?
            .as_array()?
            .iter()
            .filter_map(|model| {
                Some(format!(
                    "{}\t{}",
                    model.get("modelId")?.as_str()?,
                    model.get("name")?.as_str()?
                ))
            })
            .collect::<Vec<_>>()
            .join("\n");
        crate::antigravity::model_catalog(&rows, adapter_id)
    })
    .await
    .ok()
    .flatten();
    client.shutdown().await;
    result
}

fn format_effort_label(val: &str) -> String {
    match val.to_ascii_lowercase().as_str() {
        "low" => "Low".into(),
        "medium" => "Medium".into(),
        "high" => "High".into(),
        "xhigh" => "Extra High".into(),
        "max" => "Max".into(),
        "ultra" => "Ultra".into(),
        other => other.to_string(),
    }
}

pub(crate) fn codex_model_command(adapter: &AdapterConfig) -> Option<Command> {
    if let Some(override_path) = std::env::var_os("CODEX_PATH")
        .and_then(|path| path.into_string().ok())
        .filter(|path| !path.is_empty())
    {
        return find_cli_binary(&override_path).map(Command::new);
    }
    if let Ok((launcher, _)) = resolve_adapter_command(adapter) {
        let launcher = PathBuf::from(launcher);
        let actual = dunce::canonicalize(&launcher).unwrap_or_else(|_| launcher.clone());
        let bundled = actual
            .ancestors()
            .chain(launcher.ancestors())
            .find_map(|parent| {
                let node_modules = if parent
                    .file_name()
                    .is_some_and(|name| name == "node_modules")
                {
                    parent.to_path_buf()
                } else {
                    parent.join("node_modules")
                };
                [
                    node_modules.join(
                        "@agentclientprotocol/codex-acp/node_modules/@openai/codex/bin/codex.js",
                    ),
                    node_modules.join("@openai/codex/bin/codex.js"),
                ]
                .into_iter()
                .find(|entry| entry.is_file())
            });
        if let Some(bundle) = bundled
            && let Some(node) = find_cli_binary("node")
        {
            let mut command = Command::new(node);
            command.arg(dunce::simplified(&bundle));
            return Some(command);
        }
    }
    find_cli_binary("codex").map(Command::new)
}

async fn read_codex_result(lines: &mut Lines<BufReader<ChildStdout>>, id: i64) -> Option<Value> {
    while let Some(line) = lines.next_line().await.ok().flatten() {
        if let Ok(mut frame) = serde_json::from_str::<Value>(&line)
            && frame.get("id").and_then(Value::as_i64) == Some(id)
        {
            return frame.get_mut("result").map(Value::take);
        }
    }
    None
}

async fn probe_codex_models(adapter: &AdapterConfig) -> Option<AdapterModelCatalog> {
    let mut cmd = codex_model_command(adapter)?;
    if let Some(profile) = adapter.profile_dir.as_deref() {
        cmd.env("CODEX_HOME", dunce::simplified(profile));
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
        read_codex_result(&mut lines, 1).await?;

        stdin
            .write_all(
                br#"{"jsonrpc":"2.0","method":"initialized"}
"#,
            )
            .await
            .ok()?;
        stdin.flush().await.ok()?;

        let mut models = Vec::new();
        let mut cursor: Option<String> = None;
        let mut seen = HashSet::new();
        for id in 2..=17 {
            let request = json!({"jsonrpc":"2.0", "id":id, "method":"model/list", "params":{"cursor":cursor, "limit":100, "includeHidden":false}});
            stdin
                .write_all(format!("{request}\n").as_bytes())
                .await
                .ok()?;
            stdin.flush().await.ok()?;
            let mut page = read_codex_result(&mut lines, id).await?;
            models.append(page.get_mut("data")?.as_array_mut()?);
            cursor = page
                .get("nextCursor")
                .and_then(Value::as_str)
                .map(str::to_string);
            match &cursor {
                None => return parse_codex_models_result(&json!({"data": models})),
                Some(next) if !seen.insert(next.clone()) => return None,
                _ => {}
            }
        }
        None
    };

    let catalog = tokio::time::timeout(Duration::from_secs(8), rpc_fut)
        .await
        .ok()
        .flatten();

    guard.kill_tree();
    let _ = tokio::time::timeout(Duration::from_millis(200), child.wait()).await;
    catalog
}

fn parse_codex_models_result(result: &Value) -> Option<AdapterModelCatalog> {
    let data = result.get("data")?.as_array()?;
    let mut models = Vec::new();
    let mut model_details = Vec::new();
    let mut effort_union: Vec<ConfigOptionChoice> = Vec::new();
    let mut seen_efforts: HashSet<String> = HashSet::new();
    let mut seen_models = HashSet::new();

    for entry in data {
        if entry.get("hidden").and_then(Value::as_bool) == Some(true) {
            continue;
        }
        let Some(id) = ["model", "id"].into_iter().find_map(|key| {
            entry
                .get(key)
                .and_then(Value::as_str)
                .filter(|id| !id.is_empty())
        }) else {
            continue;
        };
        if !seen_models.insert(id.to_string()) {
            continue;
        }
        let id = id.to_string();
        let display = entry
            .get("displayName")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .unwrap_or(&id)
            .to_string();
        let description = entry
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let is_default = entry
            .get("isDefault")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let default_effort = entry
            .get("defaultReasoningEffort")
            .and_then(Value::as_str)
            .map(str::to_string);

        let mut supported = Vec::new();
        if let Some(arr) = entry
            .get("supportedReasoningEfforts")
            .and_then(Value::as_array)
        {
            for e in arr {
                if let Some(val) = e.get("reasoningEffort").and_then(Value::as_str) {
                    let ch = ConfigOptionChoice {
                        value: val.to_string(),
                        label: format_effort_label(val),
                    };
                    if seen_efforts.insert(val.to_string()) {
                        effort_union.push(ch.clone());
                    }
                    supported.push(ch);
                }
            }
        }

        models.push(ConfigOptionChoice {
            value: id.clone(),
            label: display.clone(),
        });
        model_details.push(ModelChoiceDetail {
            value: id,
            label: display,
            description,
            is_default,
            default_effort,
            supported_efforts: supported,
        });
    }

    if models.is_empty() {
        return None;
    }
    if effort_union.is_empty() {
        effort_union = ["low", "medium", "high", "xhigh", "max", "ultra"]
            .into_iter()
            .map(|e| ConfigOptionChoice {
                value: e.into(),
                label: format_effort_label(e),
            })
            .collect();
    }

    Some(AdapterModelCatalog {
        adapter_id: "codex".into(),
        models,
        model_details,
        efforts: effort_union,
        default_context_window: 200_000,
    })
}

pub fn builtin_adapter_catalog(adapter_id: &str) -> AdapterModelCatalog {
    let codex_efforts: Vec<ConfigOptionChoice> = ["low", "medium", "high", "xhigh", "max", "ultra"]
        .into_iter()
        .map(|v| ConfigOptionChoice {
            value: v.into(),
            label: format_effort_label(v),
        })
        .collect();

    let (models, model_details, efforts) = match adapter_id {
        "codex" => {
            let entries = [
                ("gpt-6-astra", "GPT-6-Astra", true, Some("medium")),
                ("gpt-5.6-sol", "GPT-5.6-Sol", false, Some("low")),
                ("gpt-5.6-terra", "GPT-5.6-Terra", false, Some("medium")),
                ("gpt-5.6-luna", "GPT-5.6-Luna", false, Some("medium")),
            ];
            let m = entries
                .iter()
                .map(|(v, l, _, _)| ConfigOptionChoice {
                    value: (*v).into(),
                    label: (*l).into(),
                })
                .collect();
            let d = entries
                .iter()
                .map(|(v, l, def, de)| ModelChoiceDetail {
                    value: (*v).into(),
                    label: (*l).into(),
                    description: String::new(),
                    is_default: *def,
                    default_effort: de.map(str::to_string),
                    supported_efforts: codex_efforts.clone(),
                })
                .collect();
            (m, d, codex_efforts)
        }
        "claude" => {
            let effs: Vec<ConfigOptionChoice> = ["low", "medium", "high"]
                .into_iter()
                .map(|v| ConfigOptionChoice {
                    value: v.into(),
                    label: format_effort_label(v),
                })
                .collect();
            let entries = [
                ("claude-sonnet-4-6", "Claude Sonnet 4.6", true),
                ("claude-opus-4-6", "Claude Opus 4.6", false),
                ("claude-haiku-4-5", "Claude Haiku 4.5", false),
            ];
            let m = entries
                .iter()
                .map(|(v, l, _)| ConfigOptionChoice {
                    value: (*v).into(),
                    label: (*l).into(),
                })
                .collect();
            let d = entries
                .iter()
                .map(|(v, l, def)| ModelChoiceDetail {
                    value: (*v).into(),
                    label: (*l).into(),
                    description: String::new(),
                    is_default: *def,
                    default_effort: Some("medium".into()),
                    supported_efforts: effs.clone(),
                })
                .collect();
            (m, d, effs)
        }
        _ => (Vec::new(), Vec::new(), Vec::new()),
    };

    let default_context_window = match adapter_id {
        "antigravity" => 1_000_000,
        _ => 200_000,
    };

    AdapterModelCatalog {
        adapter_id: adapter_id.to_string(),
        models,
        model_details,
        efforts,
        default_context_window,
    }
}

fn read_codex_configured_model(profile: Option<&Path>) -> Option<String> {
    let codex_dir = crate::profile::profile_home(profile, "CODEX_HOME", ".codex")?;
    let raw = std::fs::read_to_string(codex_dir.join("config.toml")).ok()?;
    let val: toml::Value = toml::from_str(&raw).ok()?;
    val.get("model")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

struct EphemeralCallbacks;

#[async_trait]
impl BrokerCallbacks for EphemeralCallbacks {
    async fn request_user_permission(
        &self,
        prompt: PendingPermissionView,
    ) -> Result<PermissionResolution, AcpError> {
        let reject_id = prompt
            .options
            .iter()
            .find(|o| o.kind.starts_with("reject"))
            .map(|o| o.option_id.clone())
            .unwrap_or_else(|| "reject_once".into());
        Ok(PermissionResolution::Selected {
            option_id: reject_id,
            allow_pattern: None,
        })
    }

    async fn record_trajectory_event(
        &self,
        _turn_id: &str,
        _kind: TrajectoryEventKind,
        _payload: Value,
    ) {
    }

    async fn record_touched_file(&self, _turn_id: &str, _rel_path: String) {}
}

struct NoopStreamListener;

#[async_trait]
impl TurnStreamListener for NoopStreamListener {
    async fn on_text_delta(&self, _turn_id: &str, _delta: String, _replace: bool) {}
    async fn on_thought_delta(&self, _turn_id: &str, _delta: String) {}
    async fn on_trajectory_event(
        &self,
        _turn_id: &str,
        _kind: TrajectoryEventKind,
        _payload: Value,
    ) {
    }
    async fn on_config_options_updated(&self, _options: Vec<DiscoveredConfigOption>) {}
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
    fn codex_catalog_keeps_public_model_slugs_and_skips_invalid_rows() {
        let catalog = parse_codex_models_result(&json!({"data":[
            {"hidden":true,"model":"internal"},
            {"description":"missing model"},
            {"id":"legacy-id","model":"latest-slug","displayName":"Latest","isDefault":true,
            "supportedReasoningEfforts":[{"reasoningEffort":"medium"}]},
            {"model":"latest-slug"}
        ]}))
        .unwrap();
        assert_eq!(catalog.models.len(), 1);
        assert_eq!(catalog.models[0].value, "latest-slug");
        assert!(catalog.model_details[0].is_default);
    }

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
