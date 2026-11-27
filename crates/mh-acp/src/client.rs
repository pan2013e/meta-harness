use std::collections::{HashMap, VecDeque};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::{Mutex, RwLock, oneshot};
use tracing::{debug, warn};

use mh_core::{
    ChatImage, DiscoveredConfigOption, PermissionPolicy, PermissionRule, TrajectoryEventKind,
};

use crate::AcpError;
use crate::broker::{BrokerCallbacks, MemberBrokerContext, WorkspaceBroker};
use crate::platform::{ProcessTreeGuard, prepare_isolated_command, resolve_command};
use crate::protocol::{
    ClientCapabilities, ClientInfo, ContentBlock, FsCapabilities, InitializeParams,
    InitializeResult, JsonRpcNotification, JsonRpcRequest, JsonRpcResponse, LoadSessionParams,
    McpServerSpec, NewSessionParams, NewSessionResult, PromptParams, PromptResult,
    SetConfigOptionParams, add_legacy_modes, extract_update_text, parse_config_options,
    parse_new_session_options,
};
use crate::trajectory::{TrajectoryUpdate, parse_tool_update, push_update};

#[derive(Debug, Clone, Default)]
pub struct PromptTextOutput {
    pub final_text: String,
}

#[async_trait]
pub trait TurnStreamListener: Send + Sync {
    async fn on_text_delta(&self, turn_id: &str, delta: String, new_segment: bool);
    async fn on_thought_delta(&self, turn_id: &str, delta: String);
    async fn on_trajectory_event(&self, turn_id: &str, kind: TrajectoryEventKind, payload: Value);
    async fn on_config_options_updated(&self, options: Vec<DiscoveredConfigOption>);
}

struct TurnStreamAccumulator {
    turn_id: String,
    listener: Arc<dyn TurnStreamListener>,
    text_buf: String,
    after_step_boundary: bool,
    thought_buf: String,
    trajectory_buf: Vec<TrajectoryUpdate>,
    current_segment: String,
}

#[derive(Default)]
struct PendingRequests {
    closed: bool,
    calls: HashMap<i64, oneshot::Sender<Result<JsonRpcResponse, AcpError>>>,
}

impl PendingRequests {
    fn close(&mut self, cancelled: bool) {
        self.closed = true;
        for (_, waiter) in self.calls.drain() {
            let error = if cancelled {
                AcpError::Cancelled
            } else {
                AcpError::ProcessExited("ACP agent stdout closed".into())
            };
            let _ = waiter.send(Err(error));
        }
    }
}

pub struct AcpClient {
    guard: std::sync::Mutex<ProcessTreeGuard>,
    child: Mutex<Child>,
    stdin: Mutex<ChildStdin>,
    next_id: AtomicI64,
    pending: Arc<Mutex<PendingRequests>>,
    broker: Arc<WorkspaceBroker>,
    stderr_tail: Arc<Mutex<VecDeque<String>>>,
    active_stream: Arc<Mutex<Option<TurnStreamAccumulator>>>,
    stream_flush: Arc<Mutex<()>>,
    pub agent_capabilities: RwLock<InitializeResult>,
    pub acp_session_id: RwLock<Option<String>>,
    pub config_options: RwLock<Vec<DiscoveredConfigOption>>,
    session_defaults: RwLock<Vec<DiscoveredConfigOption>>,
    pub latest_usage: RwLock<Option<(u64, u64, Option<String>)>>,
    reader_task: tokio::task::JoinHandle<()>,
    coalescer_task: tokio::task::JoinHandle<()>,
}

impl AcpClient {
    pub(crate) async fn update_turn_context(
        &self,
        callbacks: Arc<dyn BrokerCallbacks>,
        rules: Vec<PermissionRule>,
        skill_read_roots: Vec<std::path::PathBuf>,
    ) {
        self.broker
            .update_turn_context(callbacks, rules, skill_read_roots)
            .await;
    }

