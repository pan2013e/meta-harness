use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{Value, json};
use similar::TextDiff;
use tokio::io::AsyncReadExt;
use tokio::process::Command;
use tokio::sync::{Mutex, RwLock, watch};

use mh_core::{
    PendingPermissionView, PermissionPolicy, PermissionPromptOption, PermissionRule,
    TrajectoryEventKind, is_path_within, normalize_for_create,
};

use crate::AcpError;
use crate::platform::{ProcessTreeGuard, prepare_isolated_command, shell_command_invocation};
use crate::protocol::{
    FsReadTextFileParams, FsWriteTextFileParams, RequestPermissionParams, SWITCH_MCP_SERVER_NAME,
    TerminalCreateParams, TerminalIdParams, parse_permission_options,
};

#[derive(Debug, Clone)]
pub enum PermissionResolution {
    Selected {
        option_id: String,
        allow_pattern: Option<String>,
    },
    Cancelled,
}

#[async_trait]
pub trait BrokerCallbacks: Send + Sync {
    fn may_auto_approve_switch_tool(&self, _tool: &str) -> bool {
        false
    }

    async fn request_user_permission(
        &self,
        prompt: PendingPermissionView,
    ) -> Result<PermissionResolution, AcpError>;

    async fn record_trajectory_event(
        &self,
        turn_id: &str,
        kind: TrajectoryEventKind,
        payload: Value,
    );

    async fn record_touched_file(&self, turn_id: &str, rel_path: String);

    async fn record_skill_read(&self, _turn_id: &str, _target: &Path) {}
}

#[derive(Debug, Clone)]
pub struct MemberBrokerContext {
    pub session_id: String,
    pub member_handle: String,
    pub project_root: PathBuf,
    pub member_worktree: PathBuf,
    pub skill_read_roots: Arc<RwLock<Vec<PathBuf>>>,
    pub permission_policy: PermissionPolicy,
    pub active_turn_id: Arc<RwLock<Option<String>>>,
    pub rules: Arc<RwLock<Vec<PermissionRule>>>,
}

struct ManagedTerminal {
    stop: watch::Sender<bool>,
    output: Arc<Mutex<Vec<u8>>>,
    output_limit: usize,
    exit_code: watch::Receiver<Option<i32>>,
}

impl ManagedTerminal {
    async fn wait_for_exit(&self) -> Result<i32, AcpError> {
        let mut exit = self.exit_code.clone();
        let status = exit.wait_for(Option::is_some).await.map_err(|_| {
            AcpError::ProcessExited("Terminal closed without reporting its exit status".into())
        })?;
        Ok(status.unwrap_or(-1))
    }

    fn stop(&self) {
        let _ = self.stop.send(true);
    }
}

/// Brokers `fs/*`, `terminal/*`, and `session/request_permission` callbacks from an ACP harness
/// with worktree-smart sandboxing, unified diff capture, and OS process-tree tracking.
pub struct WorkspaceBroker {
    ctx: MemberBrokerContext,
    callbacks: RwLock<Arc<dyn BrokerCallbacks>>,
    terminals: Mutex<HashMap<String, Arc<ManagedTerminal>>>,
}

impl WorkspaceBroker {
    pub fn new(ctx: MemberBrokerContext, callbacks: Arc<dyn BrokerCallbacks>) -> Self {
        Self {
            ctx,
            callbacks: RwLock::new(callbacks),
            terminals: Mutex::new(HashMap::new()),
        }
    }

    pub(crate) async fn update_turn_context(
        &self,
        callbacks: Arc<dyn BrokerCallbacks>,
        rules: Vec<PermissionRule>,
        skill_read_roots: Vec<PathBuf>,
    ) {
        *self.callbacks.write().await = callbacks;
        *self.ctx.rules.write().await = rules;
        *self.ctx.skill_read_roots.write().await = skill_read_roots;
    }

    async fn callbacks(&self) -> Arc<dyn BrokerCallbacks> {
        Arc::clone(&*self.callbacks.read().await)
    }

