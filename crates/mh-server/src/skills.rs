use std::collections::HashSet;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use axum::{
    Json,
    extract::{Path as AxumPath, Query, State},
    http::StatusCode,
};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::io::AsyncReadExt;
use tokio::process::Command;

use mh_acp::{ProcessTreeGuard, prepare_isolated_command, resolve_command};
use mh_core::{
    MAX_SKILL_FILE_BYTES, SkillSource, SkillSummary, ensure_skill_frontmatter, expand_user_path,
    is_path_within, normalize_path, parse_skill_markdown, resolve_skill_file, sanitize_skill_slug,
    scan_skill_dir,
};
use mh_store::SkillStateRow;
use mh_switch::{
    BUILTIN_META_HARNESS_DOCS_ID, SwitchError, project_skill_state_key,
    read_skill_file_from_summary, reset_builtin_skill,
};

use crate::{
    AppState,
    routes::{ApiResult, err_resp},
};

#[derive(Debug, Deserialize)]
pub struct SkillsListQuery {
    pub project_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct SkillDetailQuery {
    pub project_id: Option<String>,
    pub file: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CreateCustomSkillRequest {
    pub id: Option<String>,
    pub name: String,
    pub description: String,
    pub content: String,
}

#[derive(Debug, Deserialize)]
pub struct UpdateSkillContentRequest {
    pub file: Option<String>,
    pub content: String,
}

#[derive(Debug, Deserialize)]
pub struct PatchSkillRequest {
    pub enabled: bool,
    pub project_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct InstallFolderRequest {
    pub path: String,
}

#[derive(Debug, Deserialize)]
pub struct InstallGitRequest {
    pub url: String,
    pub subpath: Option<String>,
}

struct TempDirGuard {
    path: PathBuf,
}

impl TempDirGuard {
    fn create(prefix: &str) -> Result<Self, (StatusCode, Json<Value>)> {
        Self::create_in(&std::env::temp_dir(), prefix)
    }

    fn create_in(parent: &Path, prefix: &str) -> Result<Self, (StatusCode, Json<Value>)> {
        let dir = parent.join(format!("{prefix}_{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir(&dir)
            .map_err(|e| err_resp(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        Ok(Self { path: dir })
    }
}

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn map_skill_switch_err(err: SwitchError) -> (StatusCode, Json<Value>) {
    match err {
        SwitchError::NotFound(m) => err_resp(StatusCode::NOT_FOUND, m),
        SwitchError::Validation(m) => err_resp(StatusCode::BAD_REQUEST, m),
        other => err_resp(StatusCode::INTERNAL_SERVER_ERROR, other.to_string()),
    }
}

fn resolve_project_root(
    state: &AppState,
    project_id: Option<&str>,
) -> Result<Option<PathBuf>, (StatusCode, Json<Value>)> {
    let Some(pid) = project_id.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(None);
    };
    let proj = state
        .switch
        .store
        .get_project(pid)
        .map_err(|e| err_resp(StatusCode::NOT_FOUND, e.to_string()))?;
    Ok(Some(PathBuf::from(proj.root_path)))
}

pub async fn list_skills(
    State(state): State<AppState>,
    Query(q): Query<SkillsListQuery>,
) -> ApiResult<Value> {
    let project_root = resolve_project_root(&state, q.project_id.as_deref())?;
    let skills = state
        .switch
        .list_skills_for_project(project_root.as_deref(), None)
        .map_err(map_skill_switch_err)?;
    let skills_dir = dunce::simplified(&state.switch.app_paths.skills_dir())
        .to_string_lossy()
        .into_owned();
    Ok(Json(json!({
        "skills": skills,
        "skills_dir": skills_dir,
    })))
}

pub async fn get_skill_detail(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
    Query(q): Query<SkillDetailQuery>,
) -> ApiResult<Value> {
    let project_root = resolve_project_root(&state, q.project_id.as_deref())?;
    let slug = id.trim().trim_start_matches('/').to_ascii_lowercase();
    let skills = state
        .switch
        .list_skills_for_project(project_root.as_deref(), None)
        .map_err(map_skill_switch_err)?;
    let Some(skill) = skills.into_iter().find(|s| s.id == slug) else {
        return Err(err_resp(
            StatusCode::NOT_FOUND,
            format!("unknown skill: {id}"),
        ));
    };
    let detail =
        read_skill_file_from_summary(&skill, q.file.as_deref()).map_err(map_skill_switch_err)?;
    Ok(Json(json!({ "detail": detail })))
}

pub async fn create_custom_skill(
    State(state): State<AppState>,
    Json(body): Json<CreateCustomSkillRequest>,
) -> ApiResult<Value> {
    let raw_slug = body
        .id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(&body.name);
    let slug =
        sanitize_skill_slug(raw_slug).map_err(|msg| err_resp(StatusCode::BAD_REQUEST, msg))?;
    if slug == BUILTIN_META_HARNESS_DOCS_ID {
        return Err(err_resp(
            StatusCode::BAD_REQUEST,
            "Cannot overwrite the preinstalled 'meta-harness-docs' skill slug.",
        ));
    }

    let name = body.name.trim();
    if name.is_empty() || name.len() > 120 {
        return Err(err_resp(
            StatusCode::BAD_REQUEST,
            "Skill name must be 1 to 120 characters.",
        ));
    }
    let description = body.description.trim();
    if description.is_empty() || description.len() > 600 {
        return Err(err_resp(
            StatusCode::BAD_REQUEST,
            "Skill description must be 1 to 600 characters.",
        ));
    }

    let full_md = ensure_skill_frontmatter(&slug, name, description, &body.content);
    parse_skill_markdown(&full_md, &slug).map_err(|msg| err_resp(StatusCode::BAD_REQUEST, msg))?;
    if full_md.len() as u64 > MAX_SKILL_FILE_BYTES {
        return Err(err_resp(
            StatusCode::BAD_REQUEST,
            "SKILL.md cannot exceed 512 KiB.",
        ));
    }

    let _mutation = state.switch.skill_mutations.lock().await;
    let skills_root = state.switch.app_paths.skills_dir();
    tokio::fs::create_dir_all(&skills_root)
        .await
        .map_err(|e| err_resp(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let staging = TempDirGuard::create_in(&skills_root, ".skill-install")?;
    let staged_skill_dir = staging.path.join(&slug);
    tokio::fs::create_dir(&staged_skill_dir)
        .await
        .map_err(|e| err_resp(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    tokio::fs::write(staged_skill_dir.join("SKILL.md"), full_md)
        .await
        .map_err(|e| err_resp(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    publish_staged_skills(
        &state,
        &skills_root,
        &staging.path,
        &[SkillStateRow {
            state_key: slug.clone(),
            source: SkillSource::Custom,
            origin: Some("inline".into()),
            enabled: true,
            builtin_hash: None,
            updated_at: now_millis(),
        }],
    )?;

    let all = state
        .switch
        .list_skills_for_project(None, None)
        .map_err(map_skill_switch_err)?;
    let skill = all
        .into_iter()
        .find(|s| s.id == slug)
        .ok_or_else(|| err_resp(StatusCode::INTERNAL_SERVER_ERROR, "created skill missing"))?;
    Ok(Json(json!({ "skill": skill })))
}

pub async fn update_skill_content(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
    Json(body): Json<UpdateSkillContentRequest>,
) -> ApiResult<Value> {
    let _mutation = state.switch.skill_mutations.lock().await;
    let slug = id.trim().trim_start_matches('/').to_ascii_lowercase();
    let skills = state
        .switch
        .list_skills_for_project(None, None)
        .map_err(map_skill_switch_err)?;
    let Some(existing) = skills.into_iter().find(|s| s.id == slug) else {
        return Err(err_resp(
            StatusCode::NOT_FOUND,
            format!(
                "Global skill '{id}' not found. Repo-local skills are edited in the repository."
            ),
        ));
    };
    if existing.source == SkillSource::Project {
        return Err(err_resp(
            StatusCode::BAD_REQUEST,
            "Repo-local skills live in `.agents/skills/` and are edited inside the repository.",
        ));
    }

    let rel = body
        .file
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("SKILL.md");
    if rel.eq_ignore_ascii_case("SKILL.md") {
        parse_skill_markdown(&body.content, &slug)
            .map_err(|msg| err_resp(StatusCode::BAD_REQUEST, msg))?;
    }
    let root = Path::new(&existing.skill_dir);
    let target =
        resolve_skill_file(root, rel).map_err(|msg| err_resp(StatusCode::BAD_REQUEST, msg))?;
    if body.content.len() as u64 > MAX_SKILL_FILE_BYTES {
        return Err(err_resp(
            StatusCode::BAD_REQUEST,
            "Skill file content cannot exceed 512 KiB.",
        ));
    }
    if let Some(parent) = target.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| err_resp(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    }
    let staging = TempDirGuard::create_in(root, ".skill-edit")?;
    let staged_file = staging.path.join("content");
    tokio::fs::write(&staged_file, &body.content)
        .await
        .map_err(|e| err_resp(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    tokio::fs::rename(&staged_file, &target)
        .await
        .map_err(|e| err_resp(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let refreshed = state
        .switch
        .list_skills_for_project(None, None)
        .map_err(map_skill_switch_err)?;
    let updated = refreshed
        .into_iter()
        .find(|s| s.id == slug)
        .ok_or_else(|| err_resp(StatusCode::INTERNAL_SERVER_ERROR, "updated skill missing"))?;
    let detail = read_skill_file_from_summary(&updated, Some(rel)).map_err(map_skill_switch_err)?;
    Ok(Json(json!({ "detail": detail })))
}

pub async fn patch_skill(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
    Json(body): Json<PatchSkillRequest>,
) -> ApiResult<Value> {
    let _mutation = state.switch.skill_mutations.lock().await;
    let project_root = resolve_project_root(&state, body.project_id.as_deref())?;
    let slug = id.trim().trim_start_matches('/').to_ascii_lowercase();
    let skills = state
        .switch
        .list_skills_for_project(project_root.as_deref(), None)
        .map_err(map_skill_switch_err)?;
    let Some(skill) = skills.into_iter().find(|s| s.id == slug) else {
        return Err(err_resp(
            StatusCode::NOT_FOUND,
            format!("unknown skill: {id}"),
        ));
    };

    let state_key = if skill.source == SkillSource::Project {
        let Some(ref root) = project_root else {
            return Err(err_resp(
                StatusCode::BAD_REQUEST,
                "project_id is required when toggling a repo-local skill.",
            ));
        };
        project_skill_state_key(root, &skill.id)
    } else {
        skill.id.clone()
    };

    state
        .switch
        .store
        .set_skill_enabled(
            &state_key,
            skill.source,
            skill.origin.as_deref(),
            body.enabled,
        )
        .map_err(|e| err_resp(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let refreshed = state
        .switch
        .list_skills_for_project(project_root.as_deref(), None)
        .map_err(map_skill_switch_err)?;
    let updated = refreshed
        .into_iter()
        .find(|s| s.id == slug)
        .ok_or_else(|| err_resp(StatusCode::INTERNAL_SERVER_ERROR, "skill vanished"))?;
    Ok(Json(json!({ "skill": updated })))
}

pub async fn reset_skill_handler(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
) -> ApiResult<Value> {
    let _mutation = state.switch.skill_mutations.lock().await;
    let skill = reset_builtin_skill(&state.switch.app_paths, &state.switch.store, &id)
        .map_err(map_skill_switch_err)?;
    Ok(Json(json!({ "skill": skill })))
}

pub async fn delete_skill(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
) -> ApiResult<Value> {
    let _mutation = state.switch.skill_mutations.lock().await;
    let slug = sanitize_skill_slug(&id).map_err(|msg| err_resp(StatusCode::BAD_REQUEST, msg))?;
    if slug == BUILTIN_META_HARNESS_DOCS_ID {
        return Err(err_resp(
            StatusCode::BAD_REQUEST,
            "The preinstalled Meta-Harness Docs skill can be disabled or reset, not uninstalled.",
        ));
    }

    let root = state.switch.app_paths.skills_dir();
    let target = root.join(&slug);
    if !target.exists() {
        return Err(err_resp(
            StatusCode::NOT_FOUND,
            format!("Installed skill '{slug}' not found (repo-local skills are managed in git)."),
        ));
    }
    if !is_path_within(&target, &root)
        || tokio::fs::symlink_metadata(&target)
            .await
            .is_ok_and(|meta| meta.file_type().is_symlink())
    {
        return Err(err_resp(
            StatusCode::BAD_REQUEST,
            "Invalid installed skill directory.",
        ));
    }
    tokio::fs::remove_dir_all(&target)
        .await
        .map_err(|e| err_resp(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let _ = state.switch.store.delete_skill_state(&slug);
    Ok(Json(json!({ "deleted": true })))
}

pub async fn install_skill_from_folder(
    State(state): State<AppState>,
    Json(body): Json<InstallFolderRequest>,
) -> ApiResult<Value> {
    let expanded = expand_user_path(&body.path);
    if !expanded.is_dir() {
        return Err(err_resp(
            StatusCode::BAD_REQUEST,
            format!("Not a directory: {}", expanded.display()),
        ));
    }
    let norm = normalize_path(&expanded).unwrap_or(expanded);
    let origin = dunce::simplified(&norm).to_string_lossy().into_owned();
    let _mutation = state.switch.skill_mutations.lock().await;
    let installed = import_skills_from_root(&state, &norm, SkillSource::Local, &origin)?;
    Ok(Json(json!({ "installed": installed })))
}

pub async fn install_skill_from_git(
    State(state): State<AppState>,
    Json(body): Json<InstallGitRequest>,
) -> ApiResult<Value> {
    let (clone_url, branch, url_subpath) =
        parse_git_skill_source(&body.url, body.subpath.as_deref())
            .map_err(|msg| err_resp(StatusCode::BAD_REQUEST, msg))?;

    let temp_guard = TempDirGuard::create("mh_skill_git")?;
    clone_git_repo(&clone_url, branch.as_deref(), &temp_guard.path).await?;

    let scan_root = if let Some(ref sub) = url_subpath {
        let candidate = temp_guard.path.join(sub);
        if !is_path_within(&candidate, &temp_guard.path) || !candidate.is_dir() {
            return Err(err_resp(
                StatusCode::BAD_REQUEST,
                format!("Subpath '{sub}' was not found inside the cloned repository."),
            ));
        }
        candidate
    } else {
        temp_guard.path.clone()
    };

    let origin = if let Some(sub) = url_subpath {
        format!("{} ({sub})", body.url.trim())
    } else {
        clone_url
    };
    let _mutation = state.switch.skill_mutations.lock().await;
    let installed = import_skills_from_root(&state, &scan_root, SkillSource::Git, &origin)?;
    Ok(Json(json!({ "installed": installed })))
}

fn import_skills_from_root(
    state: &AppState,
    root: &Path,
    source: SkillSource,
    origin: &str,
) -> Result<Vec<SkillSummary>, (StatusCode, Json<Value>)> {
    let candidates = discover_candidate_skill_dirs(root);
    if candidates.is_empty() {
        return Err(err_resp(
            StatusCode::BAD_REQUEST,
            format!(
                "No SKILL.md found in '{}' or its immediate skill subdirectories.",
                root.display()
            ),
        ));
    }

    let skills_root = state.switch.app_paths.skills_dir();
    std::fs::create_dir_all(&skills_root)
        .map_err(|e| err_resp(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let now = now_millis();
    let staging = TempDirGuard::create_in(&skills_root, ".skill-import")?;
    let mut rows = Vec::new();
    let mut installed_ids = HashSet::new();
    for dir in candidates {
        let folder_fallback = dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("custom-skill");
        let fallback_slug = sanitize_skill_slug(folder_fallback).unwrap_or_else(|_| "skill".into());
        let (meta, _, _) = scan_skill_dir(&dir, &fallback_slug)
            .map_err(|msg| err_resp(StatusCode::BAD_REQUEST, msg))?;
        let slug = sanitize_skill_slug(&meta.slug)
            .map_err(|msg| err_resp(StatusCode::BAD_REQUEST, msg))?;
        if slug == BUILTIN_META_HARNESS_DOCS_ID {
            return Err(err_resp(
                StatusCode::BAD_REQUEST,
                "Cannot overwrite the preinstalled 'meta-harness-docs' skill.",
            ));
        }
        if !installed_ids.insert(slug.clone()) {
            return Err(err_resp(
                StatusCode::CONFLICT,
                format!("Multiple skills use '/{slug}'."),
            ));
        }
        let staged_dir = staging.path.join(&slug);
        let mut total_files = 0usize;
        let mut total_bytes = 0u64;
        copy_skill_tree(
            &dir,
            &dir,
            &staged_dir,
            0,
            &mut total_files,
            &mut total_bytes,
        )
        .map_err(|err| err_resp(StatusCode::BAD_REQUEST, err))?;
        scan_skill_dir(&staged_dir, &slug).map_err(|err| err_resp(StatusCode::BAD_REQUEST, err))?;
        rows.push(SkillStateRow {
            state_key: slug,
            source,
            origin: Some(origin.to_string()),
            enabled: true,
            builtin_hash: None,
            updated_at: now,
        });
    }
    publish_staged_skills(state, &skills_root, &staging.path, &rows)?;

    let all = state
        .switch
        .list_skills_for_project(None, None)
        .map_err(map_skill_switch_err)?;
    Ok(all
        .into_iter()
        .filter(|s| installed_ids.contains(&s.id))
        .collect())
}

fn publish_staged_skills(
    state: &AppState,
    skills_root: &Path,
    staging: &Path,
    rows: &[SkillStateRow],
) -> Result<(), (StatusCode, Json<Value>)> {
    let existing = state
        .switch
        .list_skills_for_project(None, None)
        .map_err(map_skill_switch_err)?;
    let mut published = Vec::new();
    let result = (|| {
        for row in rows {
            let slug = &row.state_key;
            let dest = skills_root.join(slug);
            if existing.iter().any(|skill| skill.id == *slug) {
                return Err(err_resp(
                    StatusCode::CONFLICT,
                    format!(
                        "Skill '/{slug}' already exists. Open it to edit or uninstall it first."
                    ),
                ));
            }
            match std::fs::symlink_metadata(&dest) {
                Err(err) if err.kind() == ErrorKind::NotFound => {}
                Ok(_) => {
                    return Err(err_resp(
                        StatusCode::CONFLICT,
                        format!("The folder for skill '/{slug}' already exists."),
                    ));
                }
                Err(err) => {
                    return Err(err_resp(StatusCode::INTERNAL_SERVER_ERROR, err.to_string()));
                }
            }
        }
        for row in rows {
            let staged_dir = staging.join(&row.state_key);
            let dest = skills_root.join(&row.state_key);
            std::fs::rename(&staged_dir, &dest)
                .map_err(|e| err_resp(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
            published.push((staged_dir, dest));
        }
        state
            .switch
            .store
            .upsert_skill_states(rows)
            .map_err(|e| err_resp(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
    })();
    if result.is_err() {
        for (staged_dir, dest) in published.into_iter().rev() {
            if let Err(error) = std::fs::rename(&dest, &staged_dir) {
                tracing::warn!(path = %dest.display(), %error, "Could not roll back skill installation");
            }
        }
    }
    result
}

fn discover_candidate_skill_dirs(root: &Path) -> Vec<PathBuf> {
    if root.join("SKILL.md").is_file() {
        return vec![root.to_path_buf()];
    }

    let mut found = Vec::new();
    let collect_direct = |parent: &Path, out: &mut Vec<PathBuf>| {
        if !is_path_within(parent, root) {
            return;
        }
        let Ok(rd) = std::fs::read_dir(parent) else {
            return;
        };
        let mut entries: Vec<_> = rd.flatten().collect();
        entries.sort_by_key(|e| e.file_name());
        for entry in entries {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') || name == "node_modules" {
                continue;
            }
            if path.is_dir() && is_path_within(&path, parent) && path.join("SKILL.md").is_file() {
                out.push(path);
            }
        }
    };

    collect_direct(root, &mut found);
    if found.is_empty() {
        for sub in [root.join("skills"), root.join(".agents").join("skills")] {
            if sub.is_dir() {
                collect_direct(&sub, &mut found);
            }
        }
    }
    found
}

fn copy_skill_tree(
    root: &Path,
    src: &Path,
    dest: &Path,
    depth: usize,
    total_files: &mut usize,
    total_bytes: &mut u64,
) -> Result<(), String> {
    if depth > 6 {
        return Err("Skill folder directory depth exceeds limit (6).".into());
    }
    std::fs::create_dir_all(dest).map_err(|e| format!("Failed to create staging dir: {e}"))?;
    let rd = std::fs::read_dir(src)
        .map_err(|e| format!("Failed to read skill folder {}: {e}", src.display()))?;
    for entry in rd.flatten() {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if name_str.starts_with('.') || name_str == "node_modules" {
            continue;
        }
        let Ok(ft) = entry.file_type() else {
            continue;
        };
        if ft.is_symlink() {
            continue;
        }
        let src_path = entry.path();
        if !is_path_within(&src_path, root) {
            continue;
        }
        let dest_path = dest.join(&name);
        if ft.is_dir() {
            copy_skill_tree(
                root,
                &src_path,
                &dest_path,
                depth + 1,
                total_files,
                total_bytes,
            )?;
        } else if ft.is_file() {
            *total_files += 1;
            if *total_files > 200 {
                return Err("Skill package exceeds maximum of 200 files.".into());
            }
            let sz = entry.metadata().map(|m| m.len()).unwrap_or(0);
            *total_bytes += sz;
            if *total_bytes > 20 * 1024 * 1024 {
                return Err("Skill package exceeds maximum size of 20 MiB.".into());
            }
            std::fs::copy(&src_path, &dest_path)
                .map_err(|e| format!("Failed copying {}: {e}", name_str))?;
        }
    }
    Ok(())
}

pub(crate) fn parse_git_skill_source(
    raw_url: &str,
    explicit_subpath: Option<&str>,
) -> Result<(String, Option<String>, Option<String>), String> {
    let url = raw_url.trim();
    if url.is_empty() || url.starts_with('-') || url.chars().any(char::is_control) {
        return Err("Enter a valid Git repository URL or GitHub owner/repo.".into());
    }

    let clean_subpath = |raw: &str| -> Result<Option<String>, String> {
        let trimmed = raw.trim().trim_matches('/');
        if trimmed.is_empty() {
            return Ok(None);
        }
        if trimmed
            .split('/')
            .any(|seg| seg.is_empty() || seg == "." || seg == "..")
        {
            return Err("Subpath must be a clean relative path inside the repository.".into());
        }
        Ok(Some(trimmed.to_string()))
    };

    let mut branch: Option<String> = None;
    let mut subpath = explicit_subpath.map(clean_subpath).transpose()?.flatten();

    let normalized_url = if let Some(rest) = url
        .strip_prefix("https://github.com/")
        .or_else(|| url.strip_prefix("http://github.com/"))
    {
        let parts: Vec<&str> = rest.trim_end_matches('/').split('/').collect();
        if parts.len() < 2 {
            return Err("GitHub URL must include owner and repository name.".into());
        }
        let owner = parts[0];
        let repo = parts[1].trim_end_matches(".git");
        if parts.len() >= 4 && parts[2] == "tree" {
            branch = Some(parts[3].to_string());
            if parts.len() > 4 && subpath.is_none() {
                subpath = clean_subpath(&parts[4..].join("/"))?;
            }
        }
        format!("https://github.com/{owner}/{repo}.git")
    } else if !url.contains("://") && !url.starts_with("git@") {
        let parts: Vec<&str> = url.trim_end_matches('/').split('/').collect();
        let valid_part = |s: &str| {
            !s.is_empty()
                && s.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
        };
        if parts.len() == 2 && valid_part(parts[0]) && valid_part(parts[1]) {
            let repo = parts[1].trim_end_matches(".git");
            format!("https://github.com/{}/{repo}.git", parts[0])
        } else {
            return Err(
                "Enter an https://, ssh://, git@ URL, or GitHub 'owner/repo' shorthand.".into(),
            );
        }
    } else if url.starts_with("https://")
        || url.starts_with("http://")
        || url.starts_with("ssh://")
        || url.starts_with("git@")
    {
        url.to_string()
    } else {
        return Err(
            "Unsupported Git URL scheme. Use https://, ssh://, git@, or owner/repo.".into(),
        );
    };

    Ok((normalized_url, branch, subpath))
}

async fn clone_git_repo(
    clone_url: &str,
    branch: Option<&str>,
    dest: &Path,
) -> Result<(), (StatusCode, Json<Value>)> {
    let mut args = [
        "-c",
        "core.hooksPath=/dev/null",
        "-c",
        "core.fsmonitor=false",
        "clone",
        "--no-recurse-submodules",
        "--depth",
        "1",
    ]
    .into_iter()
    .map(str::to_string)
    .collect::<Vec<_>>();
    if let Some(br) = branch.filter(|s| !s.is_empty()) {
        args.push("--branch".into());
        args.push(br.to_string());
    }
    args.push("--".into());
    args.push(clone_url.to_string());
    args.push(dunce::simplified(dest).to_string_lossy().into_owned());

    let (git_exe, git_args) = resolve_command("git", &args).map_err(|e| {
        err_resp(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("git unavailable: {e}"),
        )
    })?;
    let mut cmd = Command::new(git_exe);
    cmd.args(git_args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    prepare_isolated_command(&mut cmd);

    let mut child = cmd.spawn().map_err(|e| {
        err_resp(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to spawn git: {e}"),
        )
    })?;
    let _guard = ProcessTreeGuard::attach(&child);
    let mut stderr = child.stderr.take();
    let stderr_task = tokio::spawn(async move {
        let mut buf = Vec::new();
        if let Some(ref mut stream) = stderr {
            let mut take = stream.take(8192);
            let _ = take.read_to_end(&mut buf).await;
        }
        String::from_utf8_lossy(&buf).into_owned()
    });

    let wait_res = tokio::time::timeout(Duration::from_secs(60), child.wait()).await;
    let status = match wait_res {
        Ok(Ok(st)) => st,
        Ok(Err(e)) => {
            return Err(err_resp(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()));
        }
        Err(_) => {
            let _ = child.kill().await;
            return Err(err_resp(
                StatusCode::GATEWAY_TIMEOUT,
                "git clone timed out after 60 seconds.",
            ));
        }
    };
    let err_out = stderr_task.await.unwrap_or_default();
    if !status.success() {
        let msg = err_out.trim();
        return Err(err_resp(
            StatusCode::BAD_REQUEST,
            if msg.is_empty() {
                "git clone failed".to_string()
            } else {
                format!("git clone failed: {msg}")
            },
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::parse_git_skill_source;

    #[test]
    fn parses_github_tree_and_shorthand_urls() {
        let (url, br, sub) = parse_git_skill_source(
            "https://github.com/acme/agent-skills/tree/main/skills/rust-audit",
            None,
        )
        .unwrap();
        assert_eq!(url, "https://github.com/acme/agent-skills.git");
        assert_eq!(br.as_deref(), Some("main"));
        assert_eq!(sub.as_deref(), Some("skills/rust-audit"));

        let (url2, br2, sub2) = parse_git_skill_source("acme/my-skills", None).unwrap();
        assert_eq!(url2, "https://github.com/acme/my-skills.git");
        assert!(br2.is_none());
        assert!(sub2.is_none());

        assert!(parse_git_skill_source("-upload-pack=evil", None).is_err());
        assert!(parse_git_skill_source("acme/repo", Some("../escape")).is_err());
    }
}
