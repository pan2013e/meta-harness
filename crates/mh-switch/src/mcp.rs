use std::sync::Arc;

use serde_json::{Value, json};

use mh_core::{DispatchTaskSpec, MessageKind, SenderKind, TrajectoryDetailLevel, WsEvent};
use mh_store::{NewMessageParams, RecordEpisodicJobParams};

use crate::orchestrator::truncate_str;
use crate::{SwitchEngine, SwitchError};

pub fn list_switch_tools() -> Vec<Value> {
    vec![
        json!({
            "name": "chat_list_peers",
            "description": "List all session group-chat members, roles, live states, active models, and worktree branches.",
            "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false }
        }),
        json!({
            "name": "chat_read_messages",
            "description": "Read recent messages from the session group chat.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "limit": { "type": "integer", "minimum": 1, "maximum": 200 },
                    "before_seq": { "type": "integer" }
                }
            }
        }),
        json!({
            "name": "chat_send_message",
            "description": "Post a message into the group chat and asynchronously trigger any @mentioned recipients.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "to": { "type": "array", "items": { "type": "string" } },
                    "content": { "type": "string" }
                },
                "required": ["content"]
            }
        }),
        json!({
            "name": "peer_consult",
            "description": "Synchronously consult another session member mid-turn and await their answer (protected against deadlock cycles).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "target": { "type": "string" },
                    "question": { "type": "string" },
                    "context_turn_id": { "type": "string" }
                },
                "required": ["target", "question"]
            }
        }),
        json!({
            "name": "trajectory_list",
            "description": "List recent turn trajectories for this session.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "member": { "type": "string" },
                    "limit": { "type": "integer", "minimum": 1, "maximum": 100 }
                }
            }
        }),
        json!({
            "name": "trajectory_read",
            "description": "Read a peer harness's trajectory at summary, tool_calls, diffs, or paginated full detail.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "turn_id": { "type": "string" },
                    "detail": { "type": "string", "enum": ["summary", "tool_calls", "diffs", "full"] },
                    "page": { "type": "integer", "minimum": 0 }
                },
                "required": ["turn_id"]
            }
        }),
        json!({
            "name": "memory_recall",
            "description": "Query high-level project episodic memory (who did what job, touched modules, and project facts).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": { "type": "string" },
                    "limit": { "type": "integer", "minimum": 1, "maximum": 50 }
                }
            }
        }),
        json!({
            "name": "memory_record_fact",
            "description": "Save a concise high-level architectural, ownership, or harness convention fact to project memory.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "category": { "type": "string" },
                    "summary": { "type": "string" }
                },
                "required": ["category", "summary"]
            }
        }),
        json!({
            "name": "quota_get_status",
            "description": "Retrieve normalized subscription usage and 5h/weekly headroom across all configured adapters.",
            "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false }
        }),
        json!({
            "name": "models_list_catalog",
            "description": "List discovered ACP configOptions (models, reasoning efforts) and strengths per adapter/member.",
            "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false }
        }),
        json!({
            "name": "orchestrator_dispatch",
            "description": "Configure executor model/reasoning effort and dispatch one or more tasks into executor worktrees.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "rationale": { "type": "string" },
                    "dispatches": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "target": { "type": "string" },
                                "model": { "type": "string" },
                                "effort": { "type": "string" },
                                "task_brief": { "type": "string" },
                                "mode": { "type": "string", "enum": ["parallel", "sequential"] }
                            },
                            "required": ["target", "task_brief"]
                        }
                    }
                },
                "required": ["rationale", "dispatches"]
            }
        }),
        json!({
            "name": "memory_curate_job",
            "description": "Record a curated high-level episodic memory entry summarizing a completed job.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "member_handle": { "type": "string" },
                    "turn_id": { "type": "string" },
                    "task_brief": { "type": "string" },
                    "curated_summary": { "type": "string" },
                    "touched_modules": { "type": "array", "items": { "type": "string" } },
                    "outcome": { "type": "string" },
                    "project_fact": { "type": "string" }
                },
                "required": ["member_handle", "curated_summary"]
            }
        }),
    ]
}

