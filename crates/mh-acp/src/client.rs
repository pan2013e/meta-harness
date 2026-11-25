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

use mh_core::{DiscoveredConfigOption, TrajectoryEventKind};

use crate::AcpError;
use crate::broker::{BrokerCallbacks, MemberBrokerContext, WorkspaceBroker};
use crate::platform::{ProcessTreeGuard, prepare_isolated_command, resolve_command};
use crate::protocol::{
    ClientCapabilities, ClientInfo, ContentBlock, FsCapabilities, InitializeParams,
    InitializeResult, JsonRpcNotification, JsonRpcRequest, JsonRpcResponse, LoadSessionParams,
    McpServerSpec, NewSessionParams, NewSessionResult, PromptParams, PromptResult,
    SetConfigOptionParams, extract_update_text, parse_config_options,
};

#[async_trait]
pub trait TurnStreamListener: Send + Sync {
    async fn on_text_delta(&self, turn_id: &str, delta: String);
    async fn on_thought_delta(&self, turn_id: &str, delta: String);
    async fn on_trajectory_event(&self, turn_id: &str, kind: TrajectoryEventKind, payload: Value);
    async fn on_config_options_updated(&self, options: Vec<DiscoveredConfigOption>);
}

struct TurnStreamAccumulator {
    turn_id: String,
    listener: Arc<dyn TurnStreamListener>,
    text_buf: String,
    thought_buf: String,
    full_reply: String,
}

pub struct AcpClient {
    guard: std::sync::Mutex<ProcessTreeGuard>,
    child: Mutex<Child>,
    stdin: Mutex<ChildStdin>,
    next_id: AtomicI64,
    pending: Arc<Mutex<HashMap<i64, oneshot::Sender<JsonRpcResponse>>>>,
    broker: Arc<WorkspaceBroker>,
    stderr_tail: Arc<Mutex<VecDeque<String>>>,
    active_stream: Arc<Mutex<Option<TurnStreamAccumulator>>>,
    pub agent_capabilities: RwLock<InitializeResult>,
    pub acp_session_id: RwLock<Option<String>>,
    pub config_options: RwLock<Vec<DiscoveredConfigOption>>,
    reader_task: tokio::task::JoinHandle<()>,
    coalescer_task: tokio::task::JoinHandle<()>,
}

impl AcpClient {
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
        for (k, v) in extra_env {
            cmd.env(k, v);
        }
        prepare_isolated_command(&mut cmd);

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

