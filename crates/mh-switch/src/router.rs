use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{Value, json};
use tokio::sync::{Mutex, RwLock, broadcast, oneshot};
use tracing::warn;

use mh_acp::{
    AcpError, AcpSupervisor, BrokerCallbacks, MemberSpawnSpec, MergeOutcome, PermissionResolution,
    QuotaTracker, TurnStreamListener, build_mcp_bridge_spec, builtin_adapter_catalog,
    commit_worktree_if_dirty, ensure_member_worktree, ensure_session_integration_worktree,
    get_head_sha, get_session_diff, init_git_repo, integrate_member_turn, probe_installed_adapters,
    sync_member_worktree,
};
use mh_core::{
    AppConfig, AppPaths, ChatMessage, DiffStat, DiscoveredConfigOption, DispatchTaskSpec,
    IsolationMode, MemberContextBreakdown, MemberRole, MemberStatus, MessageKind,
    PendingPermissionView, SenderKind, Session, SessionContextReport, SessionMember,
    TrajectoryEventKind, TurnStatus, WaitForGraph, WsEvent, parse_mentions,
};
use mh_store::{
    NewMessageParams, PatchSessionParams, RecordEpisodicJobParams, StartTrajectoryParams, Store,
};

use crate::SwitchError;
use crate::orchestrator::{
    MemoryCurationPromptInput, build_conflict_escalation_prompt, build_memory_curation_prompt,
    build_orchestrator_telemetry_block, pick_fallback_executor, truncate_str,
};

pub type BoxTurnFuture<'a> = std::pin::Pin<
    Box<dyn std::future::Future<Output = Result<(String, String), SwitchError>> + Send + 'a>,
>;

#[derive(Debug, Clone)]
pub enum TurnPurpose {
    Normal,
    OrchestratorTriage,
    ConflictResolution {
        member_handle: String,
        member_branch: String,
        conflicting_files: Vec<String>,
        diff_preview: String,
        source_turn_id: String,
    },
    MemoryCuration,
    PeerConsult {
        caller: String,
    },
}

#[derive(Debug, Clone)]
pub struct ChainState {
    pub chain_id: String,
    pub max_hops: u32,
    pub hops_used: u32,
    pub cancelled: bool,
    pub paused_targets: Vec<DispatchTaskSpec>,
}

pub struct PendingPermissionEntry {
    pub view: PendingPermissionView,
    pub tx: oneshot::Sender<PermissionResolution>,
}

#[derive(Debug, Clone)]
pub struct PendingDispatch {
    pub dispatch_id: String,
    pub message_id: String,
    pub chain_id: String,
    pub caller: String,
    pub rationale: String,
    pub dispatches: Vec<DispatchTaskSpec>,
}

pub struct SessionRuntime {
    pub git_merge_lock: Mutex<()>,
    shared_cwd_lock: Mutex<()>,
    pub member_locks: Mutex<HashMap<String, Arc<Mutex<()>>>>,
    pub wait_for_graph: Mutex<WaitForGraph>,
    pub chains: Mutex<HashMap<String, ChainState>>,
    pub active_member_chain: RwLock<HashMap<String, String>>,
    pub member_statuses: RwLock<HashMap<String, (MemberStatus, Option<String>)>>,
    pub pending_permissions: Mutex<HashMap<String, PendingPermissionEntry>>,
    pub pending_dispatches: Mutex<HashMap<String, PendingDispatch>>,
    pub orchestrator_dispatched_turn: Mutex<HashSet<String>>,
    pub curation_recorded_turns: Mutex<HashSet<String>>,
    pub cancel_epoch: RwLock<u64>,
}

impl Default for SessionRuntime {
    fn default() -> Self {
        Self {
            git_merge_lock: Mutex::new(()),
            shared_cwd_lock: Mutex::new(()),
            member_locks: Mutex::new(HashMap::new()),
            wait_for_graph: Mutex::new(WaitForGraph::new()),
            chains: Mutex::new(HashMap::new()),
            active_member_chain: RwLock::new(HashMap::new()),
            member_statuses: RwLock::new(HashMap::new()),
            pending_permissions: Mutex::new(HashMap::new()),
            pending_dispatches: Mutex::new(HashMap::new()),
            orchestrator_dispatched_turn: Mutex::new(HashSet::new()),
            curation_recorded_turns: Mutex::new(HashSet::new()),
            cancel_epoch: RwLock::new(0),
        }
    }
}

pub struct SwitchEngine {
    pub config: RwLock<AppConfig>,
    pub store: Store,
    pub supervisor: Arc<AcpSupervisor>,
    pub quota: Arc<QuotaTracker>,
    pub events_tx: broadcast::Sender<WsEvent>,
    pub self_exe: PathBuf,
    pub internal_mcp_url: RwLock<String>,
    pub bridge_token: String,
    pub app_paths: AppPaths,
    runtimes: Mutex<HashMap<String, Arc<SessionRuntime>>>,
}

