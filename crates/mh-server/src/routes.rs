use std::path::{Path, PathBuf};

use axum::{
    Json,
    extract::{Path as AxumPath, Query, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use mh_acp::{
    ensure_session_integration_worktree, get_session_diff, init_git_repo, inspect_repo,
    open_terminal_in_dir, pick_folder_native, probe_installed_adapters, promote_session,
    prune_session_worktrees,
};
use mh_core::{
    IsolationMode, MemberRole, PermissionPolicy, PromoteMode, expand_user_path, normalize_path,
};
use mh_store::{CreateSessionParams, PatchSessionParams, UpsertMemberParams};
use mh_switch::{SwitchError, execute_mcp_tool, list_switch_tools};

use crate::mcp_bridge::BridgeCallPayload;
use crate::{AppState, WebAssets};

pub type ApiResult<T> = Result<Json<T>, (StatusCode, Json<Value>)>;

fn err_resp(status: StatusCode, msg: impl Into<String>) -> (StatusCode, Json<Value>) {
    (status, Json(json!({ "error": msg.into() })))
}

fn map_switch_err(err: SwitchError) -> (StatusCode, Json<Value>) {
    match err {
        SwitchError::NotFound(m) => err_resp(StatusCode::NOT_FOUND, m),
        SwitchError::Validation(_) | SwitchError::CycleDetected(_) => {
            err_resp(StatusCode::BAD_REQUEST, err.to_string())
        }
        SwitchError::Store(e) => err_resp(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
        other => err_resp(StatusCode::BAD_REQUEST, other.to_string()),
    }
}

pub async fn health(State(state): State<AppState>) -> Json<Value> {
    let active_procs = state.switch.supervisor.active_process_count().await;
    Json(json!({
        "status": "ok",
        "version": env!("CARGO_PKG_VERSION"),
        "active_subprocesses": active_procs,
        "os": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
    }))
}

#[derive(Debug, Deserialize)]
pub struct FsListQuery {
    pub path: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct FsEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub is_git_repo: bool,
}

pub async fn list_fs(Query(q): Query<FsListQuery>) -> ApiResult<Value> {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    let raw = q.path.unwrap_or_default().trim().to_string();

    if raw.is_empty() {
        let mut entries: Vec<FsEntry> = Vec::new();
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();

        let mut push_root = |label: String, p: PathBuf| {
            if !p.is_dir() {
                return;
            }
            let norm = normalize_path(&p).unwrap_or(p);
            let clean = dunce::simplified(&norm).to_string_lossy().into_owned();
            let key = clean.to_lowercase();
            if seen.insert(key) {
                entries.push(FsEntry {
                    name: label,
                    path: clean,
                    is_dir: true,
                    is_git_repo: norm.join(".git").exists(),
                });
            }
        };

        let home_clean = dunce::simplified(&home).to_string_lossy().into_owned();
        push_root(format!("Home ({home_clean})"), home.clone());

        if let Some(desktop) = dirs::desktop_dir() {
            push_root("Desktop".into(), desktop);
        }
        if let Some(docs) = dirs::document_dir() {
            push_root("Documents".into(), docs);
        }
        for sub in ["repos", "projects", "code", "workspace", "dev", "src"] {
            let candidate = home.join(sub);
            if candidate.is_dir() {
                push_root(format!("~/{sub}"), candidate);
            }
        }

        #[cfg(windows)]
        {
            for letter in b'C'..=b'Z' {
                let drive = format!("{}:\\", letter as char);
                let p = Path::new(&drive);
                if p.exists() {
                    let key = drive.to_lowercase();
                    if seen.insert(key) {
                        entries.push(FsEntry {
                            name: format!("Drive ({})", &drive[..2]),
                            path: drive,
                            is_dir: true,
                            is_git_repo: false,
                        });
                    }
                }
            }
        }
        #[cfg(not(windows))]
        {
            if seen.insert("/".into()) {
                entries.push(FsEntry {
                    name: "/".into(),
                    path: "/".into(),
                    is_dir: true,
                    is_git_repo: false,
                });
            }
        }

        return Ok(Json(json!({
            "current": "",
            "parent": null,
            "entries": entries
        })));
    }

    let dir = expand_user_path(&raw);
    if !dir.is_dir() {
        return Err(err_resp(
            StatusCode::BAD_REQUEST,
            format!("not a directory: {}", dir.display()),
        ));
    }

    let norm = normalize_path(&dir).unwrap_or(dir);
    let parent = norm
        .parent()
        .map(|p| dunce::simplified(p).to_string_lossy().into_owned());

    let mut entries = Vec::new();
    let Ok(rd) = std::fs::read_dir(&norm) else {
        return Err(err_resp(StatusCode::FORBIDDEN, "cannot read directory"));
    };
    for entry in rd.flatten().take(400) {
        let p = entry.path();
        if !p.is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') && name != ".meta-harness" {
            continue;
        }
        let clean = dunce::simplified(&p).to_string_lossy().into_owned();
        entries.push(FsEntry {
            name,
            path: clean,
            is_dir: true,
            is_git_repo: p.join(".git").exists(),
        });
    }
    entries.sort_by_key(|a| a.name.to_lowercase());

    Ok(Json(json!({
        "current": dunce::simplified(&norm).to_string_lossy(),
        "parent": parent,
        "entries": entries,
    })))
}

pub async fn pick_fs_folder(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
) -> ApiResult<Value> {
    let host_is_local = headers
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .map(|h| {
            let h_lower = h.to_ascii_lowercase();
            h_lower.starts_with("127.0.0.1")
                || h_lower.starts_with("localhost")
                || h_lower.starts_with("[::1]")
        })
        .unwrap_or(true);

    if state.auth.required || !host_is_local {
        return Ok(Json(json!({
            "supported": false,
            "cancelled": false,
            "path": null,
        })));
    }

    match pick_folder_native().await {
        Ok(Some(chosen)) => {
            let norm = normalize_path(&chosen).unwrap_or(chosen);
            let clean = dunce::simplified(&norm).to_string_lossy().into_owned();
            Ok(Json(json!({
                "supported": true,
                "cancelled": false,
                "path": clean,
            })))
        }
        Ok(None) => Ok(Json(json!({
            "supported": true,
            "cancelled": true,
            "path": null,
        }))),
        Err(_) => Ok(Json(json!({
            "supported": false,
            "cancelled": false,
            "path": null,
        }))),
    }
}

pub async fn list_adapters(State(state): State<AppState>) -> Json<Value> {
    let cfg = state.switch.config.read().await.clone();
    Json(json!({ "adapters": cfg.adapters }))
}

pub async fn probe_adapters(State(state): State<AppState>) -> Json<Value> {
    let cfg = state.switch.config.read().await.clone();
    let probes = probe_installed_adapters(&cfg);
    Json(json!({ "probes": probes }))
}

pub async fn list_quotas(State(state): State<AppState>) -> Json<Value> {
    let cfg = state.switch.config.read().await.clone();
    let snapshots = state.switch.quota.all_snapshots(&cfg).await;
    Json(json!({ "quotas": snapshots }))
}

pub async fn refresh_quotas(State(state): State<AppState>) -> Json<Value> {
    let cfg = state.switch.config.read().await.clone();
    let snapshots = state.switch.quota.refresh_all(&cfg).await;
    for snap in &snapshots {
        let _ = state.switch.events_tx.send(mh_core::WsEvent::QuotaUpdated {
            snapshot: snap.clone(),
        });
    }
    Json(json!({ "quotas": snapshots }))
}

#[derive(Debug, Deserialize)]
pub struct UpdateQuotaRequest {
    pub plan_label: Option<String>,
    pub exact_probe_opt_in: Option<bool>,
    pub manual_5h_pct: Option<f32>,
    pub manual_weekly_pct: Option<f32>,
    pub clear_rate_limit: Option<bool>,
}

pub async fn update_adapter_quota(
    State(state): State<AppState>,
    AxumPath(adapter_id): AxumPath<String>,
    Json(body): Json<UpdateQuotaRequest>,
) -> ApiResult<Value> {
    let config = state.switch.config.read().await;
    let adapter = config
        .adapters
        .get(&adapter_id)
        .ok_or_else(|| err_resp(StatusCode::NOT_FOUND, "unknown adapter"))?;
    let provider = adapter.quota_provider.clone();
    let default_opt_in = adapter.exact_quota_probe_opt_in;
    drop(config);
    let prev = state
        .switch
        .store
        .get_adapter_quota_pref(&adapter_id)
        .map_err(|e| err_resp(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let plan = body
        .plan_label
        .or_else(|| prev.as_ref().and_then(|p| p.plan_label.clone()));
    let opt_in = body.exact_probe_opt_in.unwrap_or_else(|| {
        prev.as_ref()
            .map_or(default_opt_in, |p| p.exact_probe_opt_in)
    });
    let m5 = body
        .manual_5h_pct
        .or_else(|| prev.as_ref().and_then(|p| p.manual_5h_pct));
    let mw = body
        .manual_weekly_pct
        .or_else(|| prev.as_ref().and_then(|p| p.manual_weekly_pct));

    state
        .switch
        .store
        .upsert_adapter_quota_pref(&adapter_id, plan.as_deref(), opt_in, m5, mw)
        .map_err(|e| err_resp(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    state
        .switch
        .quota
        .configure_adapter(&adapter_id, plan, opt_in, m5, mw)
        .await;

    if body.clear_rate_limit.unwrap_or(false) {
        state.switch.quota.clear_rate_limit(&adapter_id).await;
    }
    let snap = state.switch.quota.snapshot(&adapter_id, &provider).await;
    let _ = state.switch.events_tx.send(mh_core::WsEvent::QuotaUpdated {
        snapshot: snap.clone(),
    });
    Ok(Json(json!({ "quota": snap })))
}

pub async fn list_projects(State(state): State<AppState>) -> ApiResult<Value> {
    let projects = state
        .switch
        .store
        .list_projects()
        .map_err(|e| err_resp(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(json!({ "projects": projects })))
}

#[derive(Debug, Deserialize)]
pub struct CreateProjectRequest {
    pub root_path: String,
    pub name: Option<String>,
}

pub async fn create_project(
    State(state): State<AppState>,
    Json(body): Json<CreateProjectRequest>,
) -> ApiResult<Value> {
    let raw = expand_user_path(&body.root_path);
    if !raw.exists() || !raw.is_dir() {
        return Err(err_resp(
            StatusCode::BAD_REQUEST,
            format!("project folder does not exist: {}", raw.display()),
        ));
    }
    let norm =
        normalize_path(&raw).map_err(|e| err_resp(StatusCode::BAD_REQUEST, e.to_string()))?;
    let clean_str = dunce::simplified(&norm).to_string_lossy().into_owned();
    let name = body
        .name
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| {
            norm.file_name()
                .and_then(|n| n.to_str())
                .filter(|n| !n.is_empty())
                .unwrap_or(&clean_str)
                .to_string()
        });
    let repo = inspect_repo(&norm).await;
    let project = state
        .switch
        .store
        .upsert_project(&name, &clean_str, repo.is_git_repo)
        .map_err(|e| err_resp(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(json!({
        "project": project,
        "repo_status": repo,
    })))
}

pub async fn git_init_project(
    State(state): State<AppState>,
    AxumPath(project_id): AxumPath<String>,
) -> ApiResult<Value> {
    let proj = state
        .switch
        .store
        .get_project(&project_id)
        .map_err(|e| err_resp(StatusCode::NOT_FOUND, e.to_string()))?;
    let status = init_git_repo(Path::new(&proj.root_path))
        .await
        .map_err(|e| err_resp(StatusCode::BAD_REQUEST, e.to_string()))?;
    let updated = state
        .switch
        .store
        .set_project_git_repo(&project_id, status.is_git_repo)
        .map_err(|e| err_resp(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(json!({
        "project": updated,
        "repo_status": status,
    })))
}

pub async fn delete_project(
    State(state): State<AppState>,
    AxumPath(project_id): AxumPath<String>,
) -> ApiResult<Value> {
    let proj = state
        .switch
        .store
        .get_project(&project_id)
        .map_err(|e| err_resp(StatusCode::NOT_FOUND, e.to_string()))?;
    if let Ok(sessions) = state.switch.store.list_sessions(&project_id) {
        for s in sessions {
            state.switch.supervisor.shutdown_session(&s.id).await;
            let _ = prune_session_worktrees(Path::new(&proj.root_path), &s.id).await;
        }
    }
    state
        .switch
        .store
        .delete_project(&project_id)
        .map_err(|e| err_resp(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(json!({ "deleted": true })))
}

pub async fn list_sessions(
    State(state): State<AppState>,
    AxumPath(project_id): AxumPath<String>,
) -> ApiResult<Value> {
    let sessions = state
        .switch
        .store
        .list_sessions(&project_id)
        .map_err(|e| err_resp(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(json!({ "sessions": sessions })))
}

#[derive(Debug, Deserialize)]
pub struct CreateMemberInput {
    pub handle: String,
    pub adapter_id: String,
    pub display_name: Option<String>,
    pub role: Option<MemberRole>,
    pub persona_prompt: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CreateSessionRequest {
    pub title: Option<String>,
    pub orchestrator_member: Option<String>,
    pub confirm_before_dispatch: Option<bool>,
    pub isolation_mode: Option<IsolationMode>,
    pub permission_policy: Option<PermissionPolicy>,
    pub members: Option<Vec<CreateMemberInput>>,
}

pub async fn create_session(
    State(state): State<AppState>,
    AxumPath(project_id): AxumPath<String>,
    Json(body): Json<CreateSessionRequest>,
) -> ApiResult<Value> {
    let proj = state
        .switch
        .store
        .get_project(&project_id)
        .map_err(|e| err_resp(StatusCode::NOT_FOUND, e.to_string()))?;
    let cfg = state.switch.config.read().await.clone();

    let sid = format!("s_{}", &uuid::Uuid::new_v4().simple().to_string()[..10]);
    let root = Path::new(&proj.root_path);
    let repo = inspect_repo(root).await;

    let want_worktree = body.isolation_mode != Some(IsolationMode::SharedCwd) && repo.is_git_repo;
    let (isolation_mode, base_branch, base_sha, int_branch, int_wt_path) = if want_worktree {
        match ensure_session_integration_worktree(root, &sid).await {
            Ok(int) => (
                IsolationMode::WorktreePerMember,
                int.base_branch,
                Some(int.base_commit_sha),
                Some(int.branch),
                Some(dunce::simplified(&int.path).to_string_lossy().into_owned()),
            ),
            Err(_) => (IsolationMode::SharedCwd, None, None, None, None),
        }
    } else {
        (IsolationMode::SharedCwd, None, None, None, None)
    };

    let member_inputs = body.members.unwrap_or_else(|| {
        cfg.adapters
            .iter()
            .filter(|(_, a)| a.enabled)
            .map(|(id, a)| CreateMemberInput {
                handle: id.clone(),
                adapter_id: id.clone(),
                display_name: Some(a.display_name.clone()),
                role: Some(MemberRole::Executor),
                persona_prompt: None,
            })
            .collect()
    });

    let orch = body.orchestrator_member.or_else(|| {
        let probes = probe_installed_adapters(&cfg);
        member_inputs
            .iter()
            .filter(|m| {
                probes
                    .iter()
                    .any(|p| p.adapter_id == m.adapter_id && p.installed)
            })
            .min_by_key(|m| m.handle != "antigravity")
            .map(|m| m.handle.clone())
    });

    let sess = state
        .switch
        .store
        .create_session(CreateSessionParams {
            id: Some(sid.clone()),
            project_id,
            title: body.title.unwrap_or_else(|| "Group Session".into()),
            base_branch,
            base_commit_sha: base_sha,
            integration_branch: int_branch,
            integration_wt_path: int_wt_path,
            isolation_mode,
            orchestrator_member: orch.clone(),
            confirm_before_dispatch: body.confirm_before_dispatch.unwrap_or(false),
            max_hops: cfg.switch.max_agent_hops_per_user_turn,
            permission_policy: body
                .permission_policy
                .unwrap_or_else(|| PermissionPolicy::parse(&cfg.switch.default_permission_policy)),
        })
        .map_err(|e| err_resp(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let mut members = Vec::new();
    for m in member_inputs {
        let role = if orch.as_deref() == Some(&m.handle) {
            MemberRole::Both
        } else {
            m.role.unwrap_or(MemberRole::Executor)
        };
        let display = m.display_name.unwrap_or_else(|| {
            cfg.adapters
                .get(&m.adapter_id)
                .map(|a| a.display_name.clone())
                .unwrap_or_else(|| m.handle.clone())
        });
        let inserted = state
            .switch
            .store
            .upsert_member(UpsertMemberParams {
                session_id: sess.id.clone(),
                handle: m.handle,
                adapter_id: m.adapter_id,
                display_name: display,
                role,
                persona_prompt: m.persona_prompt.unwrap_or_default(),
                worktree_branch: None,
                worktree_path: None,
            })
            .map_err(|e| err_resp(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        members.push(inserted);
    }

    Ok(Json(json!({ "session": sess, "members": members })))
}

pub async fn get_session(
    State(state): State<AppState>,
    AxumPath(session_id): AxumPath<String>,
) -> ApiResult<Value> {
    let sess = state
        .switch
        .store
        .get_session(&session_id)
        .map_err(|e| err_resp(StatusCode::NOT_FOUND, e.to_string()))?;
    let members = state
        .switch
        .store
        .list_members(&session_id)
        .map_err(|e| err_resp(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let rules = state
        .switch
        .store
        .list_permission_rules(&session_id)
        .map_err(|e| err_resp(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let statuses = state.switch.get_member_statuses(&session_id).await;
    let pending_perms = state.switch.list_pending_permissions(&session_id).await;
    Ok(Json(json!({
        "session": sess,
        "members": members,
        "permission_rules": rules,
        "statuses": statuses,
        "pending_permissions": pending_perms,
    })))
}

#[derive(Debug, Deserialize)]
pub struct PatchSessionRequest {
    pub title: Option<String>,
    #[serde(default, deserialize_with = "deserialize_orchestrator_patch")]
    pub orchestrator_member: Option<Option<String>>,
    pub confirm_before_dispatch: Option<bool>,
    pub max_hops: Option<u32>,
    pub permission_policy: Option<PermissionPolicy>,
}

fn deserialize_orchestrator_patch<'de, D>(value: D) -> Result<Option<Option<String>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Option::deserialize(value).map(Some)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn orchestrator_patch_can_leave_set_or_clear_the_member() {
        for (body, expected) in [
            (json!({}), None),
            (json!({"orchestrator_member": null}), Some(None)),
            (
                json!({"orchestrator_member": "codex"}),
                Some(Some("codex".into())),
            ),
        ] {
            let patch: PatchSessionRequest = serde_json::from_value(body).unwrap();
            assert_eq!(patch.orchestrator_member, expected);
        }
    }
}

pub async fn patch_session(
    State(state): State<AppState>,
    AxumPath(session_id): AxumPath<String>,
    Json(body): Json<PatchSessionRequest>,
) -> ApiResult<Value> {
    if let Some(Some(handle)) = body.orchestrator_member.as_ref() {
        state
            .switch
            .store
            .get_member(&session_id, handle)
            .map_err(|e| err_resp(StatusCode::BAD_REQUEST, e.to_string()))?;
    }
    let sess = state
        .switch
        .store
        .patch_session(
            &session_id,
            PatchSessionParams {
                title: body.title,
                orchestrator_member: body.orchestrator_member,
                confirm_before_dispatch: body.confirm_before_dispatch,
                max_hops: body.max_hops,
                permission_policy: body.permission_policy,
                working_summary: None,
            },
        )
        .map_err(|e| err_resp(StatusCode::BAD_REQUEST, e.to_string()))?;
    Ok(Json(json!({ "session": sess })))
}

pub async fn delete_session(
    State(state): State<AppState>,
    AxumPath(session_id): AxumPath<String>,
) -> ApiResult<Value> {
    let sess = state
        .switch
        .store
        .get_session(&session_id)
        .map_err(|e| err_resp(StatusCode::NOT_FOUND, e.to_string()))?;
    let proj = state
        .switch
        .store
        .get_project(&sess.project_id)
        .map_err(|e| err_resp(StatusCode::NOT_FOUND, e.to_string()))?;

    state.switch.cancel_all_in_session(&session_id).await;
    state.switch.supervisor.shutdown_session(&session_id).await;
    let _ = prune_session_worktrees(Path::new(&proj.root_path), &session_id).await;
    state
        .switch
        .store
        .delete_session(&session_id)
        .map_err(|e| err_resp(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(json!({ "deleted": true })))
}

pub async fn upsert_session_member(
    State(state): State<AppState>,
    AxumPath(session_id): AxumPath<String>,
    Json(body): Json<CreateMemberInput>,
) -> ApiResult<Value> {
    let handle = body.handle.trim_start_matches('@').to_string();
    if handle.is_empty()
        || !handle
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(err_resp(
            StatusCode::BAD_REQUEST,
            "handle must contain only alphanumeric, '-', or '_' characters",
        ));
    }
    let member = state
        .switch
        .store
        .upsert_member(UpsertMemberParams {
            session_id,
            handle: handle.clone(),
            adapter_id: body.adapter_id,
            display_name: body.display_name.unwrap_or(handle),
            role: body.role.unwrap_or(MemberRole::Executor),
            persona_prompt: body.persona_prompt.unwrap_or_default(),
            worktree_branch: None,
            worktree_path: None,
        })
        .map_err(|e| err_resp(StatusCode::BAD_REQUEST, e.to_string()))?;
    Ok(Json(json!({ "member": member })))
}

pub async fn delete_session_member(
    State(state): State<AppState>,
    AxumPath((session_id, handle)): AxumPath<(String, String)>,
) -> ApiResult<Value> {
    state
        .switch
        .store
        .delete_member(&session_id, &handle)
        .map_err(|e| err_resp(StatusCode::NOT_FOUND, e.to_string()))?;
    Ok(Json(json!({ "deleted": true })))
}

#[derive(Debug, Deserialize)]
pub struct MessagesQuery {
    pub before_seq: Option<i64>,
    pub limit: Option<u32>,
}

pub async fn list_messages(
    State(state): State<AppState>,
    AxumPath(session_id): AxumPath<String>,
    Query(q): Query<MessagesQuery>,
) -> ApiResult<Value> {
    let msgs = state
        .switch
        .store
        .list_messages(&session_id, q.before_seq, q.limit.unwrap_or(150))
        .map_err(|e| err_resp(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(json!({ "messages": msgs })))
}

#[derive(Debug, Deserialize)]
pub struct PostMessageRequest {
    pub content: String,
}

pub async fn post_message(
    State(state): State<AppState>,
    AxumPath(session_id): AxumPath<String>,
    Json(body): Json<PostMessageRequest>,
) -> ApiResult<Value> {
    let msg = state
        .switch
        .handle_user_message(&session_id, body.content)
        .await
        .map_err(map_switch_err)?;
    Ok(Json(json!({ "message": msg })))
}

pub async fn cancel_session(
    State(state): State<AppState>,
    AxumPath(session_id): AxumPath<String>,
) -> Json<Value> {
    state.switch.cancel_all_in_session(&session_id).await;
    Json(json!({ "cancelled": true }))
}

#[derive(Debug, Deserialize)]
pub struct ContinueChainRequest {
    pub extra_hops: Option<u32>,
}

pub async fn continue_chain(
    State(state): State<AppState>,
    AxumPath((session_id, chain_id)): AxumPath<(String, String)>,
    Json(body): Json<ContinueChainRequest>,
) -> ApiResult<Value> {
    state
        .switch
        .continue_chain(&session_id, &chain_id, body.extra_hops.unwrap_or(6))
        .await
        .map_err(map_switch_err)?;
    Ok(Json(json!({ "continued": true })))
}

pub async fn confirm_dispatch(
    State(state): State<AppState>,
    AxumPath((session_id, dispatch_id)): AxumPath<(String, String)>,
) -> ApiResult<Value> {
    state
        .switch
        .confirm_dispatch(&session_id, &dispatch_id)
        .await
        .map_err(map_switch_err)?;
    Ok(Json(json!({ "dispatched": true })))
}

pub async fn cancel_dispatch(
    State(state): State<AppState>,
    AxumPath((session_id, dispatch_id)): AxumPath<(String, String)>,
) -> ApiResult<Value> {
    state
        .switch
        .cancel_dispatch(&session_id, &dispatch_id)
        .await
        .map_err(map_switch_err)?;
    Ok(Json(json!({ "cancelled": true })))
}

#[derive(Debug, Deserialize)]
pub struct ResolvePermissionRequest {
    pub option_id: String,
    pub allow_pattern: Option<String>,
}

pub async fn resolve_permission(
    State(state): State<AppState>,
    AxumPath((session_id, req_id)): AxumPath<(String, String)>,
    Json(body): Json<ResolvePermissionRequest>,
) -> ApiResult<Value> {
    state
        .switch
        .resolve_permission(&session_id, &req_id, body.option_id, body.allow_pattern)
        .await
        .map_err(map_switch_err)?;
    Ok(Json(json!({ "resolved": true })))
}

pub async fn delete_permission_rule(
    State(state): State<AppState>,
    AxumPath((_session_id, rule_id)): AxumPath<(String, String)>,
) -> ApiResult<Value> {
    state
        .switch
        .store
        .delete_permission_rule(&rule_id)
        .map_err(|e| err_resp(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(json!({ "deleted": true })))
}

pub async fn get_session_diff_handler(
    State(state): State<AppState>,
    AxumPath(session_id): AxumPath<String>,
) -> ApiResult<Value> {
    let sess = state
        .switch
        .store
        .get_session(&session_id)
        .map_err(|e| err_resp(StatusCode::NOT_FOUND, e.to_string()))?;
    let (Some(wt), Some(br)) = (
        sess.integration_wt_path.as_deref(),
        sess.integration_branch.as_deref(),
    ) else {
        return Ok(Json(json!({
            "diff": null,
            "isolation_mode": sess.isolation_mode
        })));
    };
    let base = sess
        .base_commit_sha
        .as_deref()
        .or(sess.base_branch.as_deref())
        .unwrap_or("HEAD");
    let report = get_session_diff(Path::new(wt), base, br)
        .await
        .map_err(|e| err_resp(StatusCode::BAD_REQUEST, e.to_string()))?;
    Ok(Json(json!({ "diff": report })))
}

#[derive(Debug, Deserialize)]
pub struct RunCommandRequest {
    pub command: String,
}

pub async fn run_session_preview_command(
    State(state): State<AppState>,
    AxumPath(session_id): AxumPath<String>,
    Json(body): Json<RunCommandRequest>,
) -> ApiResult<Value> {
    let sess = state
        .switch
        .store
        .get_session(&session_id)
        .map_err(|e| err_resp(StatusCode::NOT_FOUND, e.to_string()))?;
    let proj = state
        .switch
        .store
        .get_project(&sess.project_id)
        .map_err(|e| err_resp(StatusCode::NOT_FOUND, e.to_string()))?;
    let cwd = sess.integration_wt_path.unwrap_or(proj.root_path);

    let (prog, args) = mh_acp::platform::shell_command_invocation(&body.command);
    let out = tokio::process::Command::new(prog)
        .args(args)
        .current_dir(&cwd)
        .output()
        .await
        .map_err(|e| err_resp(StatusCode::BAD_REQUEST, e.to_string()))?;

    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    Ok(Json(json!({
        "cwd": cwd,
        "exit_code": out.status.code().unwrap_or(-1),
        "stdout": stdout,
        "stderr": stderr,
    })))
}

pub async fn open_session_terminal(
    State(state): State<AppState>,
    AxumPath(session_id): AxumPath<String>,
) -> ApiResult<Value> {
    let sess = state
        .switch
        .store
        .get_session(&session_id)
        .map_err(|e| err_resp(StatusCode::NOT_FOUND, e.to_string()))?;
    let proj = state
        .switch
        .store
        .get_project(&sess.project_id)
        .map_err(|e| err_resp(StatusCode::NOT_FOUND, e.to_string()))?;
    let cwd = sess.integration_wt_path.unwrap_or(proj.root_path);
    open_terminal_in_dir(Path::new(&cwd))
        .map_err(|e| err_resp(StatusCode::BAD_REQUEST, e.to_string()))?;
    Ok(Json(json!({ "opened": true, "cwd": cwd })))
}

#[derive(Debug, Deserialize)]
pub struct PromoteRequest {
    pub mode: PromoteMode,
}

pub async fn promote_session_handler(
    State(state): State<AppState>,
    AxumPath(session_id): AxumPath<String>,
    Json(body): Json<PromoteRequest>,
) -> ApiResult<Value> {
    let sess = state
        .switch
        .store
        .get_session(&session_id)
        .map_err(|e| err_resp(StatusCode::NOT_FOUND, e.to_string()))?;
    let proj = state
        .switch
        .store
        .get_project(&sess.project_id)
        .map_err(|e| err_resp(StatusCode::NOT_FOUND, e.to_string()))?;
    let int_br = sess
        .integration_branch
        .as_deref()
        .ok_or_else(|| err_resp(StatusCode::BAD_REQUEST, "session has no integration branch"))?;
    let base_br = sess.base_branch.as_deref().unwrap_or("main");
    let base_sha = sess.base_commit_sha.as_deref().unwrap_or(base_br);

    let res = promote_session(
        Path::new(&proj.root_path),
        base_br,
        base_sha,
        int_br,
        body.mode,
    )
    .await
    .map_err(|e| err_resp(StatusCode::BAD_REQUEST, e.to_string()))?;

    Ok(Json(json!({ "result": res })))
}

#[derive(Debug, Deserialize)]
pub struct TrajectoryListQuery {
    pub session_id: String,
    pub member: Option<String>,
    pub limit: Option<u32>,
}

pub async fn list_trajectories(
    State(state): State<AppState>,
    Query(q): Query<TrajectoryListQuery>,
) -> ApiResult<Value> {
    let list = state
        .switch
        .store
        .list_trajectories(&q.session_id, q.member.as_deref(), q.limit.unwrap_or(40))
        .map_err(|e| err_resp(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(json!({ "trajectories": list })))
}

pub async fn get_trajectory(
    State(state): State<AppState>,
    AxumPath(turn_id): AxumPath<String>,
) -> ApiResult<Value> {
    let traj = state
        .switch
        .store
        .get_trajectory(&turn_id)
        .map_err(|e| err_resp(StatusCode::NOT_FOUND, e.to_string()))?;
    let events = state
        .switch
        .store
        .list_trajectory_events(&turn_id)
        .map_err(|e| err_resp(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(json!({
        "trajectory": traj,
        "events": events,
    })))
}

#[derive(Debug, Deserialize)]
pub struct MemoryQuery {
    pub q: Option<String>,
    pub limit: Option<u32>,
}

pub async fn get_project_memory(
    State(state): State<AppState>,
    AxumPath(project_id): AxumPath<String>,
    Query(q): Query<MemoryQuery>,
) -> ApiResult<Value> {
    let jobs = state
        .switch
        .store
        .recall_jobs(&project_id, q.q.as_deref(), q.limit.unwrap_or(30))
        .map_err(|e| err_resp(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let facts = state
        .switch
        .store
        .list_facts(&project_id, 50)
        .map_err(|e| err_resp(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(json!({ "jobs": jobs, "facts": facts })))
}

#[derive(Debug, Deserialize)]
pub struct CreateFactRequest {
    pub category: Option<String>,
    pub summary: String,
}

pub async fn create_project_fact(
    State(state): State<AppState>,
    AxumPath(project_id): AxumPath<String>,
    Json(body): Json<CreateFactRequest>,
) -> ApiResult<Value> {
    let fact = state
        .switch
        .store
        .record_fact(
            &project_id,
            body.category.as_deref().unwrap_or("architecture"),
            &body.summary,
            "user",
        )
        .map_err(|e| err_resp(StatusCode::BAD_REQUEST, e.to_string()))?;
    Ok(Json(json!({ "fact": fact })))
}

pub async fn delete_project_fact(
    State(state): State<AppState>,
    AxumPath((_project_id, fact_id)): AxumPath<(String, String)>,
) -> ApiResult<Value> {
    state
        .switch
        .store
        .delete_fact(&fact_id)
        .map_err(|e| err_resp(StatusCode::NOT_FOUND, e.to_string()))?;
    Ok(Json(json!({ "deleted": true })))
}

#[derive(Debug, Deserialize)]
pub struct PatchJobRequest {
    pub pinned: bool,
}

pub async fn patch_memory_job(
    State(state): State<AppState>,
    AxumPath(job_id): AxumPath<String>,
    Json(body): Json<PatchJobRequest>,
) -> ApiResult<Value> {
    state
        .switch
        .store
        .set_job_pinned(&job_id, body.pinned)
        .map_err(|e| err_resp(StatusCode::NOT_FOUND, e.to_string()))?;
    Ok(Json(json!({ "updated": true })))
}

pub async fn delete_memory_job(
    State(state): State<AppState>,
    AxumPath(job_id): AxumPath<String>,
) -> ApiResult<Value> {
    state
        .switch
        .store
        .delete_job(&job_id)
        .map_err(|e| err_resp(StatusCode::NOT_FOUND, e.to_string()))?;
    Ok(Json(json!({ "deleted": true })))
}

pub async fn internal_mcp_handler(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Json(payload): Json<BridgeCallPayload>,
) -> Response {
    let expected = format!("Bearer {}", state.switch.bridge_token);
    let supplied = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if supplied != expected {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "error": "invalid bridge token" })),
        )
            .into_response();
    }

    if payload.action == "list" {
        return Json(json!({ "tools": list_switch_tools() })).into_response();
    }

    let tool = payload.tool_name.as_deref().unwrap_or_default();
    match execute_mcp_tool(
        &state.switch,
        &payload.session_id,
        &payload.caller,
        tool,
        payload.arguments,
    )
    .await
    {
        Ok(res) => Json(res).into_response(),
        Err(e) => Json(json!({ "isError": true, "error": e.to_string() })).into_response(),
    }
}

pub async fn serve_embedded_spa(uri: axum::http::Uri) -> Response {
    let raw_path = uri.path().trim_start_matches('/');
    let path = if raw_path.is_empty() {
        "index.html"
    } else {
        raw_path
    };

    if let Some(file) = WebAssets::get(path) {
        let mime = guess_mime(path);
        return (
            StatusCode::OK,
            [(header::CONTENT_TYPE, mime)],
            file.data.into_owned(),
        )
            .into_response();
    }

    if let Some(index) = WebAssets::get("index.html") {
        return (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
            index.data.into_owned(),
        )
            .into_response();
    }

    (
        StatusCode::NOT_FOUND,
        "Meta-Harness frontend bundle (web/dist) not found.",
    )
        .into_response()
}

fn guess_mime(path: &str) -> &'static str {
    if path.ends_with(".html") {
        "text/html; charset=utf-8"
    } else if path.ends_with(".js") || path.ends_with(".mjs") {
        "application/javascript; charset=utf-8"
    } else if path.ends_with(".css") {
        "text/css; charset=utf-8"
    } else if path.ends_with(".svg") {
        "image/svg+xml"
    } else if path.ends_with(".png") {
        "image/png"
    } else if path.ends_with(".json") {
        "application/json"
    } else {
        "application/octet-stream"
    }
}