    pub async fn spawn(
        command: &str,
        args: &[String],
        cwd: &Path,
        extra_env: &[(String, String)],
        broker_ctx: MemberBrokerContext,
        callbacks: Arc<dyn BrokerCallbacks>,
    ) -> Result<Arc<Self>, AcpError> {
        let (prog, resolved_args) = resolve_command(command, args)?;
        let mut cmd = Command::new(&prog);
        cmd.args(&resolved_args)
            .current_dir(dunce::simplified(cwd))
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        prepare_isolated_command(&mut cmd);
        for (k, v) in extra_env {
            cmd.env(k, v);
        }

        let mut child = cmd.spawn()?;
        let guard = ProcessTreeGuard::attach(&child);
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| AcpError::ProcessExited("missing child stdin".into()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| AcpError::ProcessExited("missing child stdout".into()))?;
        let stderr = child.stderr.take();

        let pending = Arc::new(Mutex::new(PendingRequests::default()));
        let stderr_tail = Arc::new(Mutex::new(VecDeque::<String>::with_capacity(100)));
        let active_stream = Arc::new(Mutex::new(None::<TurnStreamAccumulator>));
        let stream_flush = Arc::new(Mutex::new(()));
        let broker = Arc::new(WorkspaceBroker::new(broker_ctx, callbacks));

        if let Some(err_stream) = stderr {
            let tail_clone = Arc::clone(&stderr_tail);
            tokio::spawn(async move {
                let mut lines = BufReader::new(err_stream).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    let mut q = tail_clone.lock().await;
                    if q.len() >= 100 {
                        q.pop_front();
                    }
                    q.push_back(line);
                }
            });
        }