impl SwitchEngine {
    pub fn new(
        config: AppConfig,
        store: Store,
        supervisor: Arc<AcpSupervisor>,
        quota: Arc<QuotaTracker>,
        self_exe: PathBuf,
        internal_mcp_url: String,
        bridge_token: String,
    ) -> Arc<Self> {
        Self::new_with_paths(
            config,
            store,
            supervisor,
            quota,
            self_exe,
            internal_mcp_url,
            bridge_token,
            AppPaths::resolve(),
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn new_with_paths(
        config: AppConfig,
        store: Store,
        supervisor: Arc<AcpSupervisor>,
        quota: Arc<QuotaTracker>,
        self_exe: PathBuf,
        internal_mcp_url: String,
        bridge_token: String,
        app_paths: AppPaths,
    ) -> Arc<Self> {
        let (events_tx, _) = broadcast::channel(1024);
        Arc::new(Self {
            config: RwLock::new(config),
            store,
            supervisor,
            quota,
            events_tx,
            self_exe,
            internal_mcp_url: RwLock::new(internal_mcp_url),
            bridge_token,
            app_paths,
            runtimes: Mutex::new(HashMap::new()),
        })
    }

    pub fn resolve_session_repo_root(&self, session: &Session) -> Result<PathBuf, SwitchError> {
        if let Some(ref pid) = session.project_id {
            Ok(PathBuf::from(&self.store.get_project(pid)?.root_path))
        } else {
            Ok(self.app_paths.standalone_session_root(&session.id))
        }
    }

    pub async fn set_internal_mcp_url(&self, url: String) {
        *self.internal_mcp_url.write().await = url;
    }

    pub async fn runtime(&self, session_id: &str) -> Arc<SessionRuntime> {
        let mut guard = self.runtimes.lock().await;
        Arc::clone(
            guard
                .entry(session_id.to_string())
                .or_insert_with(|| Arc::new(SessionRuntime::default())),
        )
    }

    pub async fn get_member_statuses(
        &self,
        session_id: &str,
    ) -> HashMap<String, (MemberStatus, Option<String>)> {
        let rt = self.runtime(session_id).await;
        let guard = rt.member_statuses.read().await;
        guard.clone()
    }

    pub async fn list_pending_permissions(&self, session_id: &str) -> Vec<PendingPermissionView> {
        let rt = self.runtime(session_id).await;
        let guard = rt.pending_permissions.lock().await;
        guard.values().map(|e| e.view.clone()).collect()
    }

    pub async fn active_chain_for_member(&self, session_id: &str, handle: &str) -> String {
        let rt = self.runtime(session_id).await;
        rt.active_member_chain
            .read()
            .await
            .get(handle)
            .cloned()
            .unwrap_or_else(|| "chain_mcp".into())
    }

    pub(crate) async fn available_members(&self, members: &[SessionMember]) -> HashSet<String> {
        let config = self.config.read().await;
        let probes = probe_installed_adapters(&config);
        members
            .iter()
            .filter(|member| {
                probes
                    .iter()
                    .any(|probe| probe.installed && probe.adapter_id == member.adapter_id)
            })
            .map(|member| member.handle.clone())
            .collect()
    }

    async fn validate_dispatch_targets(
        &self,
        session_id: &str,
        tasks: &[DispatchTaskSpec],
    ) -> Result<(), SwitchError> {
        let members = self.store.list_members(session_id)?;
        let available = self.available_members(&members).await;
        validate_available_targets(tasks.iter().map(|task| task.target.as_str()), &available)
    }

    /// Posts a user message and initiates routing (either explicit `@mentions` in parallel
    /// or the configured Orchestrator).
    pub async fn handle_user_message(
        self: &Arc<Self>,
        session_id: &str,
        content: String,
    ) -> Result<ChatMessage, SwitchError> {
        self.handle_user_message_with_overrides(session_id, content, None, None)
            .await
    }

    pub async fn handle_user_message_with_overrides(
        self: &Arc<Self>,
        session_id: &str,
        content: String,
        model_override: Option<String>,
        effort_override: Option<String>,
    ) -> Result<ChatMessage, SwitchError> {
        let session = self.store.get_session(session_id)?;
        let members = self.store.list_members(session_id)?;
        if members.is_empty() {
            return Err(SwitchError::Validation(
                "session has no active members".into(),
            ));
        }

        let clean_model = model_override
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        let clean_effort = effort_override
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);

        let valid_handles: HashSet<String> = members.iter().map(|m| m.handle.clone()).collect();
        let parsed = parse_mentions(&content, &valid_handles);
        let available = self.available_members(&members).await;
        let orchestrator = session
            .orchestrator_member
            .as_ref()
            .filter(|handle| available.contains(*handle));
        if parsed.has_orchestrator && orchestrator.is_none() {
            return Err(SwitchError::Validation("The session orchestrator is unavailable. Choose an available member from the roster.".into()));
        }
        let chain_id = format!("ch_{}", &uuid::Uuid::new_v4().simple().to_string()[..10]);

        let mut recipients: Vec<String> = Vec::new();
        if parsed.has_orchestrator || parsed.is_unmentioned() {
            if let Some(orch) = orchestrator {
                recipients.push(orch.clone());
            } else if let Some(first) = members.iter().find(|m| available.contains(&m.handle)) {
                recipients.push(first.handle.clone());
            }
        } else if parsed.has_all {
            for m in &members {
                if available.contains(&m.handle)
                    && (session.orchestrator_member.as_deref() != Some(&m.handle)
                        || m.role != MemberRole::Orchestrator)
                {
                    recipients.push(m.handle.clone());
                }
            }
        } else {
            for t in &parsed.targets {
                recipients.push(t.handle.clone());
            }
        }
        if recipients.is_empty() {
            return Err(SwitchError::Validation(
                "No harness is available in this session. Check Settings → Harnesses.".into(),
            ));
        }
        validate_available_targets(recipients.iter().map(String::as_str), &available)?;

        let (slash_cmds, cleaned_body) = extract_workflow_slash_commands(&content);
        if slash_cmds.iter().any(|c| c == "compact") {
            let notice = self.compact_session(session_id).await?;
            if cleaned_body.trim().is_empty() {
                return Ok(notice);
            }
        }
        let directive_prefix = build_slash_directive_prefix(&slash_cmds);
        let dispatch_content = if directive_prefix.is_empty() {
            content.clone()
        } else {
            format!("{directive_prefix}\n{content}")
        };

        let is_first_user_msg = self
            .store
            .list_messages(session_id, None, 20)
            .map(|msgs| !msgs.iter().any(|m| m.sender_kind == SenderKind::User))
            .unwrap_or(false);
        let should_auto_title = is_first_user_msg && session.title == "New session";

        let user_msg = self.store.append_message(NewMessageParams {
            id: None,
            session_id: session_id.to_string(),
            chain_id: chain_id.clone(),
            parent_msg_id: None,
            sender_kind: SenderKind::User,
            sender_id: "user".into(),
            recipients: recipients.clone(),
            kind: MessageKind::Text,
            content: content.clone(),
            turn_id: None,
            metadata: json!({}),
        })?;
        let _ = self.events_tx.send(WsEvent::ChatMessageCreated {
            message: user_msg.clone(),
        });

        let rt = self.runtime(session_id).await;
        rt.chains.lock().await.insert(
            chain_id.clone(),
            ChainState {
                chain_id: chain_id.clone(),
                max_hops: session.max_hops.max(1),
                hops_used: 0,
                cancelled: false,
                paused_targets: Vec::new(),
            },
        );

        let tasks: Vec<(DispatchTaskSpec, TurnPurpose)> =
            if parsed.has_orchestrator || parsed.is_unmentioned() {
                if let Some(orch) = orchestrator {
                    let brief = if clean_model.is_some() || clean_effort.is_some() {
                        format!(
                            "<user_model_preference model=\"{}\" effort=\"{}\"/>\n{}",
                            clean_model.as_deref().unwrap_or("default"),
                            clean_effort.as_deref().unwrap_or("default"),
                            dispatch_content
                        )
                    } else {
                        dispatch_content.clone()
                    };
                    vec![(
                        DispatchTaskSpec {
                            target: orch.clone(),
                            model: clean_model.clone(),
                            effort: clean_effort.clone(),
                            task_brief: brief,
                            mode: "parallel".into(),
                        },
                        TurnPurpose::OrchestratorTriage,
                    )]
                } else {
                    vec![(
                        DispatchTaskSpec {
                            target: recipients[0].clone(),
                            model: clean_model.clone(),
                            effort: clean_effort.clone(),
                            task_brief: dispatch_content.clone(),
                            mode: "parallel".into(),
                        },
                        TurnPurpose::Normal,
                    )]
                }
            } else if parsed.has_all {
                recipients
                    .into_iter()
                    .map(|r| {
                        (
                            DispatchTaskSpec {
                                target: r,
                                model: clean_model.clone(),
                                effort: clean_effort.clone(),
                                task_brief: dispatch_content.clone(),
                                mode: "parallel".into(),
                            },
                            TurnPurpose::Normal,
                        )
                    })
                    .collect()
            } else {
                parsed
                    .targets
                    .into_iter()
                    .map(|t| {
                        (
                            DispatchTaskSpec {
                                target: t.handle,
                                model: t.model_override.or_else(|| clean_model.clone()),
                                effort: t.effort_override.or_else(|| clean_effort.clone()),
                                task_brief: dispatch_content.clone(),
                                mode: "parallel".into(),
                            },
                            TurnPurpose::Normal,
                        )
                    })
                    .collect()
            };

        if should_auto_title {
            let engine = Arc::clone(self);
            let sid = session_id.to_string();
            let raw_prompt = content;
            tokio::spawn(async move {
                engine.trigger_auto_session_title(&sid, &raw_prompt).await;
            });
        }

        for (task, purpose) in tasks {
            let engine = Arc::clone(self);
            let sid = session_id.to_string();
            let cid = chain_id.clone();
            tokio::spawn(async move {
                let _ = engine.execute_member_turn(&sid, &cid, task, purpose).await;
            });
        }

        Ok(user_msg)
    }

    pub async fn send_peer_message(
        self: &Arc<Self>,
        session_id: &str,
        caller: &str,
        to: Vec<String>,
        content: String,
    ) -> Result<ChatMessage, SwitchError> {
        let session = self.store.get_session(session_id)?;
        let members = self.store.list_members(session_id)?;
        let valid_handles: HashSet<String> = members.iter().map(|m| m.handle.clone()).collect();
        let chain_id = self.active_chain_for_member(session_id, caller).await;

        let mut target_handles = Vec::new();
        for t in &to {
            if valid_handles.contains(t) && t != caller {
                target_handles.push(t.clone());
            }
        }
        let parsed = parse_mentions(&content, &valid_handles);
        for t in parsed.targets {
            if t.handle != caller && !target_handles.contains(&t.handle) {
                target_handles.push(t.handle);
            }
        }
        if !target_handles.is_empty() {
            let available = self.available_members(&members).await;
            validate_available_targets(target_handles.iter().map(String::as_str), &available)?;
        }

        let sender_kind = if session.orchestrator_member.as_deref() == Some(caller) {
            SenderKind::Orchestrator
        } else {
            SenderKind::Member
        };

        let msg = self.store.append_message(NewMessageParams {
            id: None,
            session_id: session_id.to_string(),
            chain_id: chain_id.clone(),
            parent_msg_id: None,
            sender_kind,
            sender_id: caller.to_string(),
            recipients: target_handles.clone(),
            kind: MessageKind::Text,
            content: content.clone(),
            turn_id: None,
            metadata: json!({ "via": "mcp_chat_send_message" }),
        })?;
        let _ = self.events_tx.send(WsEvent::ChatMessageCreated {
            message: msg.clone(),
        });

        let specs: Vec<DispatchTaskSpec> = target_handles
            .into_iter()
            .map(|h| DispatchTaskSpec {
                target: h,
                model: None,
                effort: None,
                task_brief: content.clone(),
                mode: "parallel".into(),
            })
            .collect();

        if !specs.is_empty() {
            self.enqueue_hop_dispatches(session_id, &chain_id, specs)
                .await?;
        }

        Ok(msg)
    }

    pub async fn run_peer_consult(
        self: &Arc<Self>,
        session_id: &str,
        caller: &str,
        target: &str,
        question: &str,
        context_turn_id: Option<String>,
    ) -> Result<Value, SwitchError> {
        if self.store.get_session(session_id)?.isolation_mode == IsolationMode::SharedCwd {
            return Err(SwitchError::Validation(
                "Synchronous consultations require isolated worktrees. Use chat_send_message in shared folder sessions.".into(),
            ));
        }
        let member = self.store.get_member(session_id, target)?;
        let available = self.available_members(&[member]).await;
        validate_available_targets([target], &available)?;
        let chain_id = self.active_chain_for_member(session_id, caller).await;
        self.try_consume_hop(session_id, &chain_id, vec![]).await?;

        let rt = self.runtime(session_id).await;
        {
            let mut dag = rt.wait_for_graph.lock().await;
            if let Err(cycle) = dag.try_acquire_edge(caller, target) {
                return Err(SwitchError::CycleDetected(cycle));
            }
        }

        // Guard ensuring DAG edge always releases even if task cancels/errors
        struct DagReleaseGuard {
            rt: Arc<SessionRuntime>,
            caller: String,
            target: String,
        }
        impl Drop for DagReleaseGuard {
            fn drop(&mut self) {
                let rt = Arc::clone(&self.rt);
                let c = self.caller.clone();
                let t = self.target.clone();
                tokio::spawn(async move {
                    rt.wait_for_graph.lock().await.release_edge(&c, &t);
                });
            }
        }
        let _dag_guard = DagReleaseGuard {
            rt: Arc::clone(&rt),
            caller: caller.to_string(),
            target: target.to_string(),
        };

        let q_msg = self.store.append_message(NewMessageParams {
            id: None,
            session_id: session_id.to_string(),
            chain_id: chain_id.clone(),
            parent_msg_id: None,
            sender_kind: SenderKind::Member,
            sender_id: caller.to_string(),
            recipients: vec![target.to_string()],
            kind: MessageKind::ConsultThread,
            content: question.to_string(),
            turn_id: context_turn_id.clone(),
            metadata: json!({
                "consult_role": "question",
                "caller": caller,
                "target": target,
            }),
        })?;
        let _ = self.events_tx.send(WsEvent::SwitchConsultUpdated {
            session_id: session_id.to_string(),
            message: q_msg.clone(),
        });

        self.set_member_status(session_id, caller, MemberStatus::Consulting, None)
            .await;

        let timeout_secs = self.config.read().await.switch.peer_consult_timeout_secs;
        let brief = if let Some(ctid) = &context_turn_id {
            format!(
                "[Consult from @{caller}] {question}\n(Context turn_id: {ctid}; you may call `trajectory_read` if needed. Answer concisely.)"
            )
        } else {
            format!("[Consult from @{caller}] {question}\n(Answer concisely.)")
        };

        let fut = self.execute_member_turn(
            session_id,
            &chain_id,
            DispatchTaskSpec {
                target: target.to_string(),
                model: None,
                effort: None,
                task_brief: brief,
                mode: "parallel".into(),
            },
            TurnPurpose::PeerConsult {
                caller: caller.to_string(),
            },
        );

        let result = tokio::time::timeout(Duration::from_secs(timeout_secs.max(10)), fut).await;
        self.set_member_status(session_id, caller, MemberStatus::Thinking, None)
            .await;

        match result {
            Ok(Ok((turn_id, answer))) => {
                let a_msg = self.store.append_message(NewMessageParams {
                    id: None,
                    session_id: session_id.to_string(),
                    chain_id,
                    parent_msg_id: Some(q_msg.id.clone()),
                    sender_kind: SenderKind::Member,
                    sender_id: target.to_string(),
                    recipients: vec![caller.to_string()],
                    kind: MessageKind::ConsultThread,
                    content: answer.clone(),
                    turn_id: Some(turn_id.clone()),
                    metadata: json!({
                        "consult_role": "answer",
                        "caller": caller,
                        "target": target,
                    }),
                })?;
                let _ = self.events_tx.send(WsEvent::SwitchConsultUpdated {
                    session_id: session_id.to_string(),
                    message: a_msg,
                });
                Ok(json!({
                    "target": target,
                    "turn_id": turn_id,
                    "answer": answer,
                }))
            }
            Ok(Err(e)) => Err(e),
            Err(_) => Err(SwitchError::Timeout(format!(
                "peer_consult to @{target} timed out after {timeout_secs}s"
            ))),
        }
    }

    pub async fn submit_orchestrator_dispatch(
        self: &Arc<Self>,
        session_id: &str,
        caller: &str,
        rationale: String,
        mut dispatches: Vec<DispatchTaskSpec>,
    ) -> Result<String, SwitchError> {
        let session = self.store.get_session(session_id)?;
        for dispatch in &mut dispatches {
            dispatch.target = dispatch.target.trim().trim_start_matches('@').to_string();
        }
        self.validate_dispatch_targets(session_id, &dispatches)
            .await?;
        let chain_id = self.active_chain_for_member(session_id, caller).await;
        let rt = self.runtime(session_id).await;
        rt.orchestrator_dispatched_turn
            .lock()
            .await
            .insert(chain_id.clone());

        let dispatch_id = format!("dsp_{}", &uuid::Uuid::new_v4().simple().to_string()[..10]);
        let status = if session.confirm_before_dispatch {
            "awaiting_confirmation"
        } else {
            "dispatched"
        };

        let recipients: Vec<String> = dispatches.iter().map(|d| d.target.clone()).collect();
        let card = self.store.append_message(NewMessageParams {
            id: None,
            session_id: session_id.to_string(),
            chain_id: chain_id.clone(),
            parent_msg_id: None,
            sender_kind: SenderKind::Orchestrator,
            sender_id: caller.to_string(),
            recipients,
            kind: MessageKind::DispatchCard,
            content: rationale.clone(),
            turn_id: None,
            metadata: json!({
                "dispatch_id": dispatch_id,
                "status": status,
                "dispatches": dispatches,
            }),
        })?;
        let _ = self.events_tx.send(WsEvent::ChatMessageCreated {
            message: card.clone(),
        });
        let _ = self.events_tx.send(WsEvent::SwitchDispatchDecided {
            session_id: session_id.to_string(),
            dispatch_id: dispatch_id.clone(),
            message_id: card.id.clone(),
            status: status.into(),
            rationale: rationale.clone(),
            dispatches: dispatches.clone(),
        });

        let pending = PendingDispatch {
            dispatch_id: dispatch_id.clone(),
            message_id: card.id.clone(),
            chain_id: chain_id.clone(),
            caller: caller.to_string(),
            rationale,
            dispatches: dispatches.clone(),
        };

        if session.confirm_before_dispatch {
            rt.pending_dispatches
                .lock()
                .await
                .insert(dispatch_id.clone(), pending);
        } else {
            self.launch_dispatch_tasks(session_id, pending).await?;
        }

        Ok(dispatch_id)
    }

    pub async fn confirm_dispatch(
        self: &Arc<Self>,
        session_id: &str,
        dispatch_id: &str,
    ) -> Result<(), SwitchError> {
        let rt = self.runtime(session_id).await;
        let mut dispatches = rt.pending_dispatches.lock().await;
        let pending = dispatches
            .get(dispatch_id)
            .ok_or_else(|| SwitchError::NotFound(format!("dispatch {dispatch_id}")))?;
        self.validate_dispatch_targets(session_id, &pending.dispatches)
            .await?;
        let pending = dispatches
            .remove(dispatch_id)
            .ok_or_else(|| SwitchError::NotFound(format!("dispatch {dispatch_id}")))?;
        drop(dispatches);

        let updated = self.store.update_message(
            &pending.message_id,
            &pending.rationale,
            Some(json!({
                "dispatch_id": pending.dispatch_id,
                "status": "dispatched",
                "dispatches": pending.dispatches,
            })),
        )?;
        let _ = self
            .events_tx
            .send(WsEvent::ChatMessageCompleted { message: updated });
        let _ = self.events_tx.send(WsEvent::SwitchDispatchDecided {
            session_id: session_id.to_string(),
            dispatch_id: pending.dispatch_id.clone(),
            message_id: pending.message_id.clone(),
            status: "dispatched".into(),
            rationale: pending.rationale.clone(),
            dispatches: pending.dispatches.clone(),
        });

        self.launch_dispatch_tasks(session_id, pending).await
    }

    pub async fn cancel_dispatch(
        &self,
        session_id: &str,
        dispatch_id: &str,
    ) -> Result<(), SwitchError> {
        let rt = self.runtime(session_id).await;
        let pending = rt
            .pending_dispatches
            .lock()
            .await
            .remove(dispatch_id)
            .ok_or_else(|| SwitchError::NotFound(format!("dispatch {dispatch_id}")))?;
        let updated = self.store.update_message(
            &pending.message_id,
            &pending.rationale,
            Some(json!({
                "dispatch_id": pending.dispatch_id,
                "status": "cancelled",
                "dispatches": pending.dispatches,
            })),
        )?;
        let _ = self
            .events_tx
            .send(WsEvent::ChatMessageCompleted { message: updated });
        Ok(())
    }

    async fn launch_dispatch_tasks(
        self: &Arc<Self>,
        session_id: &str,
        pending: PendingDispatch,
    ) -> Result<(), SwitchError> {
        self.enqueue_hop_dispatches(session_id, &pending.chain_id, pending.dispatches)
            .await
    }

    async fn enqueue_hop_dispatches(
        self: &Arc<Self>,
        session_id: &str,
        chain_id: &str,
        specs: Vec<DispatchTaskSpec>,
    ) -> Result<(), SwitchError> {
        self.validate_dispatch_targets(session_id, &specs).await?;
        self.try_consume_hop(session_id, chain_id, specs.clone())
            .await?;

        let mut sequential = Vec::new();
        for spec in specs {
            if spec.mode == "sequential" {
                sequential.push(spec);
            } else {
                let engine = Arc::clone(self);
                let sid = session_id.to_string();
                let cid = chain_id.to_string();
                tokio::spawn(async move {
                    let _ = engine
                        .execute_member_turn(&sid, &cid, spec, TurnPurpose::Normal)
                        .await;
                });
            }
        }

        if !sequential.is_empty() {
            let engine = Arc::clone(self);
            let sid = session_id.to_string();
            let cid = chain_id.to_string();
            tokio::spawn(async move {
                for spec in sequential {
                    if engine
                        .execute_member_turn(&sid, &cid, spec, TurnPurpose::Normal)
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
            });
        }

        Ok(())
    }

    async fn try_consume_hop(
        &self,
        session_id: &str,
        chain_id: &str,
        pending_specs: Vec<DispatchTaskSpec>,
    ) -> Result<(), SwitchError> {
        let rt = self.runtime(session_id).await;
        let mut chains = rt.chains.lock().await;
        let chain = chains.entry(chain_id.to_string()).or_insert(ChainState {
            chain_id: chain_id.to_string(),
            max_hops: self.config.read().await.switch.max_agent_hops_per_user_turn,
            hops_used: 0,
            cancelled: false,
            paused_targets: Vec::new(),
        });

        if chain.cancelled {
            return Err(SwitchError::Cancelled);
        }
        if chain.hops_used >= chain.max_hops {
            let target_names: Vec<String> =
                pending_specs.iter().map(|s| s.target.clone()).collect();
            chain.paused_targets.extend(pending_specs);
            let _ = self.events_tx.send(WsEvent::SwitchHopLimitPaused {
                session_id: session_id.to_string(),
                chain_id: chain_id.to_string(),
                hops_used: chain.hops_used,
                max_hops: chain.max_hops,
                pending_targets: target_names,
            });
            return Err(SwitchError::HopLimitReached {
                chain_id: chain_id.to_string(),
                hops_used: chain.hops_used,
            });
        }

        chain.hops_used += 1;
        Ok(())
    }

    pub async fn continue_chain(
        self: &Arc<Self>,
        session_id: &str,
        chain_id: &str,
        extra_hops: u32,
    ) -> Result<(), SwitchError> {
        let rt = self.runtime(session_id).await;
        let pending_specs = {
            let mut chains = rt.chains.lock().await;
            let chain = chains
                .get_mut(chain_id)
                .ok_or_else(|| SwitchError::NotFound(format!("chain {chain_id}")))?;
            chain.max_hops += extra_hops.max(1);
            std::mem::take(&mut chain.paused_targets)
        };
        if !pending_specs.is_empty() {
            self.enqueue_hop_dispatches(session_id, chain_id, pending_specs)
                .await?;
        }
        Ok(())
    }

    pub async fn resolve_permission(
        &self,
        session_id: &str,
        request_id: &str,
        option_id: String,
        allow_pattern: Option<String>,
    ) -> Result<(), SwitchError> {
        let rt = self.runtime(session_id).await;
        let mut pending = rt.pending_permissions.lock().await;
        let view = &pending
            .get(request_id)
            .ok_or_else(|| SwitchError::NotFound(format!("permission {request_id}")))?
            .view;
        let option = view
            .options
            .iter()
            .find(|option| option.option_id == option_id)
            .ok_or_else(|| SwitchError::Validation("unknown permission option".into()))?;
        let allowed = option.kind.starts_with("allow");
        if let Some(ref pat) = allow_pattern {
            if !allowed || view.suggested_pattern.is_none() || pat.trim().is_empty() {
                return Err(SwitchError::Validation("invalid permission pattern".into()));
            }
            self.store
                .add_permission_rule(session_id, "terminal", pat, "allow")?;
        }
        let entry = pending
            .remove(request_id)
            .ok_or_else(|| SwitchError::NotFound(format!("permission {request_id}")))?;
        drop(pending);
        let decision = if allowed { "allowed" } else { "rejected" };

        let _ = entry.tx.send(PermissionResolution::Selected {
            option_id,
            allow_pattern,
        });

        let _ = self.events_tx.send(WsEvent::PermissionResolved {
            session_id: session_id.to_string(),
            request_id: request_id.to_string(),
            decision: decision.into(),
        });
        Ok(())
    }

    pub async fn cancel_all_in_session(&self, session_id: &str) {
        let rt = self.runtime(session_id).await;
        {
            let mut epoch = rt.cancel_epoch.write().await;
            *epoch += 1;
        }
        {
            let mut chains = rt.chains.lock().await;
            for c in chains.values_mut() {
                c.cancelled = true;
            }
        }
        {
            let mut perms = rt.pending_permissions.lock().await;
            for (req_id, entry) in perms.drain() {
                let _ = entry.tx.send(PermissionResolution::Cancelled);
                let _ = self.events_tx.send(WsEvent::PermissionResolved {
                    session_id: session_id.to_string(),
                    request_id: req_id,
                    decision: "cancelled".into(),
                });
            }
        }
        rt.wait_for_graph.lock().await.clear();
        self.supervisor.cancel_session(session_id).await;
    }

    /// Executes a single ACP turn for `task.target`, handling worktree isolation, delta sync,
    /// streaming updates, post-turn Git integration, conflict escalation, and memory curation.
    pub fn execute_member_turn<'a>(
        self: &'a Arc<Self>,
        session_id: &'a str,
        chain_id: &'a str,
        task: DispatchTaskSpec,
        purpose: TurnPurpose,
    ) -> BoxTurnFuture<'a> {
        Box::pin(async move {
            let rt = self.runtime(session_id).await;
            let epoch_at_start = *rt.cancel_epoch.read().await;
            let member_lock = {
                let mut map = rt.member_locks.lock().await;
                Arc::clone(
                    map.entry(task.target.clone())
                        .or_insert_with(|| Arc::new(Mutex::new(()))),
                )
            };
            let _member_guard = member_lock.lock().await;
            let session = self.store.get_session(session_id)?;
            let _shared_cwd_guard = if session.isolation_mode == IsolationMode::SharedCwd {
                Some(rt.shared_cwd_lock.lock().await)
            } else {
                None
            };
            if *rt.cancel_epoch.read().await != epoch_at_start
                || rt
                    .chains
                    .lock()
                    .await
                    .get(chain_id)
                    .is_some_and(|chain| chain.cancelled)
            {
                return Err(SwitchError::Cancelled);
            }

            let project_root = self.resolve_session_repo_root(&session)?;
            if session.project_id.is_none() && !project_root.join(".git").exists() {
                tokio::fs::create_dir_all(&project_root).await?;
                let _ = init_git_repo(&project_root).await;
            }
            let mut member = self.store.get_member(session_id, &task.target)?;
            let previous_cwd = member.worktree_path.clone();
            let cfg = self.config.read().await.clone();
            let adapter_cfg = cfg
                .adapters
                .get(&member.adapter_id)
                .cloned()
                .ok_or_else(|| SwitchError::NotFound(format!("adapter {}", member.adapter_id)))?;

            rt.active_member_chain
                .write()
                .await
                .insert(member.handle.clone(), chain_id.to_string());

            let is_orchestrator_context = matches!(
                purpose,
                TurnPurpose::OrchestratorTriage
                    | TurnPurpose::ConflictResolution { .. }
                    | TurnPurpose::MemoryCuration
            );

            let (work_cwd, work_branch) = self
                .prepare_member_cwd(
                    &session,
                    &project_root,
                    &mut member,
                    is_orchestrator_context,
                )
                .await?;

            let rules = Arc::new(RwLock::new(self.store.list_permission_rules(session_id)?));
            let mcp_url = self.internal_mcp_url.read().await.clone();
            let mcp_spec = if !mcp_url.is_empty() {
                Some(build_mcp_bridge_spec(
                    &self.self_exe,
                    &mcp_url,
                    session_id,
                    &member.handle,
                    &self.bridge_token,
                ))
            } else {
                None
            };

            let traj = self.store.start_trajectory(StartTrajectoryParams {
                turn_id: None,
                session_id: session_id.to_string(),
                member_handle: member.handle.clone(),
                adapter_id: member.adapter_id.clone(),
                model: task.model.clone().or_else(|| member.active_model.clone()),
                effort: task.effort.clone().or_else(|| member.active_effort.clone()),
                worktree_branch: work_branch.clone(),
            })?;
            let turn_id = traj.turn_id.clone();

            self.set_member_status(
                session_id,
                &member.handle,
                MemberStatus::Spawning,
                Some(turn_id.clone()),
            )
            .await;

            let callbacks = Arc::new(SwitchCallbacks {
                engine: Arc::downgrade(self),
                session_id: session_id.to_string(),
                member_handle: member.handle.clone(),
                worktree: work_cwd.clone(),
                touched_files: Mutex::new(HashSet::new()),
                counters: Mutex::new(TurnCounters::default()),
            });

            let spawn_spec = MemberSpawnSpec {
                session_id: session_id.to_string(),
                member_handle: member.handle.clone(),
                adapter_id: member.adapter_id.clone(),
                adapter: adapter_cfg,
                project_root: project_root.clone(),
                member_worktree: work_cwd.clone(),
                permission_policy: session.permission_policy,
                existing_acp_session_id: member.acp_session_id.clone().filter(|_| {
                    previous_cwd.as_deref().map(Path::new) == Some(dunce::simplified(&work_cwd))
                }),
                extra_env: Vec::new(),
                mcp_bridge_spec: mcp_spec,
                rules,
            };

            let managed = match self
                .supervisor
                .get_or_spawn(spawn_spec, callbacks.clone())
                .await
            {
                Ok(m) => m,
                Err(e) => {
                    self.set_member_status(session_id, &member.handle, MemberStatus::Error, None)
                        .await;
                    let _ = self.store.finish_trajectory(
                        &turn_id,
                        TurnStatus::Failed,
                        &e.to_string(),
                        &[],
                        &DiffStat::default(),
                        None,
                    );
                    if !matches!(
                        purpose,
                        TurnPurpose::MemoryCuration | TurnPurpose::PeerConsult { .. }
                    ) && let Ok(message) = self.store.append_message(NewMessageParams {
                        id: None,
                        session_id: session_id.into(),
                        chain_id: chain_id.into(),
                        parent_msg_id: None,
                        sender_kind: SenderKind::System,
                        sender_id: member.handle.clone(),
                        recipients: Vec::new(),
                        kind: MessageKind::SystemNotice,
                        content: format!("Could not start @{}: {e}", member.handle),
                        turn_id: Some(turn_id.clone()),
                        metadata: json!({"error": true}),
                    }) {
                        let _ = self.events_tx.send(WsEvent::ChatMessageCreated { message });
                    }
                    return Err(e.into());
                }
            };

            *managed.active_turn_id.write().await = Some(turn_id.clone());

            let adapter_pref = self
                .store
                .get_adapter_model_pref(&member.adapter_id)
                .ok()
                .flatten();
            let wanted_model = task
                .model
                .as_deref()
                .or(member.active_model.as_deref())
                .or_else(|| {
                    adapter_pref
                        .as_ref()
                        .and_then(|p| p.default_model.as_deref())
                });
            let wanted_effort = match &purpose {
                TurnPurpose::MemoryCuration => Some("low"),
                _ => task
                    .effort
                    .as_deref()
                    .or(member.active_effort.as_deref())
                    .or_else(|| {
                        adapter_pref
                            .as_ref()
                            .and_then(|p| p.default_effort.as_deref())
                    }),
            };

            let (applied_model, applied_effort) = managed
                .client
                .apply_model_and_effort(wanted_model, wanted_effort)
                .await;

            let new_acp_sid = managed.client.acp_session_id.read().await.clone();
            let saved_effort = if matches!(purpose, TurnPurpose::MemoryCuration) {
                None
            } else {
                applied_effort.as_deref().or(wanted_effort)
            };
            let _ = self.store.update_member_runtime(
                session_id,
                &member.handle,
                new_acp_sid.as_deref(),
                applied_model.as_deref().or(wanted_model),
                saved_effort,
                work_branch.as_deref(),
                Some(dunce::simplified(&work_cwd).to_string_lossy().as_ref()),
            );

            self.set_member_status(
                session_id,
                &member.handle,
                MemberStatus::Thinking,
                Some(turn_id.clone()),
            )
            .await;

            // Build token-conscious delta prompt
            let unseen = if new_acp_sid != member.acp_session_id {
                self.store.list_messages(session_id, None, 50)
            } else {
                self.store
                    .list_unseen_deltas(session_id, member.last_seen_msg_seq, 50)
            }
            .unwrap_or_default();
            let max_seen_seq = unseen
                .last()
                .map(|m| m.seq)
                .unwrap_or(member.last_seen_msg_seq);

            let full_prompt = self
                .compose_prompt(&session, &member, &unseen, &task.task_brief, &purpose)
                .await;

            // Create visible reply bubble for normal/orchestrator/conflict turns (not silent curation or consults which create consult threads)
            let visible_msg = if !matches!(
                purpose,
                TurnPurpose::MemoryCuration | TurnPurpose::PeerConsult { .. }
            ) {
                let sender_kind = if is_orchestrator_context {
                    SenderKind::Orchestrator
                } else {
                    SenderKind::Member
                };
                let placeholder = self.store.append_message(NewMessageParams {
                    id: None,
                    session_id: session_id.to_string(),
                    chain_id: chain_id.to_string(),
                    parent_msg_id: None,
                    sender_kind,
                    sender_id: member.handle.clone(),
                    recipients: Vec::new(),
                    kind: MessageKind::Text,
                    content: String::new(),
                    turn_id: Some(turn_id.clone()),
                    metadata: json!({
                        "model": applied_model.as_deref().or(wanted_model),
                        "effort": applied_effort.as_deref().or(wanted_effort),
                        "streaming": true,
                    }),
                })?;
                let _ = self.events_tx.send(WsEvent::ChatMessageCreated {
                    message: placeholder.clone(),
                });
                Some(placeholder)
            } else {
                None
            };

            let stream_listener = Arc::new(SwitchStreamListener {
                engine: Arc::downgrade(self),
                session_id: session_id.to_string(),
                member_handle: member.handle.clone(),
                message_id: visible_msg.as_ref().map(|m| m.id.clone()),
                callbacks: Arc::clone(&callbacks),
            });

            let pre_head_sha = get_head_sha(&work_cwd).await;
            let prompt_res = managed
                .client
                .prompt(&turn_id, &full_prompt, stream_listener)
                .await;
            *managed.active_turn_id.write().await = None;

            let _ = self
                .store
                .advance_member_last_seen(session_id, &member.handle, max_seen_seq);

            match prompt_res {
                Ok((_, prompt_output)) => {
                    let reply_text = prompt_output.final_text;
                    let intermediate_notes = prompt_output.intermediate_notes;
                    let approx_tokens = ((full_prompt.len() + reply_text.len()) / 4).max(20) as u64;
                    self.quota
                        .record_turn_tokens(&member.adapter_id, approx_tokens)
                        .await;
                    if let Some(acfg) = cfg.adapters.get(&member.adapter_id) {
                        let quota = Arc::clone(&self.quota);
                        let events = self.events_tx.clone();
                        let adapter_id = member.adapter_id.clone();
                        let provider = acfg.quota_provider.clone();
                        tokio::spawn(async move {
                            if provider != "manual" {
                                quota.invalidate_if_stale(&adapter_id, 45_000).await;
                            }
                            let snapshot = quota.snapshot(&adapter_id, &provider).await;
                            let _ = events.send(WsEvent::QuotaUpdated { snapshot });
                        });
                    }

                    let touched: Vec<String> = callbacks
                        .touched_files
                        .lock()
                        .await
                        .iter()
                        .cloned()
                        .collect();
                    let summary_short = truncate_str(&reply_text, 400);
                    let head_sha = get_head_sha(&work_cwd).await;
                    let _ = self.store.finish_trajectory(
                        &turn_id,
                        TurnStatus::Completed,
                        &summary_short,
                        &touched,
                        &DiffStat::default(),
                        head_sha.as_deref(),
                    );

                    if let Some(msg) = visible_msg {
                        let updated = self.store.update_message(
                            &msg.id,
                            &reply_text,
                            Some(json!({
                                "model": applied_model.as_deref().or(wanted_model),
                                "effort": applied_effort.as_deref().or(wanted_effort),
                                "streaming": false,
                                "intermediate_notes": intermediate_notes,
                            })),
                        )?;
                        let _ = self
                            .events_tx
                            .send(WsEvent::ChatMessageCompleted { message: updated });
                    }

                    self.set_member_status(session_id, &member.handle, MemberStatus::Idle, None)
                        .await;

                    let commit_changed = pre_head_sha != head_sha;
                    self.handle_post_turn_effects(
                        &session,
                        &project_root,
                        &member,
                        &work_cwd,
                        work_branch.as_deref(),
                        chain_id,
                        &turn_id,
                        &task.task_brief,
                        &reply_text,
                        &touched,
                        &purpose,
                        commit_changed,
                    )
                    .await;
                    self.publish_session_context(session_id).await;

                    Ok((turn_id, reply_text))
                }
                Err(AcpError::RateLimited(msg)) => {
                    self.quota.mark_rate_limited(&member.adapter_id, 300).await;
                    if let Some(acfg) = cfg.adapters.get(&member.adapter_id) {
                        let snap = self
                            .quota
                            .snapshot(&member.adapter_id, &acfg.quota_provider)
                            .await;
                        let _ = self
                            .events_tx
                            .send(WsEvent::QuotaUpdated { snapshot: snap });
                    }
                    let _ = self.store.finish_trajectory(
                        &turn_id,
                        TurnStatus::Failed,
                        &msg,
                        &[],
                        &DiffStat::default(),
                        None,
                    );
                    self.set_member_status(session_id, &member.handle, MemberStatus::Error, None)
                        .await;

                    // Automatic Orchestrator / peer failover when an executor hits 429
                    if !is_orchestrator_context {
                        self.attempt_429_failover(&session, chain_id, &member, &task, &msg)
                            .await;
                    }
                    if let Some(vmsg) = visible_msg {
                        let updated = self.store.update_message(
                            &vmsg.id,
                            &format!("Rate limit hit on @{}: {msg}", member.handle),
                            Some(json!({ "error": true })),
                        )?;
                        let _ = self
                            .events_tx
                            .send(WsEvent::ChatMessageCompleted { message: updated });
                    }
                    Err(SwitchError::Acp(AcpError::RateLimited(msg)))
                }
                Err(AcpError::Cancelled) => {
                    let _ = self.store.finish_trajectory(
                        &turn_id,
                        TurnStatus::Cancelled,
                        "Cancelled",
                        &[],
                        &DiffStat::default(),
                        None,
                    );
                    if let Some(vmsg) = visible_msg {
                        let updated = self.store.update_message(
                            &vmsg.id,
                            "[Cancelled]",
                            Some(json!({ "cancelled": true })),
                        )?;
                        let _ = self
                            .events_tx
                            .send(WsEvent::ChatMessageCompleted { message: updated });
                    }
                    self.set_member_status(session_id, &member.handle, MemberStatus::Idle, None)
                        .await;
                    Err(SwitchError::Cancelled)
                }
                Err(err) => {
                    let _ = self.store.finish_trajectory(
                        &turn_id,
                        TurnStatus::Failed,
                        &err.to_string(),
                        &[],
                        &DiffStat::default(),
                        None,
                    );
                    if let Some(vmsg) = visible_msg {
                        let updated = self.store.update_message(
                            &vmsg.id,
                            &format!("Error in @{}: {err}", member.handle),
                            Some(json!({ "error": true })),
                        )?;
                        let _ = self
                            .events_tx
                            .send(WsEvent::ChatMessageCompleted { message: updated });
                    }
                    self.set_member_status(session_id, &member.handle, MemberStatus::Error, None)
                        .await;
                    Err(err.into())
                }
            }
        })
    }

