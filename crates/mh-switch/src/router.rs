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
    QuotaTracker, TurnStreamListener, build_mcp_bridge_spec, ensure_member_worktree,
    ensure_session_integration_worktree, get_head_sha, integrate_member_turn,
    probe_installed_adapters, sync_member_worktree,
};
use mh_core::{
    AppConfig, ChatMessage, DiffStat, DiscoveredConfigOption, DispatchTaskSpec, IsolationMode,
    MemberRole, MemberStatus, MessageKind, PendingPermissionView, SenderKind, Session,
    SessionMember, TrajectoryEventKind, TurnStatus, WaitForGraph, WsEvent, parse_mentions,
};
use mh_store::{NewMessageParams, RecordEpisodicJobParams, StartTrajectoryParams, Store};

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
            runtimes: Mutex::new(HashMap::new()),
        })
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

    /// Posts a user message and initiates routing (either explicit `@mentions` in parallel
    /// or the configured Orchestrator).
    pub async fn handle_user_message(
        self: &Arc<Self>,
        session_id: &str,
        content: String,
    ) -> Result<ChatMessage, SwitchError> {
        let session = self.store.get_session(session_id)?;
        let members = self.store.list_members(session_id)?;
        if members.is_empty() {
            return Err(SwitchError::Validation(
                "session has no active members".into(),
            ));
        }

        let valid_handles: HashSet<String> = members.iter().map(|m| m.handle.clone()).collect();
        let parsed = parse_mentions(&content, &valid_handles);
        let chain_id = format!("ch_{}", &uuid::Uuid::new_v4().simple().to_string()[..10]);

        let mut recipients: Vec<String> = Vec::new();
        if parsed.has_orchestrator || parsed.is_unmentioned() {
            if let Some(orch) = session.orchestrator_member.clone() {
                recipients.push(orch);
            } else if let Some(first) = members.first() {
                recipients.push(first.handle.clone());
            }
        } else if parsed.has_all {
            for m in &members {
                if session.orchestrator_member.as_deref() != Some(&m.handle)
                    || m.role != MemberRole::Orchestrator
                {
                    recipients.push(m.handle.clone());
                }
            }
        } else {
            for t in &parsed.targets {
                recipients.push(t.handle.clone());
            }
        }

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
                if let Some(orch) = session.orchestrator_member.clone() {
                    vec![(
                        DispatchTaskSpec {
                            target: orch,
                            model: None,
                            effort: None,
                            task_brief: content,
                            mode: "parallel".into(),
                        },
                        TurnPurpose::OrchestratorTriage,
                    )]
                } else {
                    vec![(
                        DispatchTaskSpec {
                            target: recipients[0].clone(),
                            model: None,
                            effort: None,
                            task_brief: content,
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
                                model: None,
                                effort: None,
                                task_brief: content.clone(),
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
                                model: t.model_override,
                                effort: t.effort_override,
                                task_brief: content.clone(),
                                mode: "parallel".into(),
                            },
                            TurnPurpose::Normal,
                        )
                    })
                    .collect()
            };

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
        let _ = self.store.get_member(session_id, target)?;
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
        dispatches: Vec<DispatchTaskSpec>,
    ) -> Result<String, SwitchError> {
        let session = self.store.get_session(session_id)?;
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
        let entry = rt
            .pending_permissions
            .lock()
            .await
            .remove(request_id)
            .ok_or_else(|| SwitchError::NotFound(format!("permission {request_id}")))?;

        if let Some(ref pat) = allow_pattern {
            let _ = self
                .store
                .add_permission_rule(session_id, &entry.view.tool_kind, pat, "allow");
        }

        let decision = if option_id.starts_with("reject") {
            "rejected"
        } else {
            "allowed"
        };

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

            if *rt.cancel_epoch.read().await != epoch_at_start {
                return Err(SwitchError::Cancelled);
            }

            let session = self.store.get_session(session_id)?;
            let project = self.store.get_project(&session.project_id)?;
            let mut member = self.store.get_member(session_id, &task.target)?;
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

            let project_root = PathBuf::from(&project.root_path);
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
                touched_files: Mutex::new(HashSet::new()),
                tool_count: Mutex::new(0),
                edit_count: Mutex::new(0),
                thought_count: Mutex::new(0),
            });

            let spawn_spec = MemberSpawnSpec {
                session_id: session_id.to_string(),
                member_handle: member.handle.clone(),
                adapter_id: member.adapter_id.clone(),
                adapter: adapter_cfg,
                project_root: project_root.clone(),
                member_worktree: work_cwd.clone(),
                permission_policy: session.permission_policy,
                existing_acp_session_id: member.acp_session_id.clone(),
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
                    return Err(e.into());
                }
            };

            *managed.active_turn_id.write().await = Some(turn_id.clone());

            let wanted_model = task.model.as_deref().or(member.active_model.as_deref());
            let wanted_effort = match &purpose {
                TurnPurpose::MemoryCuration => Some("low"),
                _ => task.effort.as_deref().or(member.active_effort.as_deref()),
            };

            let (applied_model, applied_effort) = managed
                .client
                .apply_model_and_effort(wanted_model, wanted_effort)
                .await;

            let new_acp_sid = managed.client.acp_session_id.read().await.clone();
            let _ = self.store.update_member_runtime(
                session_id,
                &member.handle,
                new_acp_sid.as_deref(),
                applied_model.as_deref().or(wanted_model),
                applied_effort.as_deref().or(wanted_effort),
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
            let unseen = self
                .store
                .list_unseen_deltas(session_id, member.last_seen_msg_seq, 50)
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

            let prompt_res = managed
                .client
                .prompt(&turn_id, &full_prompt, stream_listener)
                .await;
            *managed.active_turn_id.write().await = None;

            let _ = self
                .store
                .advance_member_last_seen(session_id, &member.handle, max_seen_seq);

            match prompt_res {
                Ok((_, reply_text)) => {
                    let approx_tokens = ((full_prompt.len() + reply_text.len()) / 4).max(20) as u64;
                    self.quota
                        .record_turn_tokens(&member.adapter_id, approx_tokens)
                        .await;
                    if let Some(acfg) = cfg.adapters.get(&member.adapter_id) {
                        let snap = self
                            .quota
                            .snapshot(&member.adapter_id, &acfg.quota_provider)
                            .await;
                        let _ = self
                            .events_tx
                            .send(WsEvent::QuotaUpdated { snapshot: snap });
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
                            })),
                        )?;
                        let _ = self
                            .events_tx
                            .send(WsEvent::ChatMessageCompleted { message: updated });
                    }

                    self.set_member_status(session_id, &member.handle, MemberStatus::Idle, None)
                        .await;

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
                    )
                    .await;

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
                    } else if let Some(vmsg) = visible_msg {
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
                    &session.project_id,
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
    ) {
        if matches!(
            purpose,
            TurnPurpose::MemoryCuration | TurnPurpose::PeerConsult { .. }
        ) {
            return;
        }

        let mut merged_files = touched_files.to_vec();
        let mut conflict_triggered = false;

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
                    let _ = self.events_tx.send(WsEvent::GitBranchUpdated {
                        session_id: session.id.clone(),
                        integration_branch: int_br.to_string(),
                        ahead_commits: 1,
                        diff_stat,
                    });

                    // Fast-forward idle peer worktrees if enabled
                    if self.config.read().await.git.sync_idle_worktrees_after_merge
                        && let Ok(all_members) = self.store.list_members(&session.id)
                    {
                        for peer in all_members {
                            if peer.handle != member.handle
                                && let Some(p_wt) = peer.worktree_path
                            {
                                let _ = sync_member_worktree(Path::new(&p_wt), int_br).await;
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

        // Trigger Orchestrator LLM memory curation pass (or deterministic fallback)
        if !conflict_triggered && !matches!(purpose, TurnPurpose::OrchestratorTriage) {
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

        // If Orchestrator already dispatched via MCP `orchestrator_dispatch`, do not double-dispatch from its summary text
        let rt = self.runtime(&session.id).await;
        let skip_mention_routing = if matches!(purpose, TurnPurpose::OrchestratorTriage) {
            rt.orchestrator_dispatched_turn
                .lock()
                .await
                .remove(chain_id)
        } else {
            false
        };

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

        let _ = project_root;
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
        let curation_mode = self.config.read().await.memory.curation_mode.clone();

        // If this was ConflictResolution, Orchestrator already had `memory_curate_job` in its prompt;
        // fallback deterministically only if Orchestrator didn't record a job.
        if matches!(purpose, TurnPurpose::ConflictResolution { .. }) {
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
            let pid = session.project_id.clone();
            let cid = chain_id.to_string();
            let m_clone = member.clone();
            let tid = turn_id.to_string();
            let brief = task_brief.to_string();
            let reply = reply_text.to_string();
            let files = touched_files.to_vec();

            tokio::spawn(async move {
                let res = engine
                    .execute_member_turn(
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
                    )
                    .await;
                if res.is_err() {
                    engine.record_deterministic_job(
                        &pid, &sid, &tid, &m_clone, &brief, &reply, &files,
                    );
                }
            });
            return;
        }

        self.record_deterministic_job(
            &session.project_id,
            &session.id,
            turn_id,
            member,
            task_brief,
            reply_text,
            touched_files,
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn record_deterministic_job(
        &self,
        project_id: &str,
        session_id: &str,
        turn_id: &str,
        member: &SessionMember,
        task_brief: &str,
        reply_text: &str,
        touched_files: &[String],
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
            curated_summary: truncate_str(reply_text, 400),
            touched_modules: touched_files.to_vec(),
            outcome: "completed".into(),
            curated_by: "system".into(),
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
        let quotas = self.quota.all_snapshots(&cfg).await;
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
}

struct SwitchCallbacks {
    engine: std::sync::Weak<SwitchEngine>,
    session_id: String,
    member_handle: String,
    touched_files: Mutex<HashSet<String>>,
    tool_count: Mutex<u32>,
    edit_count: Mutex<u32>,
    thought_count: Mutex<u32>,
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

        let mut tc = self.tool_count.lock().await;
        let mut ec = self.edit_count.lock().await;
        let mut thc = self.thought_count.lock().await;
        match kind {
            TrajectoryEventKind::ToolCall | TrajectoryEventKind::Terminal => *tc += 1,
            TrajectoryEventKind::Diff => *ec += 1,
            TrajectoryEventKind::Thought => *thc += 1,
            _ => {}
        }
        let _ = engine.events_tx.send(WsEvent::TrajectoryCounters {
            session_id: self.session_id.clone(),
            turn_id: turn_id.to_string(),
            tool_calls: *tc,
            file_edits: *ec,
            thoughts: *thc,
        });
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
    async fn on_text_delta(&self, turn_id: &str, delta: String) {
        if let Some(msg_id) = &self.message_id
            && let Some(engine) = self.engine.upgrade()
        {
            let _ = engine.events_tx.send(WsEvent::ChatMessageDelta {
                session_id: self.session_id.clone(),
                message_id: msg_id.clone(),
                turn_id: turn_id.to_string(),
                member_handle: self.member_handle.clone(),
                delta,
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