    pub async fn handle_read_text_file(
        &self,
        params: FsReadTextFileParams,
    ) -> Result<Value, AcpError> {
        let target = resolve_member_path(&self.ctx.member_worktree, &params.path);
        let inside_project = is_path_within(&target, &self.ctx.member_worktree)
            || is_path_within(&target, &self.ctx.project_root);
        let in_skill_root = self
            .ctx
            .skill_read_roots
            .read()
            .await
            .iter()
            .any(|root| is_path_within(&target, root));

        if self.ctx.permission_policy != PermissionPolicy::FullAccess
            && !inside_project
            && !in_skill_root
        {
            self.prompt_or_allow(
                "fs_external",
                &format!("Read external file: {}", target.display()),
                target.to_string_lossy().as_ref(),
                None,
                vec![
                    PermissionPromptOption {
                        option_id: "allow_once".into(),
                        label: "Allow Read Once".into(),
                        kind: "allow_once".into(),
                    },
                    PermissionPromptOption {
                        option_id: "reject_once".into(),
                        label: "Reject".into(),
                        kind: "reject_once".into(),
                    },
                ],
            )
            .await?;
        }

        let content = tokio::fs::read_to_string(&target).await?;
        if in_skill_root && let Some(turn_id) = self.ctx.active_turn_id.read().await.clone() {
            self.callbacks()
                .await
                .record_skill_read(&turn_id, &target)
                .await;
        }
        let sliced = slice_lines(&content, params.line, params.limit);
        Ok(json!({ "content": sliced }))
    }

    pub async fn handle_write_text_file(
        &self,
        params: FsWriteTextFileParams,
    ) -> Result<Value, AcpError> {
        let target = resolve_member_path(&self.ctx.member_worktree, &params.path);
        let inside_member_wt = is_path_within(&target, &self.ctx.member_worktree);

        if self.ctx.permission_policy != PermissionPolicy::FullAccess && !inside_member_wt {
            self.prompt_or_allow(
                "fs_external",
                &format!("Write file outside workspace: {}", target.display()),
                target.to_string_lossy().as_ref(),
                None,
                vec![
                    PermissionPromptOption {
                        option_id: "allow_once".into(),
                        label: "Allow Write Once".into(),
                        kind: "allow_once".into(),
                    },
                    PermissionPromptOption {
                        option_id: "reject_once".into(),
                        label: "Reject".into(),
                        kind: "reject_once".into(),
                    },
                ],
            )
            .await?;
        }

        let old_content = tokio::fs::read_to_string(&target).await.unwrap_or_default();
        if let Some(parent) = target.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        tokio::fs::write(&target, &params.content).await?;

        let rel_path = make_rel_path(&target, &self.ctx.member_worktree);
        let unified = TextDiff::from_lines(&old_content, &params.content)
            .unified_diff()
            .header(&rel_path, &rel_path)
            .to_string();

        if let Some(turn_id) = self.ctx.active_turn_id.read().await.clone() {
            self.callbacks()
                .await
                .record_touched_file(&turn_id, rel_path.clone())
                .await;
            self.callbacks()
                .await
                .record_trajectory_event(
                    &turn_id,
                    TrajectoryEventKind::Diff,
                    json!({
                        "path": rel_path,
                        "diff": unified,
                    }),
                )
                .await;
        }

        Ok(json!({}))
    }