    async fn prepare_member_cwd(
        &self,
        session: &Session,
        project_root: &Path,
        member: &mut SessionMember,
        is_orchestrator_context: bool,
    ) -> Result<(PathBuf, Option<String>), SwitchError> {
        if session.isolation_mode == IsolationMode::SharedCwd {
            return Ok((project_root.to_path_buf(), None));
        }

        let int = if let (Some(br), Some(p)) = (
            session.integration_branch.as_deref(),
            session.integration_wt_path.as_deref(),
        ) {
            let path = PathBuf::from(p);
            if path.join(".git").exists() {
                (br.to_string(), path)
            } else {
                let created =
                    ensure_session_integration_worktree(project_root, &session.id).await?;
                let _ = self.store.update_session_git_state(
                    &session.id,
                    created.base_branch.as_deref(),
                    Some(&created.base_commit_sha),
                    Some(&created.branch),
                    Some(dunce::simplified(&created.path).to_string_lossy().as_ref()),
                    IsolationMode::WorktreePerMember,
                );
                (created.branch, created.path)
            }
        } else {
            let created = ensure_session_integration_worktree(project_root, &session.id).await?;
            let _ = self.store.update_session_git_state(
                &session.id,
                created.base_branch.as_deref(),
                Some(&created.base_commit_sha),
                Some(&created.branch),
                Some(dunce::simplified(&created.path).to_string_lossy().as_ref()),
                IsolationMode::WorktreePerMember,
            );
            (created.branch, created.path)
        };

        if is_orchestrator_context {
            return Ok((int.1, Some(int.0)));
        }

        let m_wt =
            ensure_member_worktree(project_root, &session.id, &int.0, &member.handle).await?;
        member.worktree_branch = Some(m_wt.branch.clone());
        member.worktree_path = Some(dunce::simplified(&m_wt.path).to_string_lossy().into_owned());
        Ok((m_wt.path, Some(m_wt.branch)))
    }