pub async fn execute_mcp_tool(
    engine: &Arc<SwitchEngine>,
    session_id: &str,
    caller: &str,
    tool_name: &str,
    args: Value,
) -> Result<Value, SwitchError> {
    let session = engine.store.get_session(session_id)?;

    match tool_name {
        "chat_list_peers" => {
            let members = engine.store.list_members(session_id)?;
            let available = engine.available_members(&members).await;
            let statuses = engine.get_member_statuses(session_id).await;
            let list: Vec<Value> = members
                .into_iter()
                .map(|m| {
                    let (st, turn_id) = statuses
                        .get(&m.handle)
                        .cloned()
                        .unwrap_or((mh_core::MemberStatus::Hibernated, None));
                    json!({
                        "handle": m.handle,
                        "adapter_id": m.adapter_id,
                        "display_name": m.display_name,
                        "role": m.role,
                        "available": available.contains(&m.handle),
                        "status": st,
                        "current_turn_id": turn_id,
                        "active_model": m.active_model,
                        "active_effort": m.active_effort,
                        "worktree_branch": m.worktree_branch,
                    })
                })
                .collect();
            Ok(json!({ "peers": list }))
        }
        "chat_read_messages" => {
            let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(40) as u32;
            let before_seq = args.get("before_seq").and_then(|v| v.as_i64());
            let msgs = engine.store.list_messages(session_id, before_seq, limit)?;
            Ok(json!({ "messages": msgs }))
        }
        "chat_send_message" => {
            let content = args
                .get("content")
                .and_then(|v| v.as_str())
                .ok_or_else(|| SwitchError::Validation("missing `content`".into()))?
                .to_string();
            let to: Vec<String> = args
                .get("to")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|x| x.as_str().map(|s| s.trim_start_matches('@').to_string()))
                        .collect()
                })
                .unwrap_or_default();

            let msg = engine
                .send_peer_message(session_id, caller, to, content)
                .await?;
            Ok(json!({ "message_id": msg.id, "seq": msg.seq }))
        }
        "peer_consult" => {
            let target = args
                .get("target")
                .and_then(|v| v.as_str())
                .ok_or_else(|| SwitchError::Validation("missing `target`".into()))?
                .trim_start_matches('@')
                .to_string();
            let question = args
                .get("question")
                .and_then(|v| v.as_str())
                .ok_or_else(|| SwitchError::Validation("missing `question`".into()))?
                .to_string();
            let context_turn_id = args
                .get("context_turn_id")
                .and_then(|v| v.as_str())
                .map(str::to_string);

            engine
                .run_peer_consult(session_id, caller, &target, &question, context_turn_id)
                .await
        }
        "trajectory_list" => {
            let member = args
                .get("member")
                .or_else(|| args.get("harness"))
                .and_then(|v| v.as_str())
                .map(|s| s.trim_start_matches('@'));
            let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(20) as u32;
            let list = engine.store.list_trajectories(session_id, member, limit)?;
            Ok(json!({ "trajectories": list }))
        }
        "trajectory_read" => {
            let turn_id = args
                .get("turn_id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| SwitchError::Validation("missing `turn_id`".into()))?;
            let detail_str = args
                .get("detail")
                .and_then(|v| v.as_str())
                .unwrap_or("summary");
            let page = args.get("page").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
            let detail = TrajectoryDetailLevel::parse(detail_str);

            let res = engine
                .store
                .read_trajectory_paginated(turn_id, detail, page, 16_384)?;

            // Emit subtle inline activity pill when reading a peer's trajectory
            if res.trajectory.member_handle != caller {
                let pill = engine.store.append_message(NewMessageParams {
                    id: None,
                    session_id: session_id.to_string(),
                    chain_id: engine.active_chain_for_member(session_id, caller).await,
                    parent_msg_id: None,
                    sender_kind: SenderKind::System,
                    sender_id: caller.to_string(),
                    recipients: vec![res.trajectory.member_handle.clone()],
                    kind: MessageKind::ActivityPill,
                    content: format!(
                        "@{} read @{}'s trajectory ({})",
                        caller, res.trajectory.member_handle, detail_str
                    ),
                    turn_id: Some(turn_id.to_string()),
                    metadata: json!({
                        "reader": caller,
                        "owner": res.trajectory.member_handle,
                        "detail": detail_str,
                        "page": page,
                    }),
                })?;
                let _ = engine.events_tx.send(WsEvent::SwitchActivityPill {
                    session_id: session_id.to_string(),
                    message: pill,
                });
            }

            Ok(json!({
                "trajectory": res.trajectory,
                "detail": res.detail,
                "page": res.page,
                "has_more": res.has_more,
                "events": res.events,
            }))
        }
        "memory_recall" => {
            let query = args.get("query").and_then(|v| v.as_str());
            let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(10) as u32;
            let Some(ref project_id) = session.project_id else {
                return Ok(json!({ "jobs": [], "facts": [] }));
            };
            let jobs = engine.store.recall_jobs(project_id, query, limit)?;
            let facts = engine.store.list_facts(project_id, 20)?;
            Ok(json!({ "jobs": jobs, "facts": facts }))
        }
        "memory_record_fact" => {
            let Some(ref project_id) = session.project_id else {
                return Ok(json!({ "skipped": "standalone session has no project memory" }));
            };
            let category = args
                .get("category")
                .and_then(|v| v.as_str())
                .unwrap_or("architecture");
            let summary = args
                .get("summary")
                .and_then(|v| v.as_str())
                .ok_or_else(|| SwitchError::Validation("missing `summary`".into()))?;
            let fact = engine
                .store
                .record_fact(project_id, category, summary, caller)?;
            Ok(json!({ "fact": fact }))
        }
        "quota_get_status" => {
            let cfg = engine.config.read().await.clone();
            let snapshots = engine.quota.all_snapshots(&cfg).await;
            Ok(json!({ "quotas": snapshots }))
        }
        "models_list_catalog" => {
            let cfg = engine.config.read().await.clone();
            let members = engine.store.list_members(session_id)?;
            let available = engine.available_members(&members).await;
            let mut catalog = Vec::new();
            for m in members {
                let opts = engine
                    .supervisor
                    .get_discovered_options(session_id, &m.handle)
                    .await;
                let char_info = cfg.adapters.get(&m.adapter_id).map(|a| &a.characteristics);
                catalog.push(json!({
                    "handle": m.handle,
                    "adapter_id": m.adapter_id,
                    "role": m.role,
                    "available": available.contains(&m.handle),
                    "discovered_options": opts,
                    "characteristics": char_info,
                }));
            }
            Ok(json!({ "catalog": catalog }))
        }
        "orchestrator_dispatch" => {
            let rationale = args
                .get("rationale")
                .and_then(|v| v.as_str())
                .unwrap_or("Orchestrated dispatch")
                .to_string();
            let raw_dispatches = args
                .get("dispatches")
                .ok_or_else(|| SwitchError::Validation("missing `dispatches`".into()))?;
            let dispatches: Vec<DispatchTaskSpec> = serde_json::from_value(raw_dispatches.clone())
                .map_err(|e| SwitchError::Validation(format!("invalid dispatches: {e}")))?;
            if dispatches.is_empty() {
                return Err(SwitchError::Validation(
                    "`dispatches` array must not be empty".into(),
                ));
            }
            let dispatch_id = engine
                .submit_orchestrator_dispatch(session_id, caller, rationale, dispatches)
                .await?;
            Ok(json!({ "dispatch_id": dispatch_id, "status": "scheduled" }))
        }
        "memory_curate_job" => {
            let member_handle = args
                .get("member_handle")
                .and_then(|v| v.as_str())
                .ok_or_else(|| SwitchError::Validation("missing `member_handle`".into()))?
                .trim_start_matches('@')
                .to_string();
            let curated_summary = args
                .get("curated_summary")
                .and_then(|v| v.as_str())
                .ok_or_else(|| SwitchError::Validation("missing `curated_summary`".into()))?
                .to_string();
            let turn_id = args
                .get("turn_id")
                .and_then(|v| v.as_str())
                .map(str::to_string);
            let task_brief = args
                .get("task_brief")
                .and_then(|v| v.as_str())
                .unwrap_or(&curated_summary)
                .to_string();
            let touched_modules: Vec<String> = args
                .get("touched_modules")
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default();
            let outcome = args
                .get("outcome")
                .and_then(|v| v.as_str())
                .unwrap_or("completed")
                .to_string();

            let Some(ref project_id) = session.project_id else {
                return Ok(json!({ "skipped": "standalone session has no project memory" }));
            };
            let member = engine.store.get_member(session_id, &member_handle).ok();
            let traj = turn_id
                .as_deref()
                .and_then(|tid| engine.store.get_trajectory(tid).ok());

            let job = engine.store.record_curated_job(RecordEpisodicJobParams {
                project_id: project_id.clone(),
                session_id: Some(session_id.to_string()),
                turn_id,
                member_handle: member_handle.clone(),
                adapter_id: member
                    .as_ref()
                    .map(|m| m.adapter_id.clone())
                    .unwrap_or_else(|| member_handle.clone()),
                model: traj
                    .as_ref()
                    .and_then(|t| t.model.clone())
                    .or_else(|| member.as_ref().and_then(|m| m.active_model.clone())),
                effort: traj
                    .as_ref()
                    .and_then(|t| t.effort.clone())
                    .or_else(|| member.as_ref().and_then(|m| m.active_effort.clone())),
                task_brief: truncate_str(&task_brief, 240),
                curated_summary: truncate_str(&curated_summary, 400),
                touched_modules,
                outcome,
                curated_by: caller.to_string(),
                pinned: false,
            })?;

            if let Some(fact_text) = args.get("project_fact").and_then(|v| v.as_str())
                && !fact_text.trim().is_empty()
            {
                let _ = engine
                    .store
                    .record_fact(project_id, "ownership", fact_text, caller);
            }

            let _ = engine
                .events_tx
                .send(WsEvent::MemoryJobCurated { job: job.clone() });
            Ok(json!({ "job": job }))
        }
        other => Err(SwitchError::Validation(format!(
            "unknown switch MCP tool: {other}"
        ))),
    }
}