    pub async fn handle_terminal_create(
        &self,
        params: TerminalCreateParams,
    ) -> Result<Value, AcpError> {
        let full_cmd = if params.args.is_empty() {
            params.command.clone()
        } else {
            format!("{} {}", params.command, params.args.join(" "))
        };
        let cwd = params
            .cwd
            .as_deref()
            .map(|c| resolve_member_path(&self.ctx.member_worktree, c))
            .unwrap_or_else(|| self.ctx.member_worktree.clone());

        let cwd_inside = is_path_within(&cwd, &self.ctx.member_worktree);
        let auto_approved = self.ctx.permission_policy == PermissionPolicy::ApproveForMe
            && params.env.is_empty()
            && matches!(full_cmd.as_str(), "pwd" | "Get-Location" | "git --version");
        let needs_prompt = self.ctx.permission_policy != PermissionPolicy::FullAccess
            && (!cwd_inside
                || !(auto_approved || self.matches_allow_rule("terminal", &full_cmd).await));

        if needs_prompt {
            let suggested = suggest_command_pattern(&full_cmd);
            self.prompt_or_allow(
                "terminal",
                &format!("Run command in {}: `{full_cmd}`", cwd.display()),
                &full_cmd,
                Some(suggested),
                vec![
                    PermissionPromptOption {
                        option_id: "allow_once".into(),
                        label: "Allow Once".into(),
                        kind: "allow_once".into(),
                    },
                    PermissionPromptOption {
                        option_id: "allow_always".into(),
                        label: "Allow Pattern for Session".into(),
                        kind: "allow_always".into(),
                    },
                    PermissionPromptOption {
                        option_id: "reject_once".into(),
                        label: "Reject".into(),
                        kind: "reject_once".into(),
                    },
                ],
            )
            .await?;
        }

        let (prog, prog_args) = if params.args.is_empty() {
            shell_command_invocation(&params.command)
        } else {
            (params.command.clone(), params.args.clone())
        };

        let mut cmd = Command::new(&prog);
        cmd.args(&prog_args)
            .current_dir(dunce::simplified(&cwd))
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        for ev in &params.env {
            cmd.env(&ev.name, &ev.value);
        }
        prepare_isolated_command(&mut cmd);

        let mut child = cmd.spawn()?;
        let mut guard = ProcessTreeGuard::attach(&child);
        let output_buf = Arc::new(Mutex::new(Vec::<u8>::new()));
        let output_limit = params.output_byte_limit.unwrap_or(64 * 1024).max(1024);

        if let Some(mut stdout) = child.stdout.take() {
            let buf_clone = Arc::clone(&output_buf);
            tokio::spawn(async move {
                pump_stream(&mut stdout, buf_clone, output_limit).await;
            });
        }
        if let Some(mut stderr) = child.stderr.take() {
            let buf_clone = Arc::clone(&output_buf);
            tokio::spawn(async move {
                pump_stream(&mut stderr, buf_clone, output_limit).await;
            });
        }

        let terminal_id = format!("term_{}", &uuid::Uuid::new_v4().simple().to_string()[..10]);
        let (stop, mut stop_requested) = watch::channel(false);
        let (exit, exit_code) = watch::channel(None);
        tokio::spawn(async move {
            let status = tokio::select! {
                status = child.wait() => status,
                _ = async { let _ = stop_requested.wait_for(|stop| *stop).await; } => {
                    guard.kill_tree();
                    let _ = child.start_kill();
                    child.wait().await
                }
            };
            let code = status.ok().and_then(|status| status.code()).unwrap_or(-1);
            let _ = exit.send(Some(code));
        });
        let term = Arc::new(ManagedTerminal {
            stop,
            output: output_buf,
            output_limit,
            exit_code,
        });

        self.terminals
            .lock()
            .await
            .insert(terminal_id.clone(), term);

        if let Some(turn_id) = self.ctx.active_turn_id.read().await.clone() {
            self.callbacks()
                .await
                .record_trajectory_event(
                    &turn_id,
                    TrajectoryEventKind::Terminal,
                    json!({
                        "terminal_id": terminal_id,
                        "command": full_cmd,
                        "cwd": cwd.to_string_lossy(),
                        "status": "started",
                    }),
                )
                .await;
        }

        Ok(json!({ "terminalId": terminal_id }))
    }

    pub async fn handle_terminal_output(
        &self,
        params: TerminalIdParams,
    ) -> Result<Value, AcpError> {
        let term = self.get_terminal(&params.terminal_id).await?;
        let text = {
            let buf = term.output.lock().await;
            String::from_utf8_lossy(&buf).into_owned()
        };
        let exit_code = *term.exit_code.borrow();
        let truncated = text.len() >= term.output_limit;
        Ok(json!({
            "output": text,
            "truncated": truncated,
            "exitStatus": exit_code.map(|c| json!({ "exitCode": c })),
        }))
    }