    async fn compose_prompt(
        &self,
        session: &Session,
        member: &SessionMember,
        unseen: &[ChatMessage],
        task_brief: &str,
        purpose: &TurnPurpose,
    ) -> String {
        let mut buf = String::new();

        if !member.persona_prompt.trim().is_empty() {
            buf.push_str(&format!(
                "<persona_instructions handle=\"@{}\">{}</persona_instructions>\n",
                member.handle, member.persona_prompt
            ));
        }

        if !session.working_summary.trim().is_empty() {
            buf.push_str(&format!(
                "<session_compacted_summary>\n{}\n</session_compacted_summary>\n",
                session.working_summary.trim()
            ));
        }

        if !unseen.is_empty() {
            buf.push_str(&format!(
                "<group_chat_updates session=\"{}\" your_handle=\"@{}\">\n",
                session.title, member.handle
            ));
            for m in unseen {
                buf.push_str(&format!(
                    "  <msg seq=\"{}\" from=\"{}\" kind=\"{}\">{}</msg>\n",
                    m.seq,
                    m.sender_id,
                    m.kind.as_str(),
                    truncate_str(&m.content, 600)
                ));
            }
            buf.push_str("</group_chat_updates>\n");
        }

        match purpose {
            TurnPurpose::OrchestratorTriage => {
                let cfg = self.config.read().await.clone();
                let members = self.store.list_members(&session.id).unwrap_or_default();
                let mut opts = Vec::new();
                for m in &members {
                    let discovered = self
                        .supervisor
                        .get_discovered_options(&session.id, &m.handle)
                        .await;
                    opts.push((m.adapter_id.clone(), discovered));
                }
                let telemetry = build_orchestrator_telemetry_block(
                    &self.store,
                    &cfg,
                    &self.quota,
                    session.project_id.as_deref(),
                    &members,
                    &opts,
                    task_brief,
                )
                .await;
                buf.push_str(&telemetry);
                buf.push_str(task_brief);
            }
            TurnPurpose::ConflictResolution {
                member_handle,
                member_branch,
                conflicting_files,
                diff_preview,
                source_turn_id,
            } => {
                buf.push_str(&build_conflict_escalation_prompt(
                    member_handle,
                    member_branch,
                    conflicting_files,
                    diff_preview,
                    source_turn_id,
                ));
            }
            _ => {
                buf.push_str(task_brief);
            }
        }

        buf
    }