        // 25ms stream coalescer flushing text/thought chunks smoothly
        let stream_for_flush = Arc::clone(&active_stream);
        let flush_guard = Arc::clone(&stream_flush);
        let coalescer_task = tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_millis(25));
            loop {
                interval.tick().await;
                let _flush = flush_guard.lock().await;
                let (turn_id, listener, text_delta, thought_delta, trajectory) = {
                    let mut guard = stream_for_flush.lock().await;
                    let Some(acc) = guard.as_mut() else {
                        continue;
                    };
                    if acc.text_buf.is_empty()
                        && acc.thought_buf.is_empty()
                        && acc.trajectory_buf.is_empty()
                    {
                        continue;
                    }
                    (
                        acc.turn_id.clone(),
                        Arc::clone(&acc.listener),
                        std::mem::take(&mut acc.text_buf),
                        std::mem::take(&mut acc.thought_buf),
                        std::mem::take(&mut acc.trajectory_buf),
                    )
                };
                if !thought_delta.is_empty() {
                    listener.on_thought_delta(&turn_id, thought_delta).await;
                }
                if !text_delta.is_empty() {
                    listener.on_text_delta(&turn_id, text_delta, false).await;
                }
                for (kind, payload) in trajectory {
                    listener.on_trajectory_event(&turn_id, kind, payload).await;
                }
            }
        });

        let client = Arc::new_cyclic(|weak: &std::sync::Weak<Self>| {
            let weak_for_reader = weak.clone();
            let pending_for_reader = Arc::clone(&pending);
            let reader_task = tokio::spawn(async move {
                let mut lines = BufReader::new(stdout).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    let trimmed = line.trim();
                    if trimmed.is_empty() {
                        continue;
                    }
                    let Ok(val) = serde_json::from_str::<Value>(trimmed) else {
                        debug!("ignoring non-JSON line from ACP stdout: {trimmed}");
                        continue;
                    };
                    if val.get("method").is_some() && val.get("id").is_some() {
                        // Agent -> Client Request (`fs/*`, `terminal/*`, `session/request_permission`)
                        if let Some(this) = weak_for_reader.upgrade() {
                            tokio::spawn(async move {
                                this.handle_inbound_request(val).await;
                            });
                        }
                    } else if val.get("method").is_some() {
                        // Agent -> Client Notification (`session/update`)
                        if let Some(this) = weak_for_reader.upgrade() {
                            this.handle_inbound_notification(val).await;
                        }
                    } else if let Some(id_val) = val.get("id") {
                        // Response to a Client -> Agent request
                        if let Some(id_num) = id_val.as_i64()
                            && let Ok(resp) = serde_json::from_value::<JsonRpcResponse>(val)
                            && let Some(tx) = pending_for_reader.lock().await.calls.remove(&id_num)
                        {
                            let _ = tx.send(Ok(resp));
                        }
                    }
                }
                pending_for_reader.lock().await.close(false);
            });

            Self {
                guard: std::sync::Mutex::new(guard),
                child: Mutex::new(child),
                stdin: Mutex::new(stdin),
                next_id: AtomicI64::new(1),
                pending,
                broker,
                stderr_tail,
                active_stream,
                stream_flush,
                agent_capabilities: RwLock::new(InitializeResult::default()),
                acp_session_id: RwLock::new(None),
                config_options: RwLock::new(Vec::new()),
                session_defaults: RwLock::new(Vec::new()),
                latest_usage: RwLock::new(None),
                reader_task,
                coalescer_task,
            }
        });

        Ok(client)
    }

    pub async fn initialize(&self) -> Result<InitializeResult, AcpError> {
        let params = InitializeParams {
            protocol_version: 1,
            client_info: ClientInfo {
                name: "meta-harness".into(),
                version: env!("CARGO_PKG_VERSION").into(),
            },
            client_capabilities: ClientCapabilities {
                fs: FsCapabilities {
                    read_text_file: true,
                    write_text_file: true,
                },
                terminal: true,
            },
        };
        let raw = self
            .request("initialize", serde_json::to_value(params)?)
            .await?;
        let init: InitializeResult = serde_json::from_value(raw).unwrap_or_default();
        *self.agent_capabilities.write().await = init.clone();
        Ok(init)
    }

    pub async fn new_session(
        &self,
        cwd: &Path,
        mcp_servers: Vec<McpServerSpec>,
    ) -> Result<NewSessionResult, AcpError> {
        let params = NewSessionParams {
            cwd: dunce::simplified(cwd).to_string_lossy().into_owned(),
            mcp_servers,
        };
        let raw = self
            .request("session/new", serde_json::to_value(params)?)
            .await?;
        let res: NewSessionResult = serde_json::from_value(raw)?;
        *self.acp_session_id.write().await = Some(res.session_id.clone());
        let opts = parse_new_session_options(&res);
        *self.session_defaults.write().await = opts.clone();
        *self.config_options.write().await = opts;
        Ok(res)
    }

    pub async fn load_session(
        &self,
        acp_session_id: &str,
        cwd: &Path,
        mcp_servers: Vec<McpServerSpec>,
    ) -> Result<(), AcpError> {
        let params = LoadSessionParams {
            session_id: acp_session_id.to_string(),
            cwd: dunce::simplified(cwd).to_string_lossy().into_owned(),
            mcp_servers,
        };
        let response = self
            .request("session/load", serde_json::to_value(params)?)
            .await?;
        let mut options = response
            .get("configOptions")
            .and_then(Value::as_array)
            .map(|options| parse_config_options(options))
            .unwrap_or_default();
        add_legacy_modes(&mut options, response.get("modes"));
        *self.session_defaults.write().await = options.clone();
        *self.config_options.write().await = options;
        *self.acp_session_id.write().await = Some(acp_session_id.to_string());
        Ok(())
    }

    pub async fn set_config_option(&self, config_id: &str, value: &str) -> Result<(), AcpError> {
        let sid = self
            .acp_session_id
            .read()
            .await
            .clone()
            .ok_or_else(|| AcpError::Rpc {
                code: -32001,
                message: "no active ACP sessionId".into(),
                data: None,
            })?;
        let params = SetConfigOptionParams {
            session_id: sid,
            config_id: config_id.to_string(),
            value: Value::String(value.to_string()),
        };
        let res = self
            .request("session/set_config_option", serde_json::to_value(params)?)
            .await?;
        if let Some(raw_opts) = res.get("configOptions").and_then(|v| v.as_array()) {
            *self.config_options.write().await = parse_config_options(raw_opts);
        }
        Ok(())
    }

    pub(crate) async fn apply_approval_mode(
        &self,
        provider: &str,
        policy: PermissionPolicy,
    ) -> Result<(), AcpError> {
        let candidates = crate::approval::acp_mode_candidates(provider, policy);
        let Some(first) = candidates.first() else {
            return Ok(());
        };
        let options = self.config_options.read().await.clone();
        let mode = find_option_by_category_or_id(&options, &["mode"], &["mode", "permissionMode"]);
        let value = if let Some(option) = mode.filter(|option| !option.choices.is_empty()) {
            candidates
                .iter()
                .copied()
                .find(|candidate| {
                    option
                        .choices
                        .iter()
                        .any(|choice| choice.value == *candidate)
                })
                .ok_or_else(|| {
                    AcpError::PermissionDenied(format!(
                        "This harness does not offer the requested approval mode ({})",
                        policy.as_str()
                    ))
                })?
        } else {
            first
        };
        if mode.and_then(|option| option.current_value.as_deref()) == Some(value) {
            return Ok(());
        }
        let option_id = mode.map_or("mode", |option| option.id.as_str());
        match self.set_config_option(option_id, value).await {
            Err(AcpError::Rpc {
                code: -32601 | -32602,
                ..
            }) => {
                let sid = self.acp_session_id.read().await.clone();
                self.request(
                    "session/set_mode",
                    json!({ "sessionId": sid, "modeId": value }),
                )
                .await?;
                if let Some(updated) = self
                    .config_options
                    .write()
                    .await
                    .iter_mut()
                    .find(|option| option.id == option_id)
                {
                    updated.current_value = Some(value.into());
                }
            }
            result => result?,
        }
        Ok(())
    }

    pub async fn apply_model_and_effort(
        &self,
        target_model: Option<&str>,
        target_effort: Option<&str>,
        model_default_effort: Option<&str>,
    ) -> (Option<String>, Option<String>) {
        let opts = self.config_options.read().await.clone();
        let defaults = self.session_defaults.read().await.clone();
        let mut applied_model = None;
        let mut applied_effort = None;

        if let Some(req_model) = target_model.or_else(|| {
            find_option_by_category_or_id(&defaults, &["model"], &["model"])
                .and_then(|option| option.current_value.as_deref())
        }) && let Some(opt) = find_option_by_category_or_id(&opts, &["model"], &["model"])
        {
            let matched = match_choice(&opt.choices, req_model).unwrap_or(req_model);
            if opt.current_value.as_deref() == Some(matched)
                || self.set_config_option(&opt.id, matched).await.is_ok()
            {
                applied_model = Some(matched.to_string());
            } else {
                warn!("session/set_config_option failed for model={matched}");
            }
        }

        let opts = self.config_options.read().await.clone();
        let effort_categories = ["thought_level", "reasoning_effort"];
        let effort_ids = ["reasoning_effort", "thought_level", "effort"];
        if let Some(req_effort) = target_effort
            .or_else(|| {
                // A different model can have a different native default effort.
                let original = find_option_by_category_or_id(&defaults, &["model"], &["model"])
                    .and_then(|option| option.current_value.as_deref());
                if applied_model.as_deref() != original {
                    return None;
                }
                find_option_by_category_or_id(&defaults, &effort_categories, &effort_ids)
                    .and_then(|option| option.current_value.as_deref())
            })
            .or(model_default_effort.filter(|_| applied_model.is_some()))
            && let Some(opt) = find_option_by_category_or_id(&opts, &effort_categories, &effort_ids)
        {
            let matched = match_choice(&opt.choices, req_effort).unwrap_or(req_effort);
            if opt.current_value.as_deref() == Some(matched)
                || self.set_config_option(&opt.id, matched).await.is_ok()
            {
                applied_effort = Some(matched.to_string());
            } else {
                debug!("skipping effort={matched} on adapter that rejected it");
            }
        }

        (applied_model, applied_effort)
    }

    pub async fn prompt(
        &self,
        turn_id: &str,
        prompt_text: &str,
        listener: Arc<dyn TurnStreamListener>,
    ) -> Result<(PromptResult, PromptTextOutput), AcpError> {
        self.prompt_with_images(turn_id, prompt_text, &[], listener)
            .await
    }

    pub async fn prompt_with_images(
        &self,
        turn_id: &str,
        prompt_text: &str,
        images: &[ChatImage],
        listener: Arc<dyn TurnStreamListener>,
    ) -> Result<(PromptResult, PromptTextOutput), AcpError> {
        if !images.is_empty()
            && self
                .agent_capabilities
                .read()
                .await
                .agent_capabilities
                .prompt_capabilities["image"]
                != true
        {
            return Err(AcpError::ImageInputUnsupported);
        }
        let sid = self
            .acp_session_id
            .read()
            .await
            .clone()
            .ok_or_else(|| AcpError::Rpc {
                code: -32001,
                message: "no active ACP sessionId".into(),
                data: None,
            })?;

        {
            let mut guard = self.active_stream.lock().await;
            *guard = Some(TurnStreamAccumulator {
                turn_id: turn_id.to_string(),
                listener: Arc::clone(&listener),
                text_buf: String::new(),
                after_step_boundary: false,
                thought_buf: String::new(),
                trajectory_buf: Vec::new(),
                current_segment: String::new(),
            });
        }

        let mut prompt = vec![ContentBlock::Text {
            text: prompt_text.to_string(),
        }];
        prompt.extend(images.iter().map(|image| ContentBlock::Image {
            mime_type: image.mime_type.clone(),
            data: image.encode(),
        }));
        let params = PromptParams {
            session_id: sid,
            prompt,
        };

        let rpc_res = self
            .request("session/prompt", serde_json::to_value(params)?)
            .await;

        // Flush any remaining buffered deltas before returning
        let _flush = self.stream_flush.lock().await;
        let (final_text_tail, final_thought_tail, output, trajectory) = {
            let mut guard = self.active_stream.lock().await;
            if let Some(mut acc) = guard.take() {
                (
                    std::mem::take(&mut acc.text_buf),
                    std::mem::take(&mut acc.thought_buf),
                    PromptTextOutput {
                        final_text: acc.current_segment.trim().to_string(),
                    },
                    acc.trajectory_buf,
                )
            } else {
                (
                    String::new(),
                    String::new(),
                    PromptTextOutput::default(),
                    Vec::new(),
                )
            }
        };
        if !final_thought_tail.is_empty() {
            listener.on_thought_delta(turn_id, final_thought_tail).await;
        }
        if !final_text_tail.is_empty() {
            listener
                .on_text_delta(turn_id, final_text_tail, false)
                .await;
        }
        for (kind, payload) in trajectory {
            listener.on_trajectory_event(turn_id, kind, payload).await;
        }

        match rpc_res {
            Ok(val) => {
                let res: PromptResult = serde_json::from_value(val).unwrap_or_default();
                if res.stop_reason == "cancelled" {
                    return Err(AcpError::Cancelled);
                }
                Ok((res, output))
            }
            Err(err) => Err(err),
        }
    }

    pub async fn cancel(&self) -> Result<(), AcpError> {
        if let Some(sid) = self.acp_session_id.read().await.clone() {
            let _ = self
                .notify("session/cancel", json!({ "sessionId": sid }))
                .await;
        }
        Ok(())
    }

    pub async fn is_alive(&self) -> bool {
        if self.pending.lock().await.closed {
            return false;
        }
        let mut child = self.child.lock().await;
        matches!(child.try_wait(), Ok(None))
    }

    pub async fn shutdown(&self) {
        self.reader_task.abort();
        self.pending.lock().await.close(true);
        self.coalescer_task.abort();
        self.broker.cleanup_all_terminals().await;
        if let Ok(mut g) = self.guard.lock() {
            g.kill_tree();
        }
        let mut child = self.child.lock().await;
        let _ = child.kill().await;
        let _ = tokio::time::timeout(Duration::from_millis(300), child.wait()).await;
    }

    pub async fn recent_stderr(&self) -> String {
        let guard = self.stderr_tail.lock().await;
        guard.iter().cloned().collect::<Vec<_>>().join("\n")
    }

    async fn request(&self, method: &str, params: Value) -> Result<Value, AcpError> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let request = serde_json::to_string(&JsonRpcRequest {
            jsonrpc: "2.0".into(),
            id: json!(id),
            method: method.into(),
            params: Some(params),
        })?;
        let (tx, rx) = oneshot::channel();
        {
            let mut pending = self.pending.lock().await;
            if pending.closed {
                return Err(AcpError::ProcessExited("ACP connection is closed".into()));
            }
            pending.calls.insert(id, tx);
        }
        if let Err(e) = self.write_line(&request).await {
            self.pending.lock().await.calls.remove(&id);
            return Err(e);
        }

        let resp = rx
            .await
            .map_err(|_| AcpError::ProcessExited("ACP request channel dropped".into()))??;
        if let Some(err) = resp.error {
            if err.code == -32000 && err.message.eq_ignore_ascii_case("authentication required") {
                return Err(AcpError::AuthenticationRequired);
            }
            let lower = err.message.to_ascii_lowercase();
            if err.code == 429
                || lower.contains("429")
                || lower.contains("rate limit")
                || lower.contains("rate_limit")
                || lower.contains("quota")
                || lower.contains("usage limit")
            {
                return Err(AcpError::RateLimited(err.message));
            }
            return Err(AcpError::Rpc {
                code: err.code,
                message: err.message,
                data: err.data,
            });
        }
        Ok(resp.result.unwrap_or(Value::Null))
    }

    async fn notify(&self, method: &str, params: Value) -> Result<(), AcpError> {
        let notif = JsonRpcNotification {
            jsonrpc: "2.0".into(),
            method: method.into(),
            params: Some(params),
        };
        self.write_line(&serde_json::to_string(&notif)?).await
    }

    async fn send_response(&self, id: Value, result: Result<Value, AcpError>) {
        let resp = match result {
            Ok(val) => JsonRpcResponse {
                jsonrpc: "2.0".into(),
                id,
                result: Some(val),
                error: None,
            },
            Err(err) => JsonRpcResponse {
                jsonrpc: "2.0".into(),
                id,
                result: None,
                error: Some(crate::protocol::JsonRpcError {
                    code: -32000,
                    message: err.to_string(),
                    data: None,
                }),
            },
        };
        if let Ok(line) = serde_json::to_string(&resp) {
            let _ = self.write_line(&line).await;
        }
    }

    async fn write_line(&self, line: &str) -> Result<(), AcpError> {
        let mut stdin = self.stdin.lock().await;
        stdin.write_all(line.as_bytes()).await?;
        stdin.write_all(b"\n").await?;
        stdin.flush().await?;
        Ok(())
    }

    async fn handle_inbound_request(&self, msg: Value) {
        let id = msg.get("id").cloned().unwrap_or(Value::Null);
        let method = msg
            .get("method")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        let params = msg.get("params").cloned().unwrap_or(Value::Null);

        let res = match method {
            "fs/read_text_file" => match serde_json::from_value(params) {
                Ok(p) => self.broker.handle_read_text_file(p).await,
                Err(e) => Err(AcpError::Json(e)),
            },
            "fs/write_text_file" => match serde_json::from_value(params) {
                Ok(p) => self.broker.handle_write_text_file(p).await,
                Err(e) => Err(AcpError::Json(e)),
            },
            "terminal/create" => match serde_json::from_value(params) {
                Ok(p) => self.broker.handle_terminal_create(p).await,
                Err(e) => Err(AcpError::Json(e)),
            },
            "terminal/output" => match serde_json::from_value(params) {
                Ok(p) => self.broker.handle_terminal_output(p).await,
                Err(e) => Err(AcpError::Json(e)),
            },
            "terminal/wait_for_exit" => match serde_json::from_value(params) {
                Ok(p) => self.broker.handle_terminal_wait_for_exit(p).await,
                Err(e) => Err(AcpError::Json(e)),
            },
            "terminal/kill" => match serde_json::from_value(params) {
                Ok(p) => self.broker.handle_terminal_kill(p).await,
                Err(e) => Err(AcpError::Json(e)),
            },
            "terminal/release" => match serde_json::from_value(params) {
                Ok(p) => self.broker.handle_terminal_release(p).await,
                Err(e) => Err(AcpError::Json(e)),
            },
            "session/request_permission" => match serde_json::from_value(params) {
                Ok(p) => self.broker.handle_request_permission(p).await,
                Err(e) => Err(AcpError::Json(e)),
            },
            other => Err(AcpError::Rpc {
                code: -32601,
                message: format!("unsupported ACP client method: {other}"),
                data: None,
            }),
        };

        self.send_response(id, res).await;
    }

    async fn handle_inbound_notification(&self, msg: Value) {
        let method = msg
            .get("method")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        if method != "session/update" {
            return;
        }
        let Some(params) = msg.get("params") else {
            return;
        };
        if let Some(reported_id) = params.get("sessionId").and_then(Value::as_str)
            && let Some(active_id) = self.acp_session_id.read().await.as_deref()
            && reported_id != active_id
        {
            return;
        }
        let Some(update) = params.get("update") else {
            return;
        };
        let kind = update
            .get("sessionUpdate")
            .or_else(|| update.get("kind"))
            .and_then(|v| v.as_str())
            .unwrap_or_default();

        match kind {
            "agent_message_chunk" | "message" => {
                let txt = extract_update_text(update.get("content"));
                if txt.is_empty() {
                    return;
                }
                {
                    let mut guard = self.active_stream.lock().await;
                    let Some(acc) = guard.as_mut() else { return };
                    if !acc.after_step_boundary && !acc.current_segment.is_empty() {
                        acc.text_buf.push_str(&txt);
                        acc.current_segment.push_str(&txt);
                        return;
                    }
                }
                if txt.trim().is_empty() {
                    return;
                }
                // Allocate each segment on its first visible text, before a later tool
                // or peer message can be inserted. Batch subsequent deltas as usual.
                let _flush = self.stream_flush.lock().await;
                let (turn_id, listener, previous, first_text) = {
                    let mut guard = self.active_stream.lock().await;
                    let Some(acc) = guard.as_mut() else {
                        return;
                    };
                    acc.after_step_boundary = false;
                    acc.current_segment.clone_from(&txt);
                    (
                        acc.turn_id.clone(),
                        Arc::clone(&acc.listener),
                        std::mem::take(&mut acc.text_buf),
                        txt,
                    )
                };
                if !previous.is_empty() {
                    listener.on_text_delta(&turn_id, previous, false).await;
                }
                listener.on_text_delta(&turn_id, first_text, true).await;
            }
            "agent_thought_chunk" | "thought" => {
                let txt = extract_update_text(update.get("content"));
                if !txt.is_empty() {
                    let mut guard = self.active_stream.lock().await;
                    if let Some(acc) = guard.as_mut() {
                        if !acc.current_segment.trim().is_empty() {
                            acc.after_step_boundary = true;
                        }
                        acc.thought_buf.push_str(&txt);
                    }
                }
            }
            "plan" => {
                let (turn_id, listener) = {
                    let mut guard = self.active_stream.lock().await;
                    let Some(acc) = guard.as_mut() else {
                        return;
                    };
                    if !acc.current_segment.trim().is_empty() {
                        acc.after_step_boundary = true;
                    }
                    (acc.turn_id.clone(), Arc::clone(&acc.listener))
                };
                listener
                    .on_trajectory_event(&turn_id, TrajectoryEventKind::Plan, update.clone())
                    .await;
            }
            "tool_call" | "tool_call_update" => {
                if let Some(acc) = self.active_stream.lock().await.as_mut() {
                    if !acc.current_segment.trim().is_empty() {
                        acc.after_step_boundary = true;
                    }
                    for event in parse_tool_update(update, kind == "tool_call") {
                        push_update(&mut acc.trajectory_buf, event);
                    }
                }
            }
            "config_option_update" => {
                if let Some(arr) = update.get("configOptions").and_then(|v| v.as_array()) {
                    let parsed = parse_config_options(arr);
                    *self.config_options.write().await = parsed.clone();
                    let listener = {
                        let guard = self.active_stream.lock().await;
                        guard.as_ref().map(|a| Arc::clone(&a.listener))
                    };
                    if let Some(l) = listener {
                        l.on_config_options_updated(parsed).await;
                    }
                }
            }
            "current_mode_update" => {
                if let Some(current) = update.get("currentModeId").and_then(Value::as_str)
                    && let Some(option) =
                        self.config_options.write().await.iter_mut().find(|option| {
                            option.id == "mode" || option.category.as_deref() == Some("mode")
                        })
                {
                    option.current_value = Some(current.into());
                }
            }
            "usage_update" => {
                let used = update
                    .get("used")
                    .or_else(|| update.get("usedTokens"))
                    .and_then(Value::as_u64);
                let size = update
                    .get("size")
                    .or_else(|| update.get("contextWindow"))
                    .and_then(Value::as_u64);
                if let (Some(u), Some(s)) = (used, size)
                    && s > 0
                {
                    let model = find_option_by_category_or_id(
                        &self.config_options.read().await,
                        &["model"],
                        &["model"],
                    )
                    .and_then(|option| option.current_value.clone());
                    *self.latest_usage.write().await = Some((u, s, model));
                }
            }
            _ => {}
        }
    }
}

fn find_option_by_category_or_id<'a>(
    opts: &'a [DiscoveredConfigOption],
    categories: &[&str],
    ids: &[&str],
) -> Option<&'a DiscoveredConfigOption> {
    if let Some(found) = opts.iter().find(|o| {
        o.category
            .as_deref()
            .is_some_and(|c| categories.iter().any(|want| c.eq_ignore_ascii_case(want)))
    }) {
        return Some(found);
    }
    opts.iter()
        .find(|o| ids.iter().any(|want| o.id.eq_ignore_ascii_case(want)))
}

fn match_choice<'a>(
    choices: &'a [mh_core::ConfigOptionChoice],
    wanted: &'a str,
) -> Option<&'a str> {
    if choices.is_empty() {
        return Some(wanted);
    }
    if let Some(exact) = choices
        .iter()
        .find(|c| c.value.eq_ignore_ascii_case(wanted))
    {
        return Some(&exact.value);
    }
    let w_lc = wanted.to_ascii_lowercase();
    choices
        .iter()
        .find(|c| {
            c.value.to_ascii_lowercase().contains(&w_lc)
                || c.label.to_ascii_lowercase().contains(&w_lc)
        })
        .map(|c| c.value.as_str())
}