    pub async fn handle_terminal_wait_for_exit(
        &self,
        params: TerminalIdParams,
    ) -> Result<Value, AcpError> {
        let term = self.get_terminal(&params.terminal_id).await?;
        let code = term.wait_for_exit().await?;

        if let Some(turn_id) = self.ctx.active_turn_id.read().await.clone() {
            let out_tail = {
                let buf = term.output.lock().await;
                let s = String::from_utf8_lossy(&buf);
                s.chars()
                    .rev()
                    .take(1500)
                    .collect::<String>()
                    .chars()
                    .rev()
                    .collect::<String>()
            };
            self.callbacks()
                .await
                .record_trajectory_event(
                    &turn_id,
                    TrajectoryEventKind::Terminal,
                    json!({
                        "terminal_id": params.terminal_id,
                        "status": "exited",
                        "exit_code": code,
                        "output_tail": out_tail,
                    }),
                )
                .await;
        }

        Ok(json!({ "exitCode": code }))
    }

    pub async fn handle_terminal_kill(&self, params: TerminalIdParams) -> Result<Value, AcpError> {
        let term = self.get_terminal(&params.terminal_id).await?;
        term.stop();
        term.wait_for_exit().await?;
        Ok(json!({}))
    }

    pub async fn handle_terminal_release(
        &self,
        params: TerminalIdParams,
    ) -> Result<Value, AcpError> {
        let removed = self.terminals.lock().await.remove(&params.terminal_id);
        if let Some(term) = removed {
            term.stop();
            let _ = term.wait_for_exit().await;
        }
        Ok(json!({}))
    }

    pub async fn handle_request_permission(
        &self,
        params: RequestPermissionParams,
    ) -> Result<Value, AcpError> {
        let options = parse_permission_options(&params.options);
        let kind = params
            .tool_call
            .get("kind")
            .and_then(|v| v.as_str())
            .unwrap_or("tool");
        let title = params
            .tool_call
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or("Approve tool execution")
            .to_string();

        if matches!(kind, "other" | "mcp")
            && let Some(tool) = params
                .tool_call
                .get("name")
                .and_then(Value::as_str)
                .and_then(switch_tool_name)
            && let Some(allow_once) = params
                .options
                .iter()
                .filter(|option| option["kind"] == "allow_once")
                .find_map(|option| {
                    option
                        .get("optionId")
                        .or_else(|| option.get("id"))?
                        .as_str()
                        .filter(|id| !id.is_empty())
                })
            && self.ctx.active_turn_id.read().await.is_some()
            && self.callbacks().await.may_auto_approve_switch_tool(tool)
        {
            return Ok(json!({
                "outcome": { "outcome": "selected", "optionId": allow_once }
            }));
        }

        // Check every reported path before approving a brokered read or edit.
        if matches!(kind, "edit" | "read" | "write" | "fs") {
            let paths = tool_call_paths(&params.tool_call);
            let mut skill_reads = Vec::new();
            let allow_paths = {
                let skill_roots = self.ctx.skill_read_roots.read().await;
                !paths.is_empty()
                    && paths.into_iter().all(|path| {
                        let target = resolve_member_path(&self.ctx.member_worktree, path);
                        if kind == "read"
                            && skill_roots.iter().any(|root| is_path_within(&target, root))
                        {
                            skill_reads.push(target);
                            true
                        } else {
                            is_path_within(&target, &self.ctx.member_worktree)
                        }
                    })
            };

            if allow_paths
                && let Some(allow) = options
                    .iter()
                    .find(|o| o.kind == "allow_once")
                    .map(|o| o.option_id.clone())
            {
                if !skill_reads.is_empty()
                    && let Some(turn_id) = self.ctx.active_turn_id.read().await.clone()
                {
                    let callbacks = self.callbacks().await;
                    for path in skill_reads {
                        callbacks.record_skill_read(&turn_id, &path).await;
                    }
                }
                return Ok(json!({
                    "outcome": {
                        "outcome": "selected",
                        "optionId": allow,
                    }
                }));
            }
        }

        if self.ctx.permission_policy == PermissionPolicy::FullAccess
            && let Some(allow) = options.iter().find(|option| option.kind == "allow_once")
        {
            return Ok(json!({
                "outcome": { "outcome": "selected", "optionId": allow.option_id }
            }));
        }

        if (kind == "execute" || kind == "terminal")
            && self.matches_allow_rule("terminal", &title).await
            && let Some(allow) = options
                .iter()
                .find(|o| o.kind.starts_with("allow"))
                .map(|o| o.option_id.clone())
        {
            return Ok(json!({
                "outcome": { "outcome": "selected", "optionId": allow }
            }));
        }

        let prompt = PendingPermissionView {
            request_id: format!(
                "permreq_{}",
                &uuid::Uuid::new_v4().simple().to_string()[..12]
            ),
            session_id: self.ctx.session_id.clone(),
            member_handle: self.ctx.member_handle.clone(),
            turn_id: self.ctx.active_turn_id.read().await.clone(),
            tool_kind: kind.to_string(),
            title: title.clone(),
            detail: serde_json::to_string_pretty(&params.tool_call).unwrap_or(title.clone()),
            suggested_pattern: if matches!(kind, "execute" | "terminal") {
                Some(suggest_command_pattern(&title))
            } else {
                None
            },
            options,
            created_at: chrono::Utc::now().timestamp_millis(),
        };

        match self
            .callbacks()
            .await
            .request_user_permission(prompt)
            .await?
        {
            PermissionResolution::Selected {
                option_id,
                allow_pattern,
            } => {
                if let Some(pat) = allow_pattern {
                    self.ctx.rules.write().await.push(PermissionRule {
                        id: format!("rule_{}", uuid::Uuid::new_v4().simple()),
                        session_id: self.ctx.session_id.clone(),
                        tool_kind: "terminal".into(),
                        pattern: pat,
                        decision: "allow".into(),
                        created_at: chrono::Utc::now().timestamp_millis(),
                    });
                }
                Ok(json!({
                    "outcome": {
                        "outcome": "selected",
                        "optionId": option_id,
                    }
                }))
            }
            PermissionResolution::Cancelled => Ok(json!({
                "outcome": { "outcome": "cancelled" }
            })),
        }
    }