    #[allow(clippy::too_many_arguments)]
    async fn handle_post_turn_effects(
        self: &Arc<Self>,
        session: &Session,
        project_root: &Path,
        member: &SessionMember,
        work_cwd: &Path,
        work_branch: Option<&str>,
        chain_id: &str,
        turn_id: &str,
        task_brief: &str,
        reply_text: &str,
        touched_files: &[String],
        purpose: &TurnPurpose,
        integration_committed_in_turn: bool,
    ) {
        if matches!(
            purpose,
            TurnPurpose::MemoryCuration | TurnPurpose::PeerConsult { .. }
        ) {
            return;
        }

        let mut merged_files = touched_files.to_vec();
        let mut conflict_triggered = false;

        // Orchestrator or conflict resolution edits directly inside `_integration` -> auto-commit & refresh diff
        if matches!(
            purpose,
            TurnPurpose::OrchestratorTriage | TurnPurpose::ConflictResolution { .. }
        ) && session.isolation_mode == IsolationMode::WorktreePerMember
            && let (Some(int_br), Some(int_wt_str)) = (
                session.integration_branch.as_deref(),
                session.integration_wt_path.as_deref(),
            )
        {
            let int_wt = PathBuf::from(int_wt_str);
            let rt = self.runtime(&session.id).await;
            let _merge_guard = rt.git_merge_lock.lock().await;
            let mut committed_dirty = false;
            if let Ok(Some((_, _, files))) =
                commit_worktree_if_dirty(&int_wt, &member.handle, task_brief).await
            {
                committed_dirty = true;
                if !files.is_empty() {
                    merged_files = files;
                }
            }
            if (committed_dirty || integration_committed_in_turn)
                && let Ok(report) = get_session_diff(
                    project_root,
                    &int_wt,
                    session.base_branch.as_deref(),
                    session.base_commit_sha.as_deref(),
                    int_br,
                )
                .await
            {
                let _ = self.events_tx.send(WsEvent::GitBranchUpdated {
                    session_id: session.id.clone(),
                    integration_branch: int_br.to_string(),
                    ahead_commits: report.ahead_commits,
                    diff_stat: report.diff_stat,
                });
            }
        }

        // Executor turn in isolated worktree -> zero-token clean merge first
        if !matches!(
            purpose,
            TurnPurpose::OrchestratorTriage | TurnPurpose::ConflictResolution { .. }
        ) && session.isolation_mode == IsolationMode::WorktreePerMember
            && let (Some(int_br), Some(int_wt_str), Some(mem_br)) = (
                session.integration_branch.as_deref(),
                session.integration_wt_path.as_deref(),
                work_branch,
            )
        {
            let int_wt = PathBuf::from(int_wt_str);
            let rt = self.runtime(&session.id).await;
            let _merge_guard = rt.git_merge_lock.lock().await;
            let has_orch = session.orchestrator_member.is_some();

            match integrate_member_turn(
                &int_wt,
                int_br,
                work_cwd,
                mem_br,
                &member.handle,
                task_brief,
                has_orch,
            )
            .await
            {
                Ok(MergeOutcome::NoChanges) => {}
                Ok(MergeOutcome::Clean {
                    merged_sha,
                    diff_stat,
                    touched_files: git_touched,
                }) => {
                    if !git_touched.is_empty() {
                        merged_files = git_touched;
                    }
                    let notice = self.store.append_message(NewMessageParams {
                        id: None,
                        session_id: session.id.clone(),
                        chain_id: chain_id.to_string(),
                        parent_msg_id: None,
                        sender_kind: SenderKind::System,
                        sender_id: "git-broker".into(),
                        recipients: Vec::new(),
                        kind: MessageKind::MergeNotice,
                        content: format!(
                            "Auto-merged @{} ({} files, +{}/-{}) into `{int_br}`",
                            member.handle,
                            diff_stat.files_changed,
                            diff_stat.insertions,
                            diff_stat.deletions
                        ),
                        turn_id: Some(turn_id.to_string()),
                        metadata: json!({
                            "merged_sha": merged_sha,
                            "diff_stat": diff_stat,
                        }),
                    });
                    if let Ok(m) = notice {
                        let _ = self
                            .events_tx
                            .send(WsEvent::ChatMessageCreated { message: m });
                    }
                    let (ahead_commits, total_stat) = match get_session_diff(
                        project_root,
                        &int_wt,
                        session.base_branch.as_deref(),
                        session.base_commit_sha.as_deref(),
                        int_br,
                    )
                    .await
                    {
                        Ok(report) => (report.ahead_commits, report.diff_stat),
                        Err(_) => (1, diff_stat),
                    };
                    let _ = self.events_tx.send(WsEvent::GitBranchUpdated {
                        session_id: session.id.clone(),
                        integration_branch: int_br.to_string(),
                        ahead_commits,
                        diff_stat: total_stat,
                    });

                    // Fast-forward idle peer worktrees if enabled
                    if self.config.read().await.git.sync_idle_worktrees_after_merge
                        && let Ok(all_members) = self.store.list_members(&session.id)
                    {
                        for peer in all_members {
                            if peer.handle != member.handle
                                && let Some(p_wt) = peer.worktree_path
                                && Path::new(&p_wt) != int_wt
                            {
                                let peer_lock = {
                                    let mut locks = rt.member_locks.lock().await;
                                    Arc::clone(locks.entry(peer.handle).or_default())
                                };
                                if let Ok(_idle) = peer_lock.try_lock() {
                                    let _ = sync_member_worktree(Path::new(&p_wt), int_br).await;
                                }
                            }
                        }
                    }
                }
                Ok(MergeOutcome::Conflict {
                    conflicting_files,
                    diff_preview,
                    member_branch,
                }) => {
                    conflict_triggered = true;
                    let _ = self.events_tx.send(WsEvent::GitConflictEscalated {
                        session_id: session.id.clone(),
                        member_handle: member.handle.clone(),
                        conflicting_files: conflicting_files.clone(),
                    });
                    if let Some(orch) = session.orchestrator_member.clone() {
                        let engine = Arc::clone(self);
                        let sid = session.id.clone();
                        let cid = chain_id.to_string();
                        let tid = turn_id.to_string();
                        let mh = member.handle.clone();
                        tokio::spawn(async move {
                            let _ = engine
                                .execute_member_turn(
                                    &sid,
                                    &cid,
                                    DispatchTaskSpec {
                                        target: orch,
                                        model: None,
                                        effort: Some("high".into()),
                                        task_brief: "Resolve worktree merge conflict".into(),
                                        mode: "parallel".into(),
                                    },
                                    TurnPurpose::ConflictResolution {
                                        member_handle: mh,
                                        member_branch,
                                        conflicting_files,
                                        diff_preview,
                                        source_turn_id: tid,
                                    },
                                )
                                .await;
                        });
                    }
                }
                Err(e) => {
                    warn!("git integration skipped/failed for {}: {e}", member.handle);
                }
            }
        }

        // Check whether Orchestrator already dispatched via MCP `orchestrator_dispatch`
        let rt = self.runtime(&session.id).await;
        let skip_mention_routing = if matches!(purpose, TurnPurpose::OrchestratorTriage) {
            rt.orchestrator_dispatched_turn
                .lock()
                .await
                .remove(chain_id)
        } else {
            false
        };

        // Trigger Orchestrator LLM memory curation pass (with guaranteed deterministic fallback)
        // both for Executor turns AND substantive direct Orchestrator turns.
        let should_curate = !conflict_triggered
            && (!matches!(purpose, TurnPurpose::OrchestratorTriage)
                || (!skip_mention_routing
                    && (!merged_files.is_empty() || reply_text.chars().count() >= 80)));
        if should_curate {
            self.trigger_post_job_memory_curation(
                session,
                member,
                chain_id,
                turn_id,
                task_brief,
                reply_text,
                &merged_files,
                purpose,
            )
            .await;
        }

        if !skip_mention_routing && !matches!(purpose, TurnPurpose::ConflictResolution { .. }) {
            let members = self.store.list_members(&session.id).unwrap_or_default();
            let valid_handles: HashSet<String> = members.into_iter().map(|m| m.handle).collect();
            let parsed = parse_mentions(reply_text, &valid_handles);
            let specs: Vec<DispatchTaskSpec> = parsed
                .targets
                .into_iter()
                .filter(|t| t.handle != member.handle)
                .map(|t| DispatchTaskSpec {
                    target: t.handle,
                    model: t.model_override,
                    effort: t.effort_override,
                    task_brief: reply_text.to_string(),
                    mode: "parallel".into(),
                })
                .collect();
            if !specs.is_empty() {
                let _ = self
                    .enqueue_hop_dispatches(&session.id, chain_id, specs)
                    .await;
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn trigger_post_job_memory_curation(
        self: &Arc<Self>,
        session: &Session,
        member: &SessionMember,
        chain_id: &str,
        turn_id: &str,
        task_brief: &str,
        reply_text: &str,
        touched_files: &[String],
        purpose: &TurnPurpose,
    ) {
        let Some(ref project_id) = session.project_id else {
            return;
        };
        let curation_mode = self.config.read().await.memory.curation_mode.clone();

        // If this was ConflictResolution, Orchestrator already had `memory_curate_job` in its prompt;
        // ensure a deterministic fallback record lands if `memory_curate_job` was skipped.
        if let TurnPurpose::ConflictResolution { source_turn_id, .. } = purpose {
            if !self.store.has_job_for_turn(source_turn_id).unwrap_or(false)
                && !self.store.has_job_for_turn(turn_id).unwrap_or(false)
            {
                self.record_job_with_summary(
                    project_id,
                    &session.id,
                    turn_id,
                    member,
                    task_brief,
                    reply_text,
                    touched_files,
                    "system",
                );
            }
            return;
        }

        if curation_mode == "orchestrator_llm"
            && let Some(orch) = session.orchestrator_member.clone()
        {
            let prompt = build_memory_curation_prompt(MemoryCurationPromptInput {
                member_handle: &member.handle,
                adapter_id: &member.adapter_id,
                model: member.active_model.as_deref(),
                effort: member.active_effort.as_deref(),
                turn_id,
                task_brief,
                touched_modules: touched_files,
                reply_summary: reply_text,
            });
            let engine = Arc::clone(self);
            let sid = session.id.clone();
            let pid = project_id.clone();
            let cid = chain_id.to_string();
            let m_clone = member.clone();
            let tid = turn_id.to_string();
            let brief = task_brief.to_string();
            let reply = reply_text.to_string();
            let files = touched_files.to_vec();
            let orch_handle = orch.clone();

            tokio::spawn(async move {
                let res = tokio::time::timeout(
                    Duration::from_secs(25),
                    engine.execute_member_turn(
                        &sid,
                        &cid,
                        DispatchTaskSpec {
                            target: orch,
                            model: None,
                            effort: Some("low".into()),
                            task_brief: prompt,
                            mode: "parallel".into(),
                        },
                        TurnPurpose::MemoryCuration,
                    ),
                )
                .await;

                if engine.store.has_job_for_turn(&tid).unwrap_or(false) {
                    return;
                }

                match res {
                    Ok(Ok((_, llm_summary)))
                        if !llm_summary.trim().is_empty()
                            && !llm_summary.trim_start().starts_with('[') =>
                    {
                        engine.record_job_with_summary(
                            &pid,
                            &sid,
                            &tid,
                            &m_clone,
                            &brief,
                            llm_summary.trim(),
                            &files,
                            &orch_handle,
                        );
                    }
                    _ => {
                        engine.record_job_with_summary(
                            &pid, &sid, &tid, &m_clone, &brief, &reply, &files, "system",
                        );
                    }
                }
            });
            return;
        }

        self.record_job_with_summary(
            project_id,
            &session.id,
            turn_id,
            member,
            task_brief,
            reply_text,
            touched_files,
            "system",
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn record_job_with_summary(
        &self,
        project_id: &str,
        session_id: &str,
        turn_id: &str,
        member: &SessionMember,
        task_brief: &str,
        curated_summary: &str,
        touched_files: &[String],
        curated_by: &str,
    ) {
        if let Ok(job) = self.store.record_curated_job(RecordEpisodicJobParams {
            project_id: project_id.to_string(),
            session_id: Some(session_id.to_string()),
            turn_id: Some(turn_id.to_string()),
            member_handle: member.handle.clone(),
            adapter_id: member.adapter_id.clone(),
            model: member.active_model.clone(),
            effort: member.active_effort.clone(),
            task_brief: truncate_str(task_brief, 240),
            curated_summary: truncate_str(curated_summary, 400),
            touched_modules: touched_files.to_vec(),
            outcome: "completed".into(),
            curated_by: curated_by.to_string(),
            pinned: false,
        }) {
            let _ = self.events_tx.send(WsEvent::MemoryJobCurated { job });
        }
    }

    async fn attempt_429_failover(
        self: &Arc<Self>,
        session: &Session,
        chain_id: &str,
        failed_member: &SessionMember,
        task: &DispatchTaskSpec,
        err_msg: &str,
    ) {
        let cfg = self.config.read().await.clone();
        let members = self.store.list_members(&session.id).unwrap_or_default();
        let quotas = self.quota.cached_snapshots(&cfg).await;
        let probes = probe_installed_adapters(&cfg);

        if let Some(alt) =
            pick_fallback_executor(&members, &quotas, &probes, Some(&failed_member.adapter_id))
        {
            let notice = self.store.append_message(NewMessageParams {
                id: None,
                session_id: session.id.clone(),
                chain_id: chain_id.to_string(),
                parent_msg_id: None,
                sender_kind: SenderKind::System,
                sender_id: "switch".into(),
                recipients: vec![alt.handle.clone()],
                kind: MessageKind::SystemNotice,
                content: format!(
                    "429 rate limit on @{} ({err_msg}); failing over to @{} automatically.",
                    failed_member.handle, alt.handle
                ),
                turn_id: None,
                metadata: json!({ "failover_from": failed_member.handle, "failover_to": alt.handle }),
            });
            if let Ok(m) = notice {
                let _ = self
                    .events_tx
                    .send(WsEvent::ChatMessageCreated { message: m });
            }

            let engine = Arc::clone(self);
            let sid = session.id.clone();
            let cid = chain_id.to_string();
            let next_task = DispatchTaskSpec {
                target: alt.handle,
                model: None,
                effort: task.effort.clone(),
                task_brief: task.task_brief.clone(),
                mode: task.mode.clone(),
            };
            tokio::spawn(async move {
                let _ = engine
                    .execute_member_turn(&sid, &cid, next_task, TurnPurpose::Normal)
                    .await;
            });
        }
    }

    async fn set_member_status(
        &self,
        session_id: &str,
        handle: &str,
        status: MemberStatus,
        turn_id: Option<String>,
    ) {
        let rt = self.runtime(session_id).await;
        rt.member_statuses
            .write()
            .await
            .insert(handle.to_string(), (status, turn_id.clone()));

        let member = self.store.get_member(session_id, handle).ok();
        let _ = self.events_tx.send(WsEvent::MemberStatusChanged {
            session_id: session_id.to_string(),
            handle: handle.to_string(),
            status,
            active_model: member.as_ref().and_then(|m| m.active_model.clone()),
            active_effort: member.as_ref().and_then(|m| m.active_effort.clone()),
            turn_id,
        });
    }

    pub async fn compute_session_context(
        &self,
        session_id: &str,
    ) -> Result<SessionContextReport, SwitchError> {
        let session = self.store.get_session(session_id)?;
        let members = self.store.list_members(session_id)?;
        let available = self.available_members(&members).await;
        let messages = self.store.list_messages(session_id, None, 200)?;
        let trajectories = self.store.list_trajectories(session_id, None, 40)?;
        let jobs = session
            .project_id
            .as_deref()
            .and_then(|pid| self.store.recall_jobs(pid, None, 6).ok())
            .unwrap_or_default();
        let facts = session
            .project_id
            .as_deref()
            .and_then(|pid| self.store.list_facts(pid, 10).ok())
            .unwrap_or_default();

        let memory_base_tokens: u64 = jobs
            .iter()
            .map(|j| ((j.task_brief.len() + j.curated_summary.len()) / 4) as u64)
            .sum::<u64>()
            + facts
                .iter()
                .map(|f| (f.summary.len() / 4) as u64)
                .sum::<u64>();

        let active_msgs_slice = match messages
            .iter()
            .rposition(|m| m.metadata.get("compacted_context") == Some(&json!(true)))
        {
            Some(idx) => &messages[idx + 1..],
            None => &messages[..],
        };
        let chat_history_tokens: u64 = active_msgs_slice
            .iter()
            .map(|m| ((m.content.len() + m.sender_id.len()) / 4).max(1) as u64)
            .sum();

        let summary_tokens = (session.working_summary.len() / 4) as u64;
        let mut breakdowns = Vec::new();
        for m in members {
            if !available.contains(&m.handle) {
                continue;
            }
            let adapter_pref = self
                .store
                .get_adapter_model_pref(&m.adapter_id)
                .ok()
                .flatten();
            let effective_model = m
                .active_model
                .clone()
                .or_else(|| adapter_pref.and_then(|p| p.default_model));
            let is_orch = session.orchestrator_member.as_deref() == Some(m.handle.as_str());
            let system_and_persona_tokens =
                280 + summary_tokens + (m.persona_prompt.len() / 4) as u64;
            let memory_and_telemetry_tokens = memory_base_tokens + if is_orch { 320 } else { 80 };

            let mut agent_and_tool_tokens = 0u64;
            for t in trajectories
                .iter()
                .filter(|t| t.member_handle == m.handle && t.started_at >= session.created_at)
            {
                agent_and_tool_tokens += (t.summary.len() / 4) as u64;
                if let Ok(ev_page) = self.store.list_trajectory_events(&t.turn_id) {
                    let chars: usize = ev_page
                        .iter()
                        .take(30)
                        .map(|e| e.payload.to_string().len().min(600))
                        .sum();
                    agent_and_tool_tokens += (chars / 4) as u64;
                }
            }

            let catalog = builtin_adapter_catalog(&m.adapter_id);
            let live_usage = self
                .supervisor
                .get_latest_usage(session_id, &m.handle)
                .await;
            let (is_exact, used_tokens, context_window) = match live_usage {
                Some((used, size)) => (true, used, size),
                None => (
                    false,
                    system_and_persona_tokens
                        + memory_and_telemetry_tokens
                        + chat_history_tokens
                        + agent_and_tool_tokens,
                    catalog.default_context_window,
                ),
            };

            breakdowns.push(MemberContextBreakdown {
                handle: m.handle,
                adapter_id: m.adapter_id,
                model: effective_model,
                used_tokens,
                context_window,
                is_exact,
                system_and_persona_tokens,
                memory_and_telemetry_tokens,
                chat_history_tokens,
                agent_and_tool_tokens,
            });
        }

        Ok(SessionContextReport {
            session_id: session_id.to_string(),
            members: breakdowns,
        })
    }

    pub async fn compact_session(&self, session_id: &str) -> Result<ChatMessage, SwitchError> {
        let session = self.store.get_session(session_id)?;
        let messages = self.store.list_messages(session_id, None, 100)?;
        let mut lines = Vec::new();
        if !session.working_summary.trim().is_empty() {
            lines.push(format!("Prior summary: {}", session.working_summary.trim()));
        }
        for m in messages.iter().rev().take(24).rev() {
            if matches!(m.kind, MessageKind::Text | MessageKind::ConsultThread)
                && !m.content.trim().is_empty()
            {
                lines.push(format!(
                    "{}: {}",
                    m.sender_id,
                    truncate_str(m.content.trim(), 160)
                ));
            }
        }
        let summary = truncate_chars(&lines.join("\n"), 1500);
        let updated = self.store.patch_session(
            session_id,
            PatchSessionParams {
                working_summary: Some(summary),
                ..Default::default()
            },
        )?;
        let _ = self.store.clear_session_acp_ids(session_id);
        self.supervisor.reset_session_clients(session_id).await;

        let notice = self.store.append_message(NewMessageParams {
            id: None,
            session_id: session_id.to_string(),
            chain_id: format!("ch_compact_{}", uuid::Uuid::new_v4().simple()),
            parent_msg_id: None,
            sender_kind: SenderKind::System,
            sender_id: "switch".into(),
            recipients: Vec::new(),
            kind: MessageKind::SystemNotice,
            content: "Context compacted — active harness sessions reset with a concise summary."
                .into(),
            turn_id: None,
            metadata: json!({ "compacted_context": true }),
        })?;
        let _ = self.events_tx.send(WsEvent::ChatMessageCreated {
            message: notice.clone(),
        });
        let _ = self
            .events_tx
            .send(WsEvent::SessionUpdated { session: updated });
        self.publish_session_context(session_id).await;
        Ok(notice)
    }

    async fn publish_session_context(&self, session_id: &str) {
        if let Ok(report) = self.compute_session_context(session_id).await {
            let _ = self.events_tx.send(WsEvent::ContextUpdated {
                session_id: session_id.to_string(),
                report,
            });
        }
    }

    async fn trigger_auto_session_title(self: &Arc<Self>, session_id: &str, user_prompt: &str) {
        let Ok(session) = self.store.get_session(session_id) else {
            return;
        };
        if session.title != "New session" {
            return;
        }
        let Ok(repo_root) = self.resolve_session_repo_root(&session) else {
            return;
        };
        let Ok(members) = self.store.list_members(session_id) else {
            return;
        };
        let available = self.available_members(&members).await;
        let picker = session
            .orchestrator_member
            .as_deref()
            .filter(|h| available.contains(*h))
            .and_then(|h| members.iter().find(|m| m.handle == h))
            .or_else(|| members.iter().find(|m| available.contains(&m.handle)));

        let cfg = self.config.read().await.clone();
        let generated = if let Some(target_member) = picker
            && let Some(adapter_cfg) = cfg.adapters.get(&target_member.adapter_id)
        {
            let cwd = session
                .integration_wt_path
                .as_deref()
                .map(PathBuf::from)
                .filter(|p| p.is_dir())
                .filter(|_| repo_root.exists())
                .unwrap_or_else(|| {
                    if repo_root.is_dir() {
                        repo_root.clone()
                    } else {
                        std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
                    }
                });
            let prompt = format!(
                "<session_title_request>\n\
                 Write a concise 3 to 6 word title (plain text only, no quotes, no trailing period) \
                 summarizing a coding session that starts with this user message:\n\
                 {}\n\
                 </session_title_request>",
                truncate_str(user_prompt, 500)
            );
            self.supervisor
                .run_ephemeral_prompt(
                    adapter_cfg,
                    &cwd,
                    Some("low"),
                    &prompt,
                    Duration::from_secs(20),
                )
                .await
                .ok()
        } else {
            None
        };

        let title = generated
            .as_deref()
            .and_then(clean_generated_title)
            .unwrap_or_else(|| fallback_title_from_prompt(user_prompt));

        if let Ok(curr) = self.store.get_session(session_id)
            && curr.title == "New session"
            && let Ok(updated) = self.store.patch_session(
                session_id,
                PatchSessionParams {
                    title: Some(title),
                    ..Default::default()
                },
            )
        {
            let _ = self
                .events_tx
                .send(WsEvent::SessionUpdated { session: updated });
        }
    }
}

fn extract_workflow_slash_commands(raw: &str) -> (Vec<String>, String) {
    let mut cmds = Vec::new();
    let mut remaining = Vec::new();
    let mut in_fence = false;
    for line in raw.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") {
            in_fence = !in_fence;
            remaining.push(line.to_string());
            continue;
        }
        if in_fence {
            remaining.push(line.to_string());
            continue;
        }
        let mut kept_words = Vec::new();
        for tok in line.split_whitespace() {
            let lower = tok.to_ascii_lowercase();
            let stripped = lower.trim_end_matches(|c: char| ",.;:!?".contains(c));
            if matches!(
                stripped,
                "/plan" | "/review" | "/test" | "/fix" | "/compact"
            ) {
                let name = stripped.trim_start_matches('/').to_string();
                if !cmds.contains(&name) {
                    cmds.push(name);
                }
            } else {
                kept_words.push(tok);
            }
        }
        remaining.push(kept_words.join(" "));
    }
    (cmds, remaining.join("\n"))
}

fn build_slash_directive_prefix(cmds: &[String]) -> String {
    let mut parts = Vec::new();
    for cmd in cmds {
        match cmd.as_str() {
            "plan" => parts.push("Slash command `/plan`: Analyze the codebase carefully and present an explicit step-by-step plan before modifying files."),
            "review" => parts.push("Slash command `/review`: Review recent changes critically for bugs, race conditions, security flaws, and test coverage."),
            "test" => parts.push("Slash command `/test`: Write or run tests covering the requested functionality and verify all cases pass."),
            "fix" => parts.push("Slash command `/fix`: Identify the root cause cleanly and apply a targeted, verified fix."),
            _ => {}
        }
    }
    if parts.is_empty() {
        String::new()
    } else {
        format!("<slash_commands>\n{}\n</slash_commands>", parts.join("\n"))
    }
}

fn clean_generated_title(raw: &str) -> Option<String> {
    let line = raw
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())?
        .trim_matches(|c| matches!(c, '"' | '\'' | '`' | '*' | '#' | '.'))
        .trim();
    if line.is_empty() || line.eq_ignore_ascii_case("New session") {
        return None;
    }
    let words: Vec<&str> = line.split_whitespace().take(8).collect();
    let joined = words.join(" ");
    if joined.is_empty() {
        None
    } else {
        Some(truncate_chars(&joined, 60))
    }
}

fn fallback_title_from_prompt(raw: &str) -> String {
    let words: Vec<&str> = raw
        .split_whitespace()
        .filter(|w| !w.starts_with('@') && !w.starts_with('/'))
        .take(5)
        .collect();
    if words.is_empty() {
        "Quick session".into()
    } else {
        truncate_chars(&words.join(" "), 60)
    }
}

fn truncate_chars(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(max_chars).collect();
        out.push_str("...");
        out
    }
}

fn validate_available_targets<'a>(
    targets: impl IntoIterator<Item = &'a str>,
    available: &HashSet<String>,
) -> Result<(), SwitchError> {
    let unavailable: Vec<_> = targets
        .into_iter()
        .filter(|target| !available.contains(*target))
        .map(|target| format!("@{target}"))
        .collect();
    if unavailable.is_empty() {
        return Ok(());
    }
    let mut alternatives: Vec<_> = available
        .iter()
        .map(|target| format!("@{target}"))
        .collect();
    alternatives.sort();
    Err(SwitchError::Validation(format!(
        "Unavailable harnesses: {}. Available members: {}.",
        unavailable.join(", "),
        if alternatives.is_empty() {
            "none".into()
        } else {
            alternatives.join(", ")
        }
    )))
}

