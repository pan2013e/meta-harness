use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use axum::{
    Json,
    extract::{Path as AxumPath, Query, State},
    http::{Method, StatusCode, header},
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use mh_acp::{
    checkpoint_session_worktrees, commit_worktree_if_dirty, ensure_session_integration_worktree,
    get_session_diff, init_git_repo, inspect_repo, probe_installed_adapters, promote_session,
    prune_session_worktree_checkouts, prune_session_worktrees, reset_worktree_to_sha,
    sync_member_worktree,
};
use mh_core::{
    DiffStat, IsolationMode, MemberRole, MemberStatus, PermissionPolicy, PromoteMode, Session,
    StorageSummary, TurnModelSelection, WsEvent, expand_user_path, is_path_within, normalize_path,
};
use mh_store::{CreateSessionParams, PatchSessionParams, UpsertMemberParams};
use mh_switch::{SwitchError, execute_mcp_tool, list_switch_tools};

use crate::mcp_bridge::BridgeCallPayload;
use crate::{AppState, WebAssets};

pub type ApiResult<T> = Result<Json<T>, (StatusCode, Json<Value>)>;

pub(crate) fn err_resp(status: StatusCode, msg: impl Into<String>) -> (StatusCode, Json<Value>) {
    (status, Json(json!({ "error": msg.into() })))
}

fn map_switch_err(err: SwitchError) -> (StatusCode, Json<Value>) {
    match err {
        SwitchError::NotFound(m) => err_resp(StatusCode::NOT_FOUND, m),
        SwitchError::Validation(_) | SwitchError::CycleDetected(_) => {
            err_resp(StatusCode::BAD_REQUEST, err.to_string())
        }
        SwitchError::Store(e) => err_resp(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
        SwitchError::Io(e) => err_resp(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
        other => err_resp(StatusCode::BAD_REQUEST, other.to_string()),
    }
}

pub async fn health(State(state): State<AppState>) -> Json<Value> {
    let active_procs = state.switch.supervisor.active_process_count().await;
    Json(json!({
        "status": "ok",
        "version": env!("CARGO_PKG_VERSION"),
        "commit": option_env!("MH_BUILD_COMMIT"),
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

pub async fn pick_fs_folder() -> ApiResult<Value> {
    // OS native file picker is disabled for platform consistency; clients use the built-in web browser.
    Ok(Json(json!({
        "supported": false,
        "cancelled": false,
        "path": null,
    })))
}

#[derive(Debug, Deserialize)]
pub struct AdapterListQuery {
    #[serde(default)]
    refresh: bool,
    #[serde(default)]
    summary: bool,
}

pub async fn list_adapters(
    State(state): State<AppState>,
    Query(query): Query<AdapterListQuery>,
) -> ApiResult<Value> {
    let cfg = state.switch.config.read().await.clone();
    let prefs = state
        .switch
        .store
        .list_adapter_model_prefs()
        .map_err(|e| err_resp(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let mut catalogs = std::collections::BTreeMap::new();
    if query.summary {
        for id in cfg.adapters.keys() {
            catalogs.insert(id.clone(), mh_acp::supervisor::builtin_adapter_catalog(id));
        }
    } else {
        let mut probes = tokio::task::JoinSet::new();
        for (id, adapter) in cfg.adapters.clone() {
            let supervisor = std::sync::Arc::clone(&state.switch.supervisor);
            probes.spawn(async move {
                let catalog = supervisor
                    .resolve_adapter_catalog(&id, &adapter, query.refresh)
                    .await;
                (id, catalog)
            });
        }
        while let Some(result) = probes.join_next().await {
            if let Ok((id, catalog)) = result {
                catalogs.insert(id, catalog);
            }
        }
    }
    let mut defaults = std::collections::BTreeMap::new();
    for p in prefs {
        defaults.insert(p.adapter_id.clone(), p);
    }
    let personas = state
        .switch
        .store
        .list_global_personas()
        .map_err(|e| err_resp(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let saved_profiles = cfg
        .adapters
        .iter()
        .filter_map(|(id, adapter)| {
            crate::harness_jobs::has_saved_profile(
                &state.switch.app_paths.data_dir,
                id,
                adapter.profile_dir.as_deref(),
            )
            .then_some(id)
        })
        .collect::<Vec<_>>();
    Ok(Json(json!({
        "adapters": cfg.adapters,
        "catalogs": catalogs,
        "defaults": defaults,
        "global_personas": personas,
        "saved_profiles": saved_profiles,
    })))
}

#[derive(Debug, Deserialize)]
pub struct UpdateAdapterDefaultsRequest {
    #[serde(default, deserialize_with = "deserialize_orchestrator_patch")]
    pub default_model: Option<Option<String>>,
    #[serde(default, deserialize_with = "deserialize_orchestrator_patch")]
    pub default_effort: Option<Option<String>>,
}

pub async fn update_adapter_defaults(
    State(state): State<AppState>,
    AxumPath(adapter_id): AxumPath<String>,
    Json(body): Json<UpdateAdapterDefaultsRequest>,
) -> ApiResult<Value> {
    let exists = state
        .switch
        .config
        .read()
        .await
        .adapters
        .contains_key(&adapter_id);
    if !exists {
        return Err(err_resp(StatusCode::NOT_FOUND, "unknown adapter"));
    }
    let prev = state
        .switch
        .store
        .get_adapter_model_pref(&adapter_id)
        .map_err(|e| err_resp(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let model = match body.default_model {
        Some(v) => v,
        None => prev.as_ref().and_then(|p| p.default_model.clone()),
    };
    let effort = match body.default_effort {
        Some(v) => v,
        None => prev.as_ref().and_then(|p| p.default_effort.clone()),
    };
    let pref = state
        .switch
        .store
        .upsert_adapter_model_pref(&adapter_id, model.as_deref(), effort.as_deref())
        .map_err(|e| err_resp(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(json!({ "pref": pref })))
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

pub async fn shared_antigravity_usage(
    State(state): State<AppState>,
    AxumPath(adapter_id): AxumPath<String>,
) -> ApiResult<Value> {
    let adapter = state
        .switch
        .config
        .read()
        .await
        .adapters
        .get(&adapter_id)
        .cloned()
        .ok_or_else(|| err_resp(StatusCode::NOT_FOUND, "Unknown adapter"))?;
    let _activity = state
        .switch
        .supervisor
        .adapter_activity(&adapter_id)
        .await
        .try_read_owned()
        .map_err(|_| {
            err_resp(
                StatusCode::CONFLICT,
                "This harness is undergoing maintenance.",
            )
        })?;
    let windows = state
        .switch
        .quota
        .shared_antigravity_usage(&adapter_id, &adapter)
        .await
        .map_err(|error| err_resp(StatusCode::BAD_REQUEST, error.to_string()))?;
    Ok(Json(json!({ "windows": windows })))
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
    let config = state.switch.config.write().await;
    let adapter = config
        .adapters
        .get(&adapter_id)
        .ok_or_else(|| err_resp(StatusCode::NOT_FOUND, "unknown adapter"))?;
    let provider = adapter.quota_provider.clone();
    let default_opt_in = adapter.exact_quota_probe_opt_in;
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
    drop(config);
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
    if let Ok(sessions) = state.switch.store.list_sessions(Some(&project_id), true) {
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

#[derive(Debug, Default, Deserialize)]
pub struct ListSessionsQuery {
    #[serde(default)]
    pub include_archived: bool,
}

pub async fn list_sessions(
    State(state): State<AppState>,
    AxumPath(project_id): AxumPath<String>,
    Query(q): Query<ListSessionsQuery>,
) -> ApiResult<Value> {
    let sessions = state
        .switch
        .store
        .list_sessions(Some(&project_id), q.include_archived)
        .map_err(|e| err_resp(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(json!({ "sessions": sessions })))
}

pub async fn list_standalone_sessions(
    State(state): State<AppState>,
    Query(q): Query<ListSessionsQuery>,
) -> ApiResult<Value> {
    let sessions = state
        .switch
        .store
        .list_sessions(None, q.include_archived)
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
    pub active_model: Option<String>,
    pub active_effort: Option<String>,
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
    create_session_internal(state, Some(project_id), body).await
}

pub async fn create_standalone_session(
    State(state): State<AppState>,
    Json(body): Json<CreateSessionRequest>,
) -> ApiResult<Value> {
    create_session_internal(state, None, body).await
}

async fn create_session_internal(
    state: AppState,
    project_id: Option<String>,
    body: CreateSessionRequest,
) -> ApiResult<Value> {
    let cfg = state.switch.config.read().await.clone();
    let sid = format!("s_{}", &uuid::Uuid::new_v4().simple().to_string()[..10]);
    let root_path = if let Some(ref pid) = project_id {
        let proj = state
            .switch
            .store
            .get_project(pid)
            .map_err(|e| err_resp(StatusCode::NOT_FOUND, e.to_string()))?;
        PathBuf::from(proj.root_path)
    } else {
        let ws_root = state.switch.app_paths.standalone_session_root(&sid);
        tokio::fs::create_dir_all(&ws_root)
            .await
            .map_err(|e| err_resp(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        let _ = init_git_repo(&ws_root).await;
        ws_root
    };
    let repo = inspect_repo(&root_path).await;

    let want_worktree = body.isolation_mode != Some(IsolationMode::SharedCwd) && repo.is_git_repo;
    let (isolation_mode, base_branch, base_sha, int_branch, int_wt_path) = if want_worktree {
        match ensure_session_integration_worktree(&root_path, &sid).await {
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

    let probes = probe_installed_adapters(&cfg);
    let member_inputs = if let Some(members) = body.members {
        members
    } else {
        let mut members = cfg
            .adapters
            .iter()
            .filter(|(id, a)| {
                a.enabled && probes.iter().any(|p| p.adapter_id == **id && p.installed)
            })
            .map(|(id, a)| CreateMemberInput {
                handle: id.clone(),
                adapter_id: id.clone(),
                display_name: Some(a.display_name.clone()),
                role: Some(MemberRole::Executor),
                persona_prompt: None,
                active_model: None,
                active_effort: None,
            })
            .collect::<Vec<_>>();
        let personas = state
            .switch
            .store
            .list_global_personas()
            .map_err(|e| err_resp(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        for persona in personas {
            if probes
                .iter()
                .any(|p| p.adapter_id == persona.adapter_id && p.installed)
                && !members
                    .iter()
                    .any(|m| m.handle.eq_ignore_ascii_case(&persona.handle))
            {
                members.push(CreateMemberInput {
                    handle: persona.handle,
                    adapter_id: persona.adapter_id,
                    display_name: Some(persona.display_name),
                    role: Some(MemberRole::Executor),
                    persona_prompt: Some(persona.persona_prompt),
                    active_model: persona.active_model,
                    active_effort: persona.active_effort,
                });
            }
        }
        members
    };

    let orch = body.orchestrator_member.or_else(|| {
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

    let clean_title = body
        .title
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| "New session".into());

    let sess = state
        .switch
        .store
        .create_session(CreateSessionParams {
            id: Some(sid.clone()),
            project_id,
            title: clean_title,
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

    let model_prefs = state
        .switch
        .store
        .list_adapter_model_prefs()
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
        let adapter_id = m.adapter_id.clone();
        let inserted = state
            .switch
            .store
            .upsert_member(UpsertMemberParams {
                session_id: sess.id.clone(),
                handle: m.handle,
                adapter_id: adapter_id.clone(),
                display_name: display,
                role,
                persona_prompt: m.persona_prompt.unwrap_or_default(),
                worktree_branch: None,
                worktree_path: None,
            })
            .map_err(|e| err_resp(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        let pref = model_prefs.iter().find(|p| p.adapter_id == adapter_id);
        let model = m
            .active_model
            .as_deref()
            .or_else(|| pref.and_then(|p| p.default_model.as_deref()));
        let effort = m
            .active_effort
            .as_deref()
            .or_else(|| pref.and_then(|p| p.default_effort.as_deref()));
        let final_member = if model.is_some() || effort.is_some() {
            state
                .switch
                .store
                .update_member_model_effort(&sess.id, &inserted.handle, Some(model), Some(effort))
                .map_err(|e| err_resp(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        } else {
            inserted
        };
        members.push(final_member);
    }

    Ok(Json(json!({ "session": sess, "members": members })))
}

pub async fn get_session(
    State(state): State<AppState>,
    AxumPath(session_id): AxumPath<String>,
) -> ApiResult<Value> {
    let mut sess = state
        .switch
        .store
        .get_session(&session_id)
        .map_err(|e| err_resp(StatusCode::NOT_FOUND, e.to_string()))?;
    let cfg = state.switch.config.read().await.clone();
    let probes = probe_installed_adapters(&cfg);
    let mut all_members = state
        .switch
        .store
        .list_members(&session_id)
        .map_err(|e| err_resp(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    // Auto-provision default member for any newly installed adapter missing in older sessions
    for (id, a) in &cfg.adapters {
        let is_installed = a.enabled && probes.iter().any(|p| p.adapter_id == *id && p.installed);
        if is_installed
            && !all_members.iter().any(|m| m.handle == *id)
            && let Ok(created) = state.switch.store.upsert_member(UpsertMemberParams {
                session_id: session_id.clone(),
                handle: id.clone(),
                adapter_id: id.clone(),
                display_name: a.display_name.clone(),
                role: MemberRole::Executor,
                persona_prompt: String::new(),
                worktree_branch: None,
                worktree_path: None,
            })
        {
            all_members.push(created);
        }
    }

    let members: Vec<_> = all_members
        .into_iter()
        .filter(|m| {
            probes
                .iter()
                .any(|p| p.adapter_id == m.adapter_id && p.installed)
        })
        .collect();

    if let Some(ref orch) = sess.orchestrator_member
        && !members.iter().any(|m| m.handle == *orch)
    {
        let fallback_orch = members
            .iter()
            .min_by_key(|m| m.handle != "antigravity")
            .map(|m| m.handle.clone());
        if let Ok(updated) = state.switch.store.patch_session(
            &session_id,
            PatchSessionParams {
                orchestrator_member: Some(fallback_orch),
                ..Default::default()
            },
        ) {
            sess = updated;
        }
    }

    let rules = state
        .switch
        .store
        .list_permission_rules(&session_id)
        .map_err(|e| err_resp(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let statuses = state.switch.get_member_statuses(&session_id).await;
    let pending_perms = state.switch.list_pending_permissions(&session_id).await;
    let mut discovered_options = std::collections::BTreeMap::new();
    for m in &members {
        let opts = state
            .switch
            .supervisor
            .get_discovered_options(&session_id, &m.handle)
            .await;
        if !opts.is_empty() {
            discovered_options.insert(m.handle.clone(), opts);
        }
    }
    let context = state.switch.compute_session_context(&session_id).await.ok();
    Ok(Json(json!({
        "session": sess,
        "members": members,
        "permission_rules": rules,
        "statuses": statuses,
        "pending_permissions": pending_perms,
        "discovered_options": discovered_options,
        "context": context,
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
    pub archived: Option<bool>,
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
    let runtime = state.switch.runtime(&session_id).await;
    let _lifecycle = runtime.lifecycle_lock.lock().await;
    if let Some(Some(handle)) = body.orchestrator_member.as_ref() {
        state
            .switch
            .store
            .get_member(&session_id, handle)
            .map_err(|e| err_resp(StatusCode::BAD_REQUEST, e.to_string()))?;
    }
    let existing = state
        .switch
        .store
        .get_session(&session_id)
        .map_err(|e| err_resp(StatusCode::NOT_FOUND, e.to_string()))?;

    let mut archived_at_patch: Option<Option<i64>> = None;
    if let Some(archive) = body.archived {
        if archive && existing.archived_at.is_none() {
            let repo_root = state
                .switch
                .resolve_session_repo_root(&existing)
                .map_err(map_switch_err)?;
            state.switch.cancel_all_in_session(&session_id).await;
            state.switch.supervisor.shutdown_session(&session_id).await;
            let saved = async {
                checkpoint_session_worktrees(&repo_root, &session_id, &existing.title).await?;
                prune_session_worktree_checkouts(&repo_root, &session_id).await
            }
            .await;
            if let Err(error) = saved {
                state.switch.supervisor.reopen_session(&session_id).await;
                return Err(err_resp(
                    StatusCode::CONFLICT,
                    format!("Could not archive the session: {error}"),
                ));
            }
            let _ = state.switch.store.clear_session_acp_ids(&session_id);
            let now_ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as i64;
            archived_at_patch = Some(Some(now_ms));
        } else if !archive && existing.archived_at.is_some() {
            if existing.isolation_mode == IsolationMode::WorktreePerMember {
                let repo_root = state
                    .switch
                    .resolve_session_repo_root(&existing)
                    .map_err(map_switch_err)?;
                let int = ensure_session_integration_worktree(&repo_root, &session_id)
                    .await
                    .map_err(|error| err_resp(StatusCode::CONFLICT, error.to_string()))?;
                let int_wt = dunce::simplified(&int.path).to_string_lossy().into_owned();
                state
                    .switch
                    .store
                    .update_session_git_state(
                        &session_id,
                        existing
                            .base_branch
                            .as_deref()
                            .or(int.base_branch.as_deref()),
                        existing
                            .base_commit_sha
                            .as_deref()
                            .or(Some(int.base_commit_sha.as_str())),
                        Some(&int.branch),
                        Some(&int_wt),
                        existing.isolation_mode,
                    )
                    .map_err(|error| {
                        err_resp(StatusCode::INTERNAL_SERVER_ERROR, error.to_string())
                    })?;
            }
            archived_at_patch = Some(None);
        }
    }

    let patched = state.switch.store.patch_session(
        &session_id,
        PatchSessionParams {
            title: body.title,
            orchestrator_member: body.orchestrator_member,
            confirm_before_dispatch: body.confirm_before_dispatch,
            max_hops: body.max_hops,
            permission_policy: body.permission_policy,
            working_summary: None,
            archived_at: archived_at_patch,
        },
    );
    if matches!(archived_at_patch, Some(None)) && patched.is_ok()
        || matches!(archived_at_patch, Some(Some(_))) && patched.is_err()
    {
        state.switch.supervisor.reopen_session(&session_id).await;
    }
    let sess = patched.map_err(|error| err_resp(StatusCode::BAD_REQUEST, error.to_string()))?;
    let _ = state.switch.events_tx.send(WsEvent::SessionUpdated {
        session: sess.clone(),
    });
    Ok(Json(json!({ "session": sess })))
}

#[derive(Debug, Deserialize)]
pub struct PatchMemberConfigRequest {
    #[serde(default, deserialize_with = "deserialize_orchestrator_patch")]
    pub active_model: Option<Option<String>>,
    #[serde(default, deserialize_with = "deserialize_orchestrator_patch")]
    pub active_effort: Option<Option<String>>,
}

pub async fn patch_member_config(
    State(state): State<AppState>,
    AxumPath((session_id, handle)): AxumPath<(String, String)>,
    Json(body): Json<PatchMemberConfigRequest>,
) -> ApiResult<Value> {
    let final_member = state
        .switch
        .store
        .update_member_model_effort(
            &session_id,
            &handle,
            body.active_model.as_ref().map(|o| o.as_deref()),
            body.active_effort.as_ref().map(|o| o.as_deref()),
        )
        .map_err(|e| err_resp(StatusCode::BAD_REQUEST, e.to_string()))?;

    let statuses = state.switch.get_member_statuses(&session_id).await;
    let (status, turn_id) = statuses
        .get(&handle)
        .cloned()
        .unwrap_or((MemberStatus::Hibernated, None));
    let _ = state.switch.events_tx.send(WsEvent::MemberStatusChanged {
        session_id: session_id.clone(),
        handle: handle.clone(),
        status,
        active_model: final_member.active_model.clone(),
        active_effort: final_member.active_effort.clone(),
        turn_id,
    });
    Ok(Json(json!({ "member": final_member })))
}

pub async fn get_session_context_handler(
    State(state): State<AppState>,
    AxumPath(session_id): AxumPath<String>,
) -> ApiResult<Value> {
    let report = state
        .switch
        .compute_session_context(&session_id)
        .await
        .map_err(map_switch_err)?;
    Ok(Json(json!({ "context": report })))
}

#[derive(Deserialize)]
pub struct DraftContextRequest {
    text: String,
}

pub async fn estimate_draft_context(Json(body): Json<DraftContextRequest>) -> ApiResult<Value> {
    if body.text.len() > 128 * 1024 {
        return Err(err_resp(
            StatusCode::PAYLOAD_TOO_LARGE,
            "Draft exceeds the token count limit.",
        ));
    }
    let tokens = tokio::task::spawn_blocking(move || mh_core::estimate_tokens(&body.text))
        .await
        .map_err(|error| err_resp(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
    Ok(Json(json!({ "tokens": tokens })))
}

pub async fn compact_session_handler(
    State(state): State<AppState>,
    AxumPath(session_id): AxumPath<String>,
) -> ApiResult<Value> {
    let msg = state
        .switch
        .compact_session(&session_id)
        .await
        .map_err(map_switch_err)?;
    Ok(Json(json!({ "message": msg })))
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

    state.switch.cancel_all_in_session(&session_id).await;
    state.switch.supervisor.shutdown_session(&session_id).await;
    if let Ok(repo_root) = state.switch.resolve_session_repo_root(&sess) {
        let _ = prune_session_worktrees(&repo_root, &session_id).await;
        if sess.project_id.is_none() {
            for attempt in 0..4 {
                if tokio::fs::remove_dir_all(&repo_root).await.is_ok() || !repo_root.exists() {
                    break;
                }
                if attempt < 3 {
                    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                }
            }
        }
    }
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
    if !state
        .switch
        .config
        .read()
        .await
        .adapters
        .contains_key(&body.adapter_id)
    {
        return Err(err_resp(StatusCode::BAD_REQUEST, "unknown adapter"));
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
    let member = if body.active_model.is_some() || body.active_effort.is_some() {
        state
            .switch
            .store
            .update_member_model_effort(
                &member.session_id,
                &member.handle,
                body.active_model.as_deref().map(Some),
                body.active_effort.as_deref().map(Some),
            )
            .map_err(|error| err_resp(StatusCode::BAD_REQUEST, error.to_string()))?
    } else {
        member
    };
    Ok(Json(json!({ "member": member })))
}

pub async fn delete_session_member(
    State(state): State<AppState>,
    AxumPath((session_id, handle)): AxumPath<(String, String)>,
) -> ApiResult<Value> {
    let runtime = state.switch.runtime(&session_id).await;
    let member_lock = {
        let mut locks = runtime.member_locks.lock().await;
        std::sync::Arc::clone(locks.entry(handle.clone()).or_default())
    };
    let _idle = member_lock.try_lock().map_err(|_| {
        err_resp(
            StatusCode::CONFLICT,
            "Stop this persona's turn before removing it.",
        )
    })?;
    state
        .switch
        .store
        .delete_member(&session_id, &handle)
        .map_err(|e| err_resp(StatusCode::NOT_FOUND, e.to_string()))?;
    state
        .switch
        .supervisor
        .retire_member(&session_id, &handle)
        .await;
    runtime.member_statuses.write().await.remove(&handle);
    runtime.active_member_chain.write().await.remove(&handle);
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
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub effort: Option<String>,
    #[serde(default)]
    pub recipient_overrides: HashMap<String, TurnModelSelection>,
}

pub async fn post_message(
    State(state): State<AppState>,
    AxumPath(session_id): AxumPath<String>,
    Json(body): Json<PostMessageRequest>,
) -> ApiResult<Value> {
    let msg = state
        .switch
        .handle_user_message_with_overrides(
            &session_id,
            body.content,
            TurnModelSelection {
                model: body.model,
                effort: body.effort,
            },
            body.recipient_overrides,
        )
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
    let repo_root = state
        .switch
        .resolve_session_repo_root(&sess)
        .map_err(map_switch_err)?;
    let (Some(wt), Some(br)) = (
        sess.integration_wt_path.as_deref(),
        sess.integration_branch.as_deref(),
    ) else {
        return Ok(Json(json!({
            "diff": null,
            "isolation_mode": sess.isolation_mode
        })));
    };
    let report = get_session_diff(
        &repo_root,
        Path::new(wt),
        sess.base_branch.as_deref(),
        sess.base_commit_sha.as_deref(),
        br,
    )
    .await
    .map_err(|e| err_resp(StatusCode::BAD_REQUEST, e.to_string()))?;
    Ok(Json(json!({ "diff": report })))
}

pub async fn serve_session_worktree_file(
    State(state): State<AppState>,
    AxumPath((session_id, rel_path)): AxumPath<(String, String)>,
    Query(query): Query<SessionFileQuery>,
    method: Method,
) -> Response {
    let mut response =
        read_session_worktree_file(&state, &session_id, &rel_path, query.preview, method).await;
    if query.preview
        && response.status().is_success()
        && guess_mime(&rel_path).starts_with("text/html")
        && let Ok(grant) = header::HeaderValue::from_str(&crate::web_preview::issue(&session_id))
    {
        response
            .headers_mut()
            .insert(crate::web_preview::GRANT_HEADER, grant);
        response.headers_mut().insert(
            header::CACHE_CONTROL,
            header::HeaderValue::from_static("private, no-store"),
        );
    }
    response
}

pub(crate) async fn read_session_worktree_file(
    state: &AppState,
    session_id: &str,
    rel_path: &str,
    preview: bool,
    method: Method,
) -> Response {
    use tokio::io::AsyncReadExt;

    let Ok(sess) = state.switch.store.get_session(session_id) else {
        return (StatusCode::NOT_FOUND, "session not found").into_response();
    };
    let Ok(repo_root) = state.switch.resolve_session_repo_root(&sess) else {
        return (StatusCode::NOT_FOUND, "session workspace not found").into_response();
    };
    let root_owned = match active_session_workspace(&sess, &repo_root) {
        Ok(root) => root,
        Err(error) => return error.into_response(),
    };
    let root = root_owned.as_path();
    let cleaned = rel_path.trim_start_matches(['/', '\\']);
    if cleaned.is_empty() {
        return (StatusCode::BAD_REQUEST, "empty path").into_response();
    }
    let candidate = root.join(cleaned);
    if !is_path_within(&candidate, root) || !candidate.is_file() {
        return (StatusCode::NOT_FOUND, "file not found").into_response();
    }
    const PREVIEW_LIMIT: u64 = 24 * 1024 * 1024;
    let Ok(file) = tokio::fs::File::open(&candidate).await else {
        return (StatusCode::FORBIDDEN, "cannot read file").into_response();
    };
    let mut data = Vec::new();
    let size = if method == Method::HEAD {
        let Ok(metadata) = file.metadata().await else {
            return (StatusCode::FORBIDDEN, "cannot read file").into_response();
        };
        metadata.len()
    } else {
        let max_bytes = if preview { PREVIEW_LIMIT + 1 } else { u64::MAX };
        if file.take(max_bytes).read_to_end(&mut data).await.is_err() {
            return (StatusCode::FORBIDDEN, "cannot read file").into_response();
        }
        data.len() as u64
    };
    if preview && size > PREVIEW_LIMIT {
        return (
            StatusCode::PAYLOAD_TOO_LARGE,
            "File is too large to preview.",
        )
            .into_response();
    }
    let mime = guess_mime(&candidate.to_string_lossy());
    let content_policy = if preview && mime.starts_with("text/html") {
        "sandbox allow-scripts; default-src 'self' data: blob:; script-src 'self' 'unsafe-inline' blob:; style-src 'self' 'unsafe-inline'; connect-src 'none'; form-action 'none'; frame-src 'none'; object-src 'none'; base-uri 'none'"
    } else {
        "sandbox allow-scripts allow-forms"
    };
    let mut response = (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, mime),
            (header::CONTENT_SECURITY_POLICY, content_policy),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
        ],
        data,
    )
        .into_response();
    response
        .headers_mut()
        .insert(header::CONTENT_LENGTH, header::HeaderValue::from(size));
    response
}

#[derive(Deserialize)]
pub struct SessionFileQuery {
    #[serde(default)]
    preview: bool,
}

pub(crate) fn active_session_workspace(
    sess: &Session,
    repo_root: &Path,
) -> Result<PathBuf, (StatusCode, String)> {
    if sess.archived_at.is_some() {
        return Err((
            StatusCode::CONFLICT,
            "Restore the session before opening its workspace.".into(),
        ));
    }
    match sess.isolation_mode {
        IsolationMode::SharedCwd if repo_root.is_dir() => Ok(repo_root.to_path_buf()),
        IsolationMode::WorktreePerMember => sess
            .integration_wt_path
            .as_deref()
            .map(PathBuf::from)
            .filter(|root| root.join(".git").is_file())
            .ok_or_else(|| {
                (
                    StatusCode::CONFLICT,
                    "The session integration worktree is unavailable.".into(),
                )
            }),
        _ => Err((
            StatusCode::NOT_FOUND,
            "The session workspace is unavailable.".into(),
        )),
    }
}

#[derive(Debug, Deserialize)]
pub struct RunCommandRequest {
    pub command: String,
    #[serde(default)]
    pub cwd: Option<String>,
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
    let repo_root = state
        .switch
        .resolve_session_repo_root(&sess)
        .map_err(map_switch_err)?;
    let base_root_owned = active_session_workspace(&sess, &repo_root)
        .map_err(|(status, error)| err_resp(status, error))?;
    let base_root = base_root_owned.as_path();
    let mut cwd = base_root.to_path_buf();
    if let Some(req_cwd) = body.cwd.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        let cand = PathBuf::from(req_cwd);
        if is_path_within(&cand, base_root) && cand.is_dir() {
            cwd = dunce::canonicalize(&cand).unwrap_or(cand);
        }
    }

    let trimmed = body.command.trim();
    if trimmed == "cd" || trimmed.starts_with("cd ") {
        let arg = trimmed
            .strip_prefix("cd")
            .unwrap_or("")
            .trim()
            .trim_matches(|c| c == '"' || c == '\'');
        let target = if arg.is_empty() {
            base_root.to_path_buf()
        } else {
            let p = PathBuf::from(arg);
            if p.is_absolute() { p } else { cwd.join(p) }
        };
        if !target.is_dir() || !is_path_within(&target, base_root) {
            return Ok(Json(json!({
                "cwd": dunce::simplified(&cwd).to_string_lossy(),
                "exit_code": 1,
                "stdout": "",
                "stderr": format!("cd: cannot access directory inside session workspace: {}", arg),
            })));
        }
        let clean = dunce::canonicalize(&target).unwrap_or(target);
        return Ok(Json(json!({
            "cwd": dunce::simplified(&clean).to_string_lossy(),
            "exit_code": 0,
            "stdout": "",
            "stderr": "",
        })));
    }

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
        "cwd": dunce::simplified(&cwd).to_string_lossy(),
        "exit_code": out.status.code().unwrap_or(-1),
        "stdout": stdout,
        "stderr": stderr,
    })))
}

pub async fn open_session_terminal() -> ApiResult<Value> {
    Err(err_resp(
        StatusCode::BAD_REQUEST,
        "OS terminal launching is disabled; use the built-in Web Shell.",
    ))
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
    let runtime = state.switch.runtime(&session_id).await;
    let _merge = runtime.git_merge_lock.lock().await;
    let sess = state
        .switch
        .store
        .get_session(&session_id)
        .map_err(|e| err_resp(StatusCode::NOT_FOUND, e.to_string()))?;
    if sess.project_id.is_none() {
        return Err(err_resp(
            StatusCode::BAD_REQUEST,
            "Standalone sessions do not have a parent repository branch to promote into.",
        ));
    }
    let repo_root = state
        .switch
        .resolve_session_repo_root(&sess)
        .map_err(map_switch_err)?;
    let int_br = sess
        .integration_branch
        .as_deref()
        .ok_or_else(|| err_resp(StatusCode::BAD_REQUEST, "session has no integration branch"))?;
    let base_br = sess.base_branch.as_deref().unwrap_or("main");
    let base_sha = sess.base_commit_sha.as_deref().unwrap_or(base_br);

    if let Some(wt) = sess.integration_wt_path.as_deref() {
        commit_worktree_if_dirty(Path::new(wt), "session", &sess.title)
            .await
            .map_err(|error| {
                err_resp(
                    StatusCode::CONFLICT,
                    format!("Could not save the integration worktree: {error}"),
                )
            })?;
    }

    let res = promote_session(&repo_root, base_br, base_sha, int_br, body.mode)
        .await
        .map_err(|e| err_resp(StatusCode::BAD_REQUEST, e.to_string()))?;

    if let Some(new_sha) = res.new_head_sha.as_deref() {
        if let Some(wt) = sess.integration_wt_path.as_deref() {
            reset_worktree_to_sha(Path::new(wt), new_sha).await.map_err(|error| {
                err_resp(StatusCode::CONFLICT, format!(
                    "Promoted changes to {base_br}, but could not update the session worktree: {error}",
                ))
            })?;
        }
        state
            .switch
            .store
            .update_session_git_state(
                &session_id,
                Some(base_br),
                Some(new_sha),
                Some(int_br),
                sess.integration_wt_path.as_deref(),
                sess.isolation_mode,
            )
            .map_err(|error| err_resp(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
        if let Ok(members) = state.switch.store.list_members(&session_id) {
            for m in members {
                if let Some(p_wt) = m.worktree_path.as_deref() {
                    let member_lock = {
                        let mut locks = runtime.member_locks.lock().await;
                        std::sync::Arc::clone(locks.entry(m.handle).or_default())
                    };
                    if let Ok(_idle) = member_lock.try_lock() {
                        let _ = sync_member_worktree(Path::new(p_wt), int_br).await;
                    }
                }
            }
        }
        let _ = state.switch.events_tx.send(WsEvent::GitBranchUpdated {
            session_id,
            integration_branch: int_br.to_string(),
            ahead_commits: 0,
            diff_stat: DiffStat::default(),
        });
    }

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

async fn dir_size_bytes(root: PathBuf) -> u64 {
    tokio::task::spawn_blocking(move || {
        let mut total = 0u64;
        if !root.exists() {
            return 0;
        }
        let mut stack = vec![root];
        while let Some(dir) = stack.pop() {
            let Ok(rd) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in rd.flatten() {
                let path = entry.path();
                let Ok(meta) = std::fs::symlink_metadata(&path) else {
                    continue;
                };
                if meta.file_type().is_symlink() {
                    continue;
                }
                if meta.is_dir() {
                    stack.push(path);
                } else if meta.is_file() {
                    total = total.saturating_add(meta.len());
                }
            }
        }
        total
    })
    .await
    .unwrap_or(0)
}

async fn compute_storage_summary(switch: &mh_switch::SwitchEngine) -> StorageSummary {
    let db_path = switch.app_paths.db_path();
    let mut database_bytes = 0u64;
    for candidate in [
        db_path.clone(),
        PathBuf::from(format!("{}-wal", db_path.display())),
        PathBuf::from(format!("{}-shm", db_path.display())),
    ] {
        if let Ok(meta) = tokio::fs::metadata(&candidate).await {
            database_bytes = database_bytes.saturating_add(meta.len());
        }
    }

    let projects = switch.store.list_projects().unwrap_or_default();
    let all_sessions = switch.store.list_all_sessions(true).unwrap_or_default();
    let active_ids: std::collections::HashSet<String> = all_sessions
        .iter()
        .filter(|s| s.archived_at.is_none())
        .map(|s| s.id.clone())
        .collect();
    let archived_ids: std::collections::HashSet<String> = all_sessions
        .iter()
        .filter(|s| s.archived_at.is_some())
        .map(|s| s.id.clone())
        .collect();

    let mut active_worktrees_bytes = 0u64;
    let mut active_worktrees_count = 0u32;
    let mut standalone_workspaces_bytes = 0u64;
    let mut standalone_count = 0u32;
    let mut archived_bytes = 0u64;
    let mut cache_and_orphans_bytes = 0u64;
    let mut orphan_dirs_count = 0u32;

    for proj in &projects {
        let wt_root = Path::new(&proj.root_path)
            .join(".meta-harness")
            .join("worktrees");
        if let Ok(mut rd) = tokio::fs::read_dir(&wt_root).await {
            while let Ok(Some(entry)) = rd.next_entry().await {
                let p = entry.path();
                if !p.is_dir() {
                    continue;
                }
                let name = entry.file_name().to_string_lossy().into_owned();
                let sz = dir_size_bytes(p).await;
                if active_ids.contains(&name) {
                    active_worktrees_count = active_worktrees_count.saturating_add(1);
                    active_worktrees_bytes = active_worktrees_bytes.saturating_add(sz);
                } else if archived_ids.contains(&name) {
                    archived_bytes = archived_bytes.saturating_add(sz);
                } else {
                    orphan_dirs_count = orphan_dirs_count.saturating_add(1);
                    cache_and_orphans_bytes = cache_and_orphans_bytes.saturating_add(sz);
                }
            }
        }
    }

    let ws_root = &switch.app_paths.workspaces_dir;
    if let Ok(mut rd) = tokio::fs::read_dir(ws_root).await {
        while let Ok(Some(entry)) = rd.next_entry().await {
            let p = entry.path();
            if !p.is_dir() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            let sz = dir_size_bytes(p).await;
            if active_ids.contains(&name) {
                standalone_count = standalone_count.saturating_add(1);
                standalone_workspaces_bytes = standalone_workspaces_bytes.saturating_add(sz);
            } else if archived_ids.contains(&name) {
                archived_bytes = archived_bytes.saturating_add(sz);
            } else {
                orphan_dirs_count = orphan_dirs_count.saturating_add(1);
                cache_and_orphans_bytes = cache_and_orphans_bytes.saturating_add(sz);
            }
        }
    }

    for sub in ["cache", "logs"] {
        let dir = switch.app_paths.data_dir.join(sub);
        if dir.exists() {
            cache_and_orphans_bytes =
                cache_and_orphans_bytes.saturating_add(dir_size_bytes(dir).await);
        }
    }

    let total_bytes = database_bytes
        .saturating_add(active_worktrees_bytes)
        .saturating_add(standalone_workspaces_bytes)
        .saturating_add(archived_bytes)
        .saturating_add(cache_and_orphans_bytes);

    StorageSummary {
        total_bytes,
        database_bytes,
        active_worktrees_bytes,
        active_worktrees_count,
        standalone_workspaces_bytes,
        standalone_count,
        archived_sessions_count: archived_ids.len() as u32,
        archived_bytes,
        cache_and_orphans_bytes,
        orphan_dirs_count,
        data_dir: dunce::simplified(&switch.app_paths.data_dir)
            .to_string_lossy()
            .into_owned(),
        workspaces_dir: dunce::simplified(&switch.app_paths.workspaces_dir)
            .to_string_lossy()
            .into_owned(),
    }
}

pub async fn get_storage_handler(State(state): State<AppState>) -> ApiResult<Value> {
    let storage = compute_storage_summary(&state.switch).await;
    Ok(Json(json!({ "storage": storage })))
}

#[derive(Debug, Deserialize)]
pub struct CleanupStorageRequest {
    pub action: String,
}

pub async fn cleanup_storage_handler(
    State(state): State<AppState>,
    Json(body): Json<CleanupStorageRequest>,
) -> ApiResult<Value> {
    let action = body.action.trim().to_string();
    if action != "clear_caches" && action != "delete_archived" {
        return Err(err_resp(
            StatusCode::BAD_REQUEST,
            "action must be 'clear_caches' or 'delete_archived'",
        ));
    }

    let before = compute_storage_summary(&state.switch).await;
    let mut removed_sessions = 0usize;
    let mut removed_dirs = 0usize;

    if action == "clear_caches" {
        let all_ids: std::collections::HashSet<String> = state
            .switch
            .store
            .list_all_sessions(true)
            .map_err(|error| err_resp(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?
            .into_iter()
            .map(|s| s.id)
            .collect();
        let projects = state
            .switch
            .store
            .list_projects()
            .map_err(|error| err_resp(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
        for proj in &projects {
            let wt_root = Path::new(&proj.root_path)
                .join(".meta-harness")
                .join("worktrees");
            if let Ok(mut rd) = tokio::fs::read_dir(&wt_root).await {
                while let Ok(Some(entry)) = rd.next_entry().await {
                    let p = entry.path();
                    if !p.is_dir() {
                        continue;
                    }
                    let name = entry.file_name().to_string_lossy().into_owned();
                    if !all_ids.contains(&name) {
                        let _ = prune_session_worktrees(Path::new(&proj.root_path), &name).await;
                        if p.exists() {
                            let _ = tokio::fs::remove_dir_all(&p).await;
                        }
                        removed_dirs += 1;
                    }
                }
            }
        }
        let ws_root = &state.switch.app_paths.workspaces_dir;
        if let Ok(mut rd) = tokio::fs::read_dir(ws_root).await {
            while let Ok(Some(entry)) = rd.next_entry().await {
                let p = entry.path();
                if !p.is_dir() {
                    continue;
                }
                let name = entry.file_name().to_string_lossy().into_owned();
                if !all_ids.contains(&name) && tokio::fs::remove_dir_all(&p).await.is_ok() {
                    removed_dirs += 1;
                }
            }
        }
        for sub in ["cache", "logs"] {
            let dir = state.switch.app_paths.data_dir.join(sub);
            if dir.exists() {
                let _ = tokio::fs::remove_dir_all(&dir).await;
            }
        }
        let _ = state.switch.store.vacuum();
    } else {
        let archived = state
            .switch
            .store
            .delete_archived_sessions()
            .map_err(|e| err_resp(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        removed_sessions = archived.len();
        for sess in &archived {
            state.switch.supervisor.shutdown_session(&sess.id).await;
            if let Some(ref pid) = sess.project_id {
                if let Ok(proj) = state.switch.store.get_project(pid)
                    && prune_session_worktrees(Path::new(&proj.root_path), &sess.id)
                        .await
                        .is_ok()
                {
                    removed_dirs += 1;
                }
            } else {
                let ws_root = state.switch.app_paths.standalone_session_root(&sess.id);
                if ws_root.exists() {
                    for attempt in 0..4 {
                        if tokio::fs::remove_dir_all(&ws_root).await.is_ok() || !ws_root.exists() {
                            removed_dirs += 1;
                            break;
                        }
                        if attempt < 3 {
                            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                        }
                    }
                }
            }
        }
        let _ = state.switch.store.vacuum();
    }

    let after = compute_storage_summary(&state.switch).await;
    let freed_bytes = before.total_bytes.saturating_sub(after.total_bytes);
    Ok(Json(json!({
        "result": {
            "action": action,
            "removed_sessions": removed_sessions,
            "removed_dirs": removed_dirs,
            "freed_bytes": freed_bytes,
        },
        "storage": after,
    })))
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
    let lower = path.to_ascii_lowercase();
    if lower.ends_with(".html") || lower.ends_with(".htm") {
        "text/html; charset=utf-8"
    } else if lower.ends_with(".js") || lower.ends_with(".mjs") {
        "application/javascript; charset=utf-8"
    } else if lower.ends_with(".css") {
        "text/css; charset=utf-8"
    } else if lower.ends_with(".svg") {
        "image/svg+xml"
    } else if lower.ends_with(".png") {
        "image/png"
    } else if lower.ends_with(".webp") {
        "image/webp"
    } else if lower.ends_with(".avif") {
        "image/avif"
    } else if lower.ends_with(".bmp") {
        "image/bmp"
    } else if lower.ends_with(".jpg") || lower.ends_with(".jpeg") {
        "image/jpeg"
    } else if lower.ends_with(".gif") {
        "image/gif"
    } else if lower.ends_with(".ico") {
        "image/x-icon"
    } else if lower.ends_with(".woff2") {
        "font/woff2"
    } else if lower.ends_with(".json") {
        "application/json"
    } else if lower.ends_with(".wasm") {
        "application/wasm"
    } else if lower.ends_with(".pdf") {
        "application/pdf"
    } else if lower.ends_with(".md") || lower.ends_with(".txt") {
        "text/plain; charset=utf-8"
    } else {
        "application/octet-stream"
    }
}