    async fn prompt_or_allow(
        &self,
        tool_kind: &str,
        title: &str,
        detail: &str,
        suggested_pattern: Option<String>,
        options: Vec<PermissionPromptOption>,
    ) -> Result<(), AcpError> {
        let prompt = PendingPermissionView {
            request_id: format!(
                "permreq_{}",
                &uuid::Uuid::new_v4().simple().to_string()[..12]
            ),
            session_id: self.ctx.session_id.clone(),
            member_handle: self.ctx.member_handle.clone(),
            turn_id: self.ctx.active_turn_id.read().await.clone(),
            tool_kind: tool_kind.to_string(),
            title: title.to_string(),
            detail: detail.to_string(),
            suggested_pattern,
            options,
            created_at: chrono::Utc::now().timestamp_millis(),
        };

        match self
            .callbacks()
            .await
            .request_user_permission(prompt)
            .await?
        {
            PermissionResolution::Selected {
                option_id,
                allow_pattern,
            } => {
                if option_id.starts_with("reject") {
                    return Err(AcpError::PermissionDenied(title.to_string()));
                }
                if let Some(pat) = allow_pattern {
                    self.ctx.rules.write().await.push(PermissionRule {
                        id: format!("rule_{}", uuid::Uuid::new_v4().simple()),
                        session_id: self.ctx.session_id.clone(),
                        tool_kind: tool_kind.to_string(),
                        pattern: pat,
                        decision: "allow".into(),
                        created_at: chrono::Utc::now().timestamp_millis(),
                    });
                }
                Ok(())
            }
            PermissionResolution::Cancelled => {
                Err(AcpError::PermissionDenied("cancelled by user".into()))
            }
        }
    }

    async fn matches_allow_rule(&self, tool_kind: &str, candidate: &str) -> bool {
        let rules = self.ctx.rules.read().await;
        rules.iter().any(|r| {
            r.tool_kind == tool_kind
                && r.decision == "allow"
                && wildcard_match(&r.pattern, candidate)
        })
    }

    async fn get_terminal(&self, id: &str) -> Result<Arc<ManagedTerminal>, AcpError> {
        self.terminals
            .lock()
            .await
            .get(id)
            .cloned()
            .ok_or_else(|| AcpError::Rpc {
                code: -32602,
                message: format!("unknown terminalId: {id}"),
                data: None,
            })
    }

    pub async fn cleanup_all_terminals(&self) {
        let map = std::mem::take(&mut *self.terminals.lock().await);
        for term in map.values() {
            term.stop();
        }
        for (_, term) in map {
            let _ = term.wait_for_exit().await;
        }
    }
}