struct SwitchCallbacks {
    engine: std::sync::Weak<SwitchEngine>,
    session_id: String,
    member_handle: String,
    worktree: PathBuf,
    touched_files: Mutex<HashSet<String>>,
    counters: Mutex<TurnCounters>,
}

#[derive(Default)]
struct TurnCounters {
    tool_ids: HashSet<String>,
    tool_calls: u32,
    file_edits: u32,
    thoughts: u32,
}

impl TurnCounters {
    fn record(&mut self, kind: TrajectoryEventKind, payload: &Value) -> bool {
        match kind {
            TrajectoryEventKind::ToolCall => {
                if let Some(id) = payload.get("toolCallId").and_then(Value::as_str)
                    && !self.tool_ids.insert(id.to_string())
                {
                    return false;
                }
                self.tool_calls += 1;
            }
            TrajectoryEventKind::Diff => self.file_edits += 1,
            TrajectoryEventKind::Thought => self.thoughts += 1,
            _ => return false,
        }
        true
    }
}

#[async_trait]
impl BrokerCallbacks for SwitchCallbacks {
    async fn request_user_permission(
        &self,
        prompt: PendingPermissionView,
    ) -> Result<PermissionResolution, AcpError> {
        let Some(engine) = self.engine.upgrade() else {
            return Ok(PermissionResolution::Cancelled);
        };
        let (tx, rx) = oneshot::channel();
        let req_id = prompt.request_id.clone();
        let rt = engine.runtime(&self.session_id).await;

        rt.pending_permissions.lock().await.insert(
            req_id.clone(),
            PendingPermissionEntry {
                view: prompt.clone(),
                tx,
            },
        );

        let _ = engine.store.append_message(NewMessageParams {
            id: None,
            session_id: self.session_id.clone(),
            chain_id: engine
                .active_chain_for_member(&self.session_id, &self.member_handle)
                .await,
            parent_msg_id: None,
            sender_kind: SenderKind::System,
            sender_id: self.member_handle.clone(),
            recipients: Vec::new(),
            kind: MessageKind::PermissionCard,
            content: prompt.title.clone(),
            turn_id: prompt.turn_id.clone(),
            metadata: serde_json::to_value(&prompt).unwrap_or(Value::Null),
        });

        let _ = engine
            .events_tx
            .send(WsEvent::PermissionRequested { prompt });

        engine
            .set_member_status(
                &self.session_id,
                &self.member_handle,
                MemberStatus::WaitingPermission,
                None,
            )
            .await;

        let res = rx.await.unwrap_or(PermissionResolution::Cancelled);

        engine
            .set_member_status(
                &self.session_id,
                &self.member_handle,
                MemberStatus::Thinking,
                None,
            )
            .await;

        Ok(res)
    }

