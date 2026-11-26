use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{Value, json};
use similar::TextDiff;
use tokio::io::AsyncReadExt;
use tokio::process::{Child, Command};
use tokio::sync::{Mutex, RwLock};

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
}

#[derive(Debug, Clone)]
pub struct MemberBrokerContext {
    pub session_id: String,
    pub member_handle: String,
    pub project_root: PathBuf,
    pub member_worktree: PathBuf,
    pub permission_policy: PermissionPolicy,
    pub active_turn_id: Arc<RwLock<Option<String>>>,
    pub rules: Arc<RwLock<Vec<PermissionRule>>>,
}

struct ManagedTerminal {
    _guard: ProcessTreeGuard,
    child: Mutex<Child>,
    output: Arc<Mutex<Vec<u8>>>,
    output_limit: usize,
    exit_code: Arc<Mutex<Option<i32>>>,
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
    ) {
        *self.callbacks.write().await = callbacks;
        *self.ctx.rules.write().await = rules;
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

        if !inside_project {
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
        let sliced = slice_lines(&content, params.line, params.limit);
        Ok(json!({ "content": sliced }))
    }

    pub async fn handle_write_text_file(
        &self,
        params: FsWriteTextFileParams,
    ) -> Result<Value, AcpError> {
        let target = resolve_member_path(&self.ctx.member_worktree, &params.path);
        let inside_member_wt = is_path_within(&target, &self.ctx.member_worktree);

        let needs_prompt = match self.ctx.permission_policy {
            PermissionPolicy::Strict => true,
            PermissionPolicy::WorktreeSmart | PermissionPolicy::AutonomousSandbox => {
                !inside_member_wt
            }
        };

        if needs_prompt {
            let reason = if inside_member_wt {
                format!("Write file inside worktree: {}", target.display())
            } else {
                format!("Write file OUTSIDE worktree: {}", target.display())
            };
            self.prompt_or_allow(
                "fs_external",
                &reason,
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
        let needs_prompt = match self.ctx.permission_policy {
            PermissionPolicy::AutonomousSandbox => !cwd_inside || is_risky_command(&full_cmd),
            PermissionPolicy::WorktreeSmart | PermissionPolicy::Strict => {
                !cwd_inside || !self.matches_allow_rule("terminal", &full_cmd).await
            }
        };

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
        let guard = ProcessTreeGuard::attach(&child);
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
        let term = Arc::new(ManagedTerminal {
            _guard: guard,
            child: Mutex::new(child),
            output: output_buf,
            output_limit,
            exit_code: Arc::new(Mutex::new(None)),
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
        let exit_code = *term.exit_code.lock().await;
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
        let code = {
            let mut child = term.child.lock().await;
            let status = child.wait().await?;
            status.code().unwrap_or(-1)
        };
        *term.exit_code.lock().await = Some(code);

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
        let mut child = term.child.lock().await;
        let _ = child.kill().await;
        Ok(json!({}))
    }

    pub async fn handle_terminal_release(
        &self,
        params: TerminalIdParams,
    ) -> Result<Value, AcpError> {
        if let Some(term) = self.terminals.lock().await.remove(&params.terminal_id) {
            let mut child = term.child.lock().await;
            let _ = child.kill().await;
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

        if kind == "other"
            && let Some(tool) = params
                .tool_call
                .get("name")
                .and_then(Value::as_str)
                .and_then(|name| name.strip_prefix("mcp_"))
                .and_then(|name| name.strip_prefix(SWITCH_MCP_SERVER_NAME))
                .and_then(|name| name.strip_prefix('_'))
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

        // Worktree-smart fast-path for edit/read tool calls strictly inside member worktree
        if matches!(
            self.ctx.permission_policy,
            PermissionPolicy::WorktreeSmart | PermissionPolicy::AutonomousSandbox
        ) && matches!(kind, "edit" | "read" | "write" | "fs")
            && let Some(loc_path) = extract_tool_call_path(&params.tool_call)
        {
            let resolved = resolve_member_path(&self.ctx.member_worktree, &loc_path);
            if is_path_within(&resolved, &self.ctx.member_worktree)
                && let Some(allow) = options
                    .iter()
                    .find(|o| o.kind.starts_with("allow"))
                    .map(|o| o.option_id.clone())
            {
                return Ok(json!({
                    "outcome": {
                        "outcome": "selected",
                        "optionId": allow,
                    }
                }));
            }
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
        for (_, term) in map {
            let mut child = term.child.lock().await;
            let _ = child.kill().await;
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

fn extract_tool_call_path(tool_call: &Value) -> Option<String> {
    if let Some(p) = tool_call
        .get("locations")
        .and_then(|l| l.as_array())
        .and_then(|a| a.first())
        .and_then(|item| item.get("path"))
        .and_then(|v| v.as_str())
    {
        return Some(p.to_string());
    }
    tool_call
        .get("rawInput")
        .and_then(|r| r.get("path").or_else(|| r.get("file_path")))
        .and_then(|v| v.as_str())
        .map(str::to_string)
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

fn is_risky_command(cmd: &str) -> bool {
    let lower = cmd.to_ascii_lowercase();
    lower.contains("git push")
        || lower.contains("npm publish")
        || lower.contains("rm -rf")
        || lower.contains("del /s")
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestCallbacks {
        decision: PermissionResolution,
        events: Mutex<Vec<(String, TrajectoryEventKind, Value)>>,
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
    }

    #[tokio::test]
    async fn worktree_smart_allows_inside_writes_and_gates_outside() {
        let tmp = tempfile::tempdir().unwrap();
        let proj = tmp.path().join("proj");
        let wt = proj.join(".meta-harness/worktrees/s1/_exec_codex");
        let outside = tmp.path().join("outside.txt");
        std::fs::create_dir_all(&wt).unwrap();

        let cb = Arc::new(TestCallbacks {
            decision: PermissionResolution::Selected {
                option_id: "reject_once".into(),
                allow_pattern: None,
            },
            events: Mutex::new(Vec::new()),
            auto_approve_switch_tools: false,
        });

        let broker = WorkspaceBroker::new(
            MemberBrokerContext {
                session_id: "s1".into(),
                member_handle: "codex".into(),
                project_root: proj,
                member_worktree: wt.clone(),
                permission_policy: PermissionPolicy::WorktreeSmart,
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
                permission_policy: PermissionPolicy::Strict,
                active_turn_id: Arc::clone(&active_turn),
                rules: Arc::new(RwLock::new(Vec::new())),
            },
            Arc::new(TestCallbacks {
                decision: PermissionResolution::Selected {
                    option_id: "reject".into(),
                    allow_pattern: None,
                },
                events: Mutex::new(Vec::new()),
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