pub fn resolve_member_path(member_worktree: &Path, raw: &str) -> PathBuf {
    let p = Path::new(raw);
    let joined = if p.is_absolute() {
        p.to_path_buf()
    } else {
        member_worktree.join(p)
    };
    normalize_for_create(&joined).unwrap_or(joined)
}

fn make_rel_path(target: &Path, root: &Path) -> String {
    let norm_root = dunce::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    target
        .strip_prefix(&norm_root)
        .unwrap_or(target)
        .to_string_lossy()
        .replace('\\', "/")
}

fn slice_lines(content: &str, line: Option<u32>, limit: Option<u32>) -> String {
    if line.is_none() && limit.is_none() {
        return content.to_string();
    }
    let start_idx = line.unwrap_or(1).saturating_sub(1) as usize;
    let max_lines = limit.unwrap_or(u32::MAX) as usize;
    content
        .lines()
        .skip(start_idx)
        .take(max_lines)
        .collect::<Vec<_>>()
        .join("\n")
}

async fn pump_stream<R: tokio::io::AsyncRead + Unpin>(
    reader: &mut R,
    buf: Arc<Mutex<Vec<u8>>>,
    limit: usize,
) {
    let mut chunk = [0u8; 4096];
    loop {
        match reader.read(&mut chunk).await {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                let mut guard = buf.lock().await;
                if guard.len() < limit {
                    let take = (limit - guard.len()).min(n);
                    guard.extend_from_slice(&chunk[..take]);
                }
            }
        }
    }
}

fn switch_tool_name(name: &str) -> Option<&str> {
    name.strip_prefix("mcp__")
        .and_then(|name| name.strip_prefix(SWITCH_MCP_SERVER_NAME))
        .and_then(|name| name.strip_prefix("__"))
        .or_else(|| {
            name.strip_prefix("mcp_")
                .and_then(|name| name.strip_prefix(SWITCH_MCP_SERVER_NAME))
                .and_then(|name| name.strip_prefix('_'))
        })
}

fn tool_call_paths(tool_call: &Value) -> Vec<&str> {
    let mut paths = Vec::new();
    if let Some(locations) = tool_call.get("locations").and_then(Value::as_array) {
        for item in locations {
            let Some(path) = item
                .get("path")
                .and_then(Value::as_str)
                .filter(|path| !path.is_empty())
            else {
                return Vec::new();
            };
            paths.push(path);
        }
    }
    if let Some(input) = tool_call.get("rawInput") {
        for key in ["path", "file_path"] {
            if let Some(path) = input
                .get(key)
                .and_then(Value::as_str)
                .filter(|path| !path.is_empty())
                && !paths.contains(&path)
            {
                paths.push(path);
            }
        }
    }
    paths
}

pub fn suggest_command_pattern(cmd: &str) -> String {
    let parts: Vec<&str> = cmd.split_whitespace().collect();
    match parts.as_slice() {
        [first, second, ..] => format!("{first} {second}*"),
        [first] => format!("{first}*"),
        [] => "*".to_string(),
    }
}