    async fn record_trajectory_event(
        &self,
        turn_id: &str,
        kind: TrajectoryEventKind,
        payload: Value,
    ) {
        let Some(engine) = self.engine.upgrade() else {
            return;
        };
        if kind == TrajectoryEventKind::Diff
            && let Some(path) = payload.get("path").and_then(Value::as_str)
        {
            let path = Path::new(path);
            if let Ok(relative) = path.strip_prefix(&self.worktree) {
                self.record_touched_file(turn_id, relative.to_string_lossy().into_owned())
                    .await;
            } else if path.is_relative() {
                self.record_touched_file(turn_id, path.to_string_lossy().into_owned())
                    .await;
            }
        }
        let mut counters = self.counters.lock().await;
        let changed = counters.record(kind, &payload);
        if let Ok(inserted) = engine
            .store
            .append_trajectory_events(turn_id, &[(kind, payload)])
        {
            for ev in inserted {
                let _ = engine.events_tx.send(WsEvent::TrajectoryEvent {
                    session_id: self.session_id.clone(),
                    turn_id: turn_id.to_string(),
                    event: ev,
                });
            }
        }

        if changed {
            let _ = engine.events_tx.send(WsEvent::TrajectoryCounters {
                session_id: self.session_id.clone(),
                turn_id: turn_id.to_string(),
                tool_calls: counters.tool_calls,
                file_edits: counters.file_edits,
                thoughts: counters.thoughts,
            });
        }
    }