        let pending = Arc::new(Mutex::new(
            HashMap::<i64, oneshot::Sender<JsonRpcResponse>>::new(),
        ));
        let stderr_tail = Arc::new(Mutex::new(VecDeque::<String>::with_capacity(100)));
        let active_stream = Arc::new(Mutex::new(None::<TurnStreamAccumulator>));
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
        let coalescer_task = tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_millis(25));
            loop {
                interval.tick().await;
                let (turn_id, listener, text_delta, thought_delta) = {
                    let mut guard = stream_for_flush.lock().await;
                    let Some(acc) = guard.as_mut() else {
                        continue;
                    };
                    if acc.text_buf.is_empty() && acc.thought_buf.is_empty() {
                        continue;
                    }
                    (
                        acc.turn_id.clone(),
                        Arc::clone(&acc.listener),
                        std::mem::take(&mut acc.text_buf),
                        std::mem::take(&mut acc.thought_buf),
                    )
                };
                if !thought_delta.is_empty() {
                    listener.on_thought_delta(&turn_id, thought_delta).await;
                }
                if !text_delta.is_empty() {
                    listener.on_text_delta(&turn_id, text_delta).await;
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
                            && let Some(tx) = pending_for_reader.lock().await.remove(&id_num)
                        {
                            let _ = tx.send(resp);
                        }
                    }
                }
                // Wake any remaining waiters if stdout closed
                let mut guard = pending_for_reader.lock().await;
                for (_, tx) in guard.drain() {
                    let _ = tx.send(JsonRpcResponse {
                        jsonrpc: "2.0".into(),
                        id: Value::Null,
                        result: None,
                        error: Some(crate::protocol::JsonRpcError {
                            code: -32000,
                            message: "ACP agent stdout closed".into(),
                            data: None,
                        }),
                    });
                }
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
                agent_capabilities: RwLock::new(InitializeResult::default()),
                acp_session_id: RwLock::new(None),
                config_options: RwLock::new(Vec::new()),
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
        let opts = parse_config_options(&res.config_options);
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
        let _ = self
            .request("session/load", serde_json::to_value(params)?)
            .await?;
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

    pub async fn apply_model_and_effort(
        &self,
        target_model: Option<&str>,
        target_effort: Option<&str>,
    ) -> (Option<String>, Option<String>) {
        let opts = self.config_options.read().await.clone();
        let mut applied_model = None;
        let mut applied_effort = None;

        if let Some(req_model) = target_model
            && let Some(opt) = find_option_by_category_or_id(&opts, &["model"], &["model"])
        {
            let matched = match_choice(&opt.choices, req_model).unwrap_or(req_model);
            if self.set_config_option(&opt.id, matched).await.is_ok() {
                applied_model = Some(matched.to_string());
            } else {
                warn!("session/set_config_option failed for model={matched}");
            }
        }

        if let Some(req_effort) = target_effort
            && let Some(opt) = find_option_by_category_or_id(
                &opts,
                &["thought_level", "reasoning_effort"],
                &["reasoning_effort", "thought_level", "effort"],
            )
        {
            let matched = match_choice(&opt.choices, req_effort).unwrap_or(req_effort);
            if self.set_config_option(&opt.id, matched).await.is_ok() {
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
    ) -> Result<(PromptResult, String), AcpError> {
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
                thought_buf: String::new(),
                full_reply: String::new(),
            });
        }

        let params = PromptParams {
            session_id: sid,
            prompt: vec![ContentBlock::Text {
                text: prompt_text.to_string(),
            }],
        };

        let rpc_res = self
            .request("session/prompt", serde_json::to_value(params)?)
            .await;

        // Flush any remaining buffered deltas before returning
        let (final_text_tail, final_thought_tail, full_reply) = {
            let mut guard = self.active_stream.lock().await;
            if let Some(mut acc) = guard.take() {
                (
                    std::mem::take(&mut acc.text_buf),
                    std::mem::take(&mut acc.thought_buf),
                    acc.full_reply,
                )
            } else {
                (String::new(), String::new(), String::new())
            }
        };
        if !final_thought_tail.is_empty() {
            listener.on_thought_delta(turn_id, final_thought_tail).await;
        }
        if !final_text_tail.is_empty() {
            listener.on_text_delta(turn_id, final_text_tail).await;
        }

        match rpc_res {
            Ok(val) => {
                let res: PromptResult = serde_json::from_value(val).unwrap_or_default();
                if res.stop_reason == "cancelled" {
                    return Err(AcpError::Cancelled);
                }
                Ok((res, full_reply))
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
        let mut child = self.child.lock().await;
        matches!(child.try_wait(), Ok(None))
    }

    pub async fn shutdown(&self) {
        self.coalescer_task.abort();
        self.reader_task.abort();
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
        let (tx, rx) = oneshot::channel();
        self.pending.lock().await.insert(id, tx);

        let req = JsonRpcRequest {
            jsonrpc: "2.0".into(),
            id: json!(id),
            method: method.into(),
            params: Some(params),
        };
        if let Err(e) = self.write_line(&serde_json::to_string(&req)?).await {
            self.pending.lock().await.remove(&id);
            return Err(e);
        }

        let resp = rx
            .await
            .map_err(|_| AcpError::ProcessExited("ACP request channel dropped".into()))?;
        if let Some(err) = resp.error {
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
        let Some(update) = msg.get("params").and_then(|p| p.get("update")) else {
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
                if !txt.is_empty() {
                    let mut guard = self.active_stream.lock().await;
                    if let Some(acc) = guard.as_mut() {
                        acc.text_buf.push_str(&txt);
                        acc.full_reply.push_str(&txt);
                    }
                }
            }
            "agent_thought_chunk" | "thought" => {
                let txt = extract_update_text(update.get("content"));
                if !txt.is_empty() {
                    let mut guard = self.active_stream.lock().await;
                    if let Some(acc) = guard.as_mut() {
                        acc.thought_buf.push_str(&txt);
                    }
                }
            }
            "plan" => {
                let (turn_id, listener) = {
                    let guard = self.active_stream.lock().await;
                    let Some(acc) = guard.as_ref() else {
                        return;
                    };
                    (acc.turn_id.clone(), Arc::clone(&acc.listener))
                };
                listener
                    .on_trajectory_event(&turn_id, TrajectoryEventKind::Plan, update.clone())
                    .await;
            }
            "tool_call" | "tool_call_update" => {
                let (turn_id, listener) = {
                    let guard = self.active_stream.lock().await;
                    let Some(acc) = guard.as_ref() else {
                        return;
                    };
                    (acc.turn_id.clone(), Arc::clone(&acc.listener))
                };
                listener
                    .on_trajectory_event(&turn_id, TrajectoryEventKind::ToolCall, update.clone())
                    .await;
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