pub fn wildcard_match(pattern: &str, candidate: &str) -> bool {
    if let Some(prefix) = pattern.strip_suffix('*') {
        candidate.starts_with(prefix)
    } else {
        pattern == candidate
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestCallbacks {
        decision: PermissionResolution,
        events: Mutex<Vec<(String, TrajectoryEventKind, Value)>>,
        skill_reads: Mutex<Vec<PathBuf>>,
        auto_approve_switch_tools: bool,
    }

    #[async_trait]
    impl BrokerCallbacks for TestCallbacks {
        fn may_auto_approve_switch_tool(&self, tool: &str) -> bool {
            self.auto_approve_switch_tools && tool == "chat_list_peers"
        }

        async fn request_user_permission(
            &self,
            _prompt: PendingPermissionView,
        ) -> Result<PermissionResolution, AcpError> {
            Ok(self.decision.clone())
        }

        async fn record_trajectory_event(
            &self,
            turn_id: &str,
            kind: TrajectoryEventKind,
            payload: Value,
        ) {
            self.events
                .lock()
                .await
                .push((turn_id.to_string(), kind, payload));
        }

        async fn record_touched_file(&self, _turn_id: &str, _rel_path: String) {}

        async fn record_skill_read(&self, _turn_id: &str, target: &Path) {
            self.skill_reads.lock().await.push(target.to_path_buf());
        }
    }

    #[tokio::test]
    async fn worktree_smart_allows_inside_writes_and_gates_outside() {
        let tmp = tempfile::tempdir().unwrap();
        let proj = tmp.path().join("proj");
        let wt = proj.join(".meta-harness/worktrees/s1/_exec_codex");
        let skills_root = tmp.path().join("skills");
        let outside = tmp.path().join("outside.txt");
        std::fs::create_dir_all(&wt).unwrap();
        std::fs::create_dir_all(skills_root.join("demo-skill")).unwrap();
        std::fs::write(
            skills_root.join("demo-skill/SKILL.md"),
            "# Demo Skill\nRead me.\n",
        )
        .unwrap();

        let cb = Arc::new(TestCallbacks {
            decision: PermissionResolution::Selected {
                option_id: "reject_once".into(),
                allow_pattern: None,
            },
            events: Mutex::new(Vec::new()),
            skill_reads: Mutex::new(Vec::new()),
            auto_approve_switch_tools: false,
        });

        let broker = WorkspaceBroker::new(
            MemberBrokerContext {
                session_id: "s1".into(),
                member_handle: "codex".into(),
                project_root: proj,
                member_worktree: wt.clone(),
                skill_read_roots: Arc::new(RwLock::new(vec![skills_root.clone()])),
                permission_policy: PermissionPolicy::RequireApproval,
                active_turn_id: Arc::new(RwLock::new(Some("t1".into()))),
                rules: Arc::new(RwLock::new(Vec::new())),
            },
            cb.clone(),
        );

        // Inside write succeeds automatically and records unified diff
        broker
            .handle_write_text_file(FsWriteTextFileParams {
                session_id: "s1".into(),
                path: "src/lib.rs".into(),
                content: "pub fn hello() {}\n".into(),
            })
            .await
            .unwrap();
        assert!(wt.join("src/lib.rs").exists());
        assert_eq!(cb.events.lock().await.len(), 1);

        // Reading inside skill_read_roots succeeds without prompt and fires callback
        let read_res = broker
            .handle_read_text_file(FsReadTextFileParams {
                session_id: "s1".into(),
                path: skills_root
                    .join("demo-skill/SKILL.md")
                    .to_string_lossy()
                    .into_owned(),
                line: None,
                limit: None,
            })
            .await
            .unwrap();
        assert!(read_res["content"].as_str().unwrap().contains("Demo Skill"));
        assert_eq!(cb.skill_reads.lock().await.len(), 1);

        let read_request = |locations: Value| RequestPermissionParams {
            session_id: "s1".into(),
            tool_call: json!({"kind":"read", "locations":locations}),
            options: vec![
                json!({"optionId":"always", "kind":"allow_always"}),
                json!({"optionId":"once", "kind":"allow_once"}),
                json!({"optionId":"reject_once", "kind":"reject_once"}),
            ],
        };
        let skill_file = skills_root.join("demo-skill/SKILL.md");
        let read_allowed = broker
            .handle_request_permission(read_request(json!([{"path":skill_file}])))
            .await
            .unwrap();
        assert_eq!(read_allowed["outcome"]["optionId"], "once");
        let read_prompted = broker
            .handle_request_permission(read_request(json!([{"path":skill_file},{"path":outside}])))
            .await
            .unwrap();
        assert_eq!(read_prompted["outcome"]["optionId"], "reject_once");

        // Writing inside skill_read_roots stays gated!
        let write_skill_err = broker
            .handle_write_text_file(FsWriteTextFileParams {
                session_id: "s1".into(),
                path: skills_root
                    .join("demo-skill/SKILL.md")
                    .to_string_lossy()
                    .into_owned(),
                content: "overwrite".into(),
            })
            .await
            .unwrap_err();
        assert!(matches!(write_skill_err, AcpError::PermissionDenied(_)));

        // Outside write triggers prompt -> rejected
        let err = broker
            .handle_write_text_file(FsWriteTextFileParams {
                session_id: "s1".into(),
                path: outside.to_string_lossy().into_owned(),
                content: "evil".into(),
            })
            .await
            .unwrap_err();
        assert!(matches!(err, AcpError::PermissionDenied(_)));

        let (command, args) = if cfg!(windows) {
            (
                "powershell.exe",
                vec!["-NoProfile", "-Command", "Start-Sleep -Seconds 10"],
            )
        } else {
            ("sleep", vec!["10"])
        };
        broker.ctx.rules.write().await.push(PermissionRule {
            id: "test_sleep".into(),
            session_id: "s1".into(),
            tool_kind: "terminal".into(),
            pattern: format!("{command}*"),
            decision: "allow".into(),
            created_at: 0,
        });
        let terminal = broker
            .handle_terminal_create(TerminalCreateParams {
                session_id: "s1".into(),
                command: command.into(),
                args: args.into_iter().map(str::to_string).collect(),
                cwd: None,
                env: Vec::new(),
                output_byte_limit: None,
            })
            .await
            .unwrap();
        let request = TerminalIdParams {
            session_id: "s1".into(),
            terminal_id: terminal["terminalId"].as_str().unwrap().into(),
        };
        let wait = broker.handle_terminal_wait_for_exit(request.clone());
        let stop = async {
            tokio::time::sleep(std::time::Duration::from_millis(80)).await;
            broker.handle_terminal_kill(request).await
        };
        let (waited, stopped) = tokio::time::timeout(std::time::Duration::from_secs(2), async {
            tokio::join!(wait, stop)
        })
        .await
        .unwrap();
        assert!(waited.is_ok() && stopped.is_ok());
    }

    #[tokio::test]
    async fn switch_tool_auto_approval_uses_the_registered_name_and_one_time_option() {
        let tmp = tempfile::tempdir().unwrap();
        let active_turn = Arc::new(RwLock::new(Some("t1".into())));
        let broker = WorkspaceBroker::new(
            MemberBrokerContext {
                session_id: "s1".into(),
                member_handle: "antigravity".into(),
                project_root: tmp.path().into(),
                member_worktree: tmp.path().into(),
                skill_read_roots: Arc::new(RwLock::new(Vec::new())),
                permission_policy: PermissionPolicy::RequireApproval,
                active_turn_id: Arc::clone(&active_turn),
                rules: Arc::new(RwLock::new(Vec::new())),
            },
            Arc::new(TestCallbacks {
                decision: PermissionResolution::Selected {
                    option_id: "reject".into(),
                    allow_pattern: None,
                },
                events: Mutex::new(Vec::new()),
                skill_reads: Mutex::new(Vec::new()),
                auto_approve_switch_tools: true,
            }),
        );

        let request = |kind: &str, name: Option<&str>, include_once: bool| {
            let mut options = vec![json!({ "optionId": "always", "kind": "allow_always" })];
            if include_once {
                options.push(json!({ "optionId": "once", "kind": "allow_once" }));
            }
            options.push(json!({ "optionId": "reject", "kind": "reject_once" }));
            RequestPermissionParams {
                session_id: "native_session".into(),
                tool_call: json!({
                    "kind": kind,
                    "name": name,
                    "title": "Run mcp_meta-harness-switch_chat_list_peers",
                }),
                options,
            }
        };
        let recognized = Some("mcp_meta-harness-switch_chat_list_peers");
        let allowed = broker
            .handle_request_permission(request("other", recognized, true))
            .await
            .unwrap();
        assert_eq!(allowed["outcome"]["optionId"], "once");

        for (kind, name, include_once) in [
            ("other", None, true),
            ("other", Some("mcp_meta-harness-switch_unknown_tool"), true),
            ("other", Some("mcp_other-server_chat_list_peers"), true),
            ("execute", recognized, true),
            ("other", recognized, false),
        ] {
            let denied = broker
                .handle_request_permission(request(kind, name, include_once))
                .await
                .unwrap();
            assert_eq!(denied["outcome"]["optionId"], "reject");
        }

        *active_turn.write().await = None;
        let idle = broker
            .handle_request_permission(request("other", recognized, true))
            .await
            .unwrap();
        assert_eq!(idle["outcome"]["optionId"], "reject");
    }
}