    async fn record_touched_file(&self, _turn_id: &str, rel_path: String) {
        self.touched_files.lock().await.insert(rel_path);
    }
}

struct SwitchStreamListener {
    engine: std::sync::Weak<SwitchEngine>,
    session_id: String,
    member_handle: String,
    message_id: Option<String>,
    callbacks: Arc<SwitchCallbacks>,
}

#[async_trait]
impl TurnStreamListener for SwitchStreamListener {
    async fn on_text_delta(&self, turn_id: &str, delta: String, replace: bool) {
        if let Some(msg_id) = &self.message_id
            && let Some(engine) = self.engine.upgrade()
        {
            let _ = engine.events_tx.send(WsEvent::ChatMessageDelta {
                session_id: self.session_id.clone(),
                message_id: msg_id.clone(),
                turn_id: turn_id.to_string(),
                member_handle: self.member_handle.clone(),
                delta,
                replace,
            });
        }
    }

    async fn on_thought_delta(&self, turn_id: &str, delta: String) {
        self.callbacks
            .record_trajectory_event(
                turn_id,
                TrajectoryEventKind::Thought,
                json!({ "text": delta }),
            )
            .await;
    }

    async fn on_trajectory_event(&self, turn_id: &str, kind: TrajectoryEventKind, payload: Value) {
        self.callbacks
            .record_trajectory_event(turn_id, kind, payload)
            .await;
    }

    async fn on_config_options_updated(&self, _options: Vec<DiscoveredConfigOption>) {}
}
