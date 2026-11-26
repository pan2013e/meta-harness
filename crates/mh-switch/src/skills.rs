use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde_json::json;
use tracing::warn;

use mh_core::{
    AppPaths, MAX_SKILL_FILE_BYTES, MessageKind, SenderKind, Session, SkillDetail, SkillSource,
    SkillSummary, compute_skill_tree_hash, is_path_within, normalize_path, resolve_skill_file,
    sanitize_skill_slug, scan_skill_dir,
};
use mh_store::{NewMessageParams, SkillStateRow, Store};

use crate::{SwitchEngine, SwitchError};

pub const BUILTIN_META_HARNESS_DOCS_ID: &str = "meta-harness-docs";

pub const BUILTIN_META_HARNESS_DOCS_FILES: &[(&str, &str)] = &[
    (
        "SKILL.md",
        include_str!("../../../skills/meta-harness-docs/SKILL.md"),
    ),
    (
        "references/usage-and-workflows.md",
        include_str!("../../../skills/meta-harness-docs/references/usage-and-workflows.md"),
    ),
    (
        "references/architecture-and-crates.md",
        include_str!("../../../skills/meta-harness-docs/references/architecture-and-crates.md"),
    ),
    (
        "references/protocols-and-invariants.md",
        include_str!("../../../skills/meta-harness-docs/references/protocols-and-invariants.md"),
    ),
    (
        "references/dev-testing-and-release.md",
        include_str!("../../../skills/meta-harness-docs/references/dev-testing-and-release.md"),
    ),
];

/// Materializes and upgrades the embedded `meta-harness-docs` skill inside `<data_dir>/skills/`
/// without overwriting user-edited content or disabling user preferences.
pub fn ensure_builtin_skills(paths: &AppPaths, store: &Store) {
    if let Err(err) = ensure_builtin_skills_inner(paths, store) {
        warn!("failed to synchronize built-in skills: {err}");
    }
}

fn ensure_builtin_skills_inner(paths: &AppPaths, store: &Store) -> Result<(), SwitchError> {
    let embedded_hash = compute_skill_tree_hash(BUILTIN_META_HARNESS_DOCS_FILES);
    let skill_dir = builtin_skill_dir(paths)?;
    let state = store.get_skill_state(BUILTIN_META_HARNESS_DOCS_ID)?;
    let now = chrono::Utc::now().timestamp_millis();

    if !skill_dir.join("SKILL.md").is_file() {
        write_embedded_skill_files(&skill_dir, BUILTIN_META_HARNESS_DOCS_FILES)?;
        store.upsert_skill_state(&SkillStateRow {
            state_key: BUILTIN_META_HARNESS_DOCS_ID.into(),
            source: SkillSource::Builtin,
            origin: Some("embedded".into()),
            enabled: state.as_ref().is_none_or(|s| s.enabled),
            builtin_hash: Some(embedded_hash),
            updated_at: now,
        })?;
        return Ok(());
    }

    let disk_hash = disk_skill_tree_hash(&skill_dir);
    let should_auto_upgrade = match (&state, &disk_hash) {
        (Some(row), Some(on_disk)) => {
            row.builtin_hash.as_deref() == Some(on_disk.as_str()) && on_disk != &embedded_hash
        }
        _ => false,
    };

    if should_auto_upgrade {
        write_embedded_skill_files(&skill_dir, BUILTIN_META_HARNESS_DOCS_FILES)?;
        store.upsert_skill_state(&SkillStateRow {
            state_key: BUILTIN_META_HARNESS_DOCS_ID.into(),
            source: SkillSource::Builtin,
            origin: Some("embedded".into()),
            enabled: state.as_ref().is_none_or(|s| s.enabled),
            builtin_hash: Some(embedded_hash),
            updated_at: now,
        })?;
    } else if state.is_none() {
        store.upsert_skill_state(&SkillStateRow {
            state_key: BUILTIN_META_HARNESS_DOCS_ID.into(),
            source: SkillSource::Builtin,
            origin: Some("embedded".into()),
            enabled: true,
            builtin_hash: Some(embedded_hash),
            updated_at: now,
        })?;
    }

    Ok(())
}

/// Restores a built-in skill directory cleanly to its embedded binary contents.
pub fn reset_builtin_skill(
    paths: &AppPaths,
    store: &Store,
    skill_id: &str,
) -> Result<SkillSummary, SwitchError> {
    if !skill_id.eq_ignore_ascii_case(BUILTIN_META_HARNESS_DOCS_ID) {
        return Err(SwitchError::NotFound(format!(
            "unknown built-in skill: {skill_id}"
        )));
    }
    let skill_dir = builtin_skill_dir(paths)?;
    if skill_dir.exists() {
        std::fs::remove_dir_all(&skill_dir)?;
    }
    write_embedded_skill_files(&skill_dir, BUILTIN_META_HARNESS_DOCS_FILES)?;
    let embedded_hash = compute_skill_tree_hash(BUILTIN_META_HARNESS_DOCS_FILES);
    let existing_enabled = store
        .get_skill_state(BUILTIN_META_HARNESS_DOCS_ID)?
        .is_none_or(|row| row.enabled);
    store.upsert_skill_state(&SkillStateRow {
        state_key: BUILTIN_META_HARNESS_DOCS_ID.into(),
        source: SkillSource::Builtin,
        origin: Some("embedded".into()),
        enabled: existing_enabled,
        builtin_hash: Some(embedded_hash),
        updated_at: chrono::Utc::now().timestamp_millis(),
    })?;

    let all = list_skills_with_roots(paths, store, None, None)?;
    all.into_iter()
        .find(|s| s.id == BUILTIN_META_HARNESS_DOCS_ID)
        .ok_or_else(|| SwitchError::NotFound("meta-harness-docs missing after reset".into()))
}

fn builtin_skill_dir(paths: &AppPaths) -> Result<PathBuf, SwitchError> {
    let root = paths.skills_dir();
    std::fs::create_dir_all(&root)?;
    let dir = root.join(BUILTIN_META_HARNESS_DOCS_ID);
    if !is_path_within(&dir, &root)
        || std::fs::symlink_metadata(&dir).is_ok_and(|meta| meta.file_type().is_symlink())
    {
        return Err(SwitchError::Validation(
            "Invalid built-in skill directory".into(),
        ));
    }
    Ok(dir)
}

fn write_embedded_skill_files(dir: &Path, files: &[(&str, &str)]) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    for (rel, _) in files {
        resolve_skill_file(dir, rel).map_err(std::io::Error::other)?;
    }
    for (rel, content) in files {
        let dest = dir.join(rel);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(dest, content)?;
    }
    Ok(())
}

pub fn disk_skill_tree_hash(dir: &Path) -> Option<String> {
    let (_, rels, _) = scan_skill_dir(dir, BUILTIN_META_HARNESS_DOCS_ID).ok()?;
    let mut contents = Vec::with_capacity(rels.len());
    for rel in &rels {
        let raw = std::fs::read_to_string(dir.join(rel)).ok()?;
        contents.push((rel.clone(), raw));
    }
    let borrowed: Vec<(&str, &str)> = contents
        .iter()
        .map(|(p, c)| (p.as_str(), c.as_str()))
        .collect();
    Some(compute_skill_tree_hash(&borrowed))
}

pub fn project_skill_state_key(project_root: &Path, slug: &str) -> String {
    let norm = normalize_path(project_root).unwrap_or_else(|_| project_root.to_path_buf());
    format!(
        "project:{}:{}",
        dunce::simplified(&norm)
            .to_string_lossy()
            .to_ascii_lowercase(),
        slug.to_ascii_lowercase()
    )
}

pub fn list_skills_with_roots(
    paths: &AppPaths,
    store: &Store,
    project_root: Option<&Path>,
    active_worktree: Option<&Path>,
) -> Result<Vec<SkillSummary>, SwitchError> {
    let states = store.list_skill_states()?;
    let embedded_hash = compute_skill_tree_hash(BUILTIN_META_HARNESS_DOCS_FILES);
    let mut skills: Vec<SkillSummary> = Vec::new();
    let mut seen_ids: HashSet<String> = HashSet::new();

    // 1. Global skills in `<MH_DATA_DIR>/skills/`
    let global_root = paths.skills_dir();
    if let Ok(rd) = std::fs::read_dir(&global_root) {
        let mut dirs: Vec<_> = rd.flatten().collect();
        dirs.sort_by_key(|e| e.file_name());
        for entry in dirs {
            let dir_path = entry.path();
            if !dir_path.is_dir() || !is_path_within(&dir_path, &global_root) {
                continue;
            }
            let dir_name = entry.file_name().to_string_lossy().into_owned();
            let Ok(id) = sanitize_skill_slug(&dir_name) else {
                continue;
            };
            if seen_ids.contains(&id) {
                continue;
            }
            let Ok((meta, files, updated_at)) = scan_skill_dir(&dir_path, &id) else {
                continue;
            };
            let state = states.get(&id);
            let source = if id == BUILTIN_META_HARNESS_DOCS_ID {
                SkillSource::Builtin
            } else {
                state.map(|s| s.source).unwrap_or(SkillSource::Local)
            };
            let builtin_modified = if source == SkillSource::Builtin {
                disk_skill_tree_hash(&dir_path)
                    .map(|h| h != embedded_hash)
                    .unwrap_or(false)
            } else {
                false
            };
            let norm_dir = normalize_path(&dir_path).unwrap_or(dir_path);
            let skill_dir = dunce::simplified(&norm_dir).to_string_lossy().into_owned();
            let skill_md_path = dunce::simplified(&norm_dir.join("SKILL.md"))
                .to_string_lossy()
                .into_owned();
            seen_ids.insert(id.clone());
            skills.push(SkillSummary {
                id,
                name: meta.name,
                description: meta.description,
                source,
                origin: state.and_then(|s| s.origin.clone()),
                enabled: state.map(|s| s.enabled).unwrap_or(true),
                builtin_modified,
                skill_dir,
                skill_md_path,
                files,
                updated_at: state
                    .map(|s| s.updated_at.max(updated_at))
                    .unwrap_or(updated_at),
            });
        }
    }

    // 2. Repo-local skills in `<active_worktree_or_project_root>/.agents/skills/`
    let scan_root = active_worktree
        .filter(|wt| wt.join(".agents").join("skills").is_dir())
        .or(project_root);
    let key_root = project_root.or(active_worktree);
    if let (Some(base), Some(key_base)) = (scan_root, key_root) {
        let local_skills_dir = base.join(".agents").join("skills");
        if is_path_within(&local_skills_dir, base)
            && let Ok(rd) = std::fs::read_dir(&local_skills_dir)
        {
            let mut dirs: Vec<_> = rd.flatten().collect();
            dirs.sort_by_key(|e| e.file_name());
            for entry in dirs {
                let dir_path = entry.path();
                if !dir_path.is_dir() || !is_path_within(&dir_path, &local_skills_dir) {
                    continue;
                }
                let dir_name = entry.file_name().to_string_lossy().into_owned();
                let Ok(id) = sanitize_skill_slug(&dir_name) else {
                    continue;
                };
                if seen_ids.contains(&id) {
                    continue;
                }
                let Ok((meta, files, updated_at)) = scan_skill_dir(&dir_path, &id) else {
                    continue;
                };
                let state_key = project_skill_state_key(key_base, &id);
                let state = states.get(&state_key);
                let norm_dir = normalize_path(&dir_path).unwrap_or(dir_path);
                let skill_dir = dunce::simplified(&norm_dir).to_string_lossy().into_owned();
                let skill_md_path = dunce::simplified(&norm_dir.join("SKILL.md"))
                    .to_string_lossy()
                    .into_owned();
                seen_ids.insert(id.clone());
                skills.push(SkillSummary {
                    id,
                    name: meta.name,
                    description: meta.description,
                    source: SkillSource::Project,
                    origin: Some(skill_dir.clone()),
                    enabled: state.map(|s| s.enabled).unwrap_or(true),
                    builtin_modified: false,
                    skill_dir,
                    skill_md_path,
                    files,
                    updated_at,
                });
            }
        }
    }

    skills.sort_by(|a, b| {
        let rank = |s: &SkillSummary| match s.source {
            SkillSource::Builtin => 0u8,
            SkillSource::Project => 1,
            _ => 2,
        };
        rank(a)
            .cmp(&rank(b))
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    Ok(skills)
}

pub fn read_skill_file_from_summary(
    skill: &SkillSummary,
    rel_path: Option<&str>,
) -> Result<SkillDetail, SwitchError> {
    let chosen_rel = rel_path
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("SKILL.md");
    let root = Path::new(&skill.skill_dir);
    let candidate = resolve_skill_file(root, chosen_rel).map_err(SwitchError::Validation)?;
    if !candidate.is_file() {
        return Err(SwitchError::NotFound(format!(
            "Skill file '{}' not found in '{}'",
            chosen_rel, skill.id
        )));
    }
    let meta = std::fs::metadata(&candidate)?;
    if meta.len() > MAX_SKILL_FILE_BYTES {
        return Err(SwitchError::Validation(
            "Skill file exceeds 512 KiB read limit.".into(),
        ));
    }
    let content = std::fs::read_to_string(&candidate)?;
    Ok(SkillDetail {
        skill: skill.clone(),
        file_path: chosen_rel.replace('\\', "/"),
        content,
    })
}

pub fn format_available_skills_xml(skills: &[&SkillSummary]) -> String {
    if skills.is_empty() {
        return String::new();
    }
    let mut out = String::from(
        "<available_skills>\n\
         Use `skill_read` (MCP tool) or read the listed `location` file to load a relevant skill's instructions or `references/` files before acting.\n",
    );
    for s in skills {
        let extra_files = s
            .files
            .iter()
            .filter(|f| f.as_str() != "SKILL.md")
            .take(8)
            .cloned()
            .collect::<Vec<_>>()
            .join(", ");
        out.push_str(&format!(
            "  <skill id=\"{}\" command=\"/{}\" name=\"{}\" location=\"{}\"",
            s.id,
            s.id,
            xml_escape(&s.name),
            xml_escape(&s.skill_md_path),
        ));
        if !extra_files.is_empty() {
            out.push_str(&format!(" files=\"{}\"", xml_escape(&extra_files)));
        }
        out.push_str(&format!(">{}</skill>\n", xml_escape(&s.description)));
    }
    out.push_str("</available_skills>\n");
    out
}

pub fn format_activated_skills_xml(activated: &[(SkillSummary, String)]) -> String {
    if activated.is_empty() {
        return String::new();
    }
    let mut out = String::from("<activated_skills>\n");
    for (skill, content) in activated {
        out.push_str(&format!(
            "  <skill id=\"{}\" name=\"{}\" dir=\"{}\">\n{}\n  </skill>\n",
            skill.id,
            xml_escape(&skill.name),
            xml_escape(&skill.skill_dir),
            content.trim()
        ));
    }
    out.push_str("</activated_skills>\n");
    out
}

fn xml_escape(raw: &str) -> String {
    raw.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

impl SwitchEngine {
    pub fn list_skills_for_project(
        &self,
        project_root: Option<&Path>,
        active_worktree: Option<&Path>,
    ) -> Result<Vec<SkillSummary>, SwitchError> {
        list_skills_with_roots(&self.app_paths, &self.store, project_root, active_worktree)
    }

    pub fn list_session_skills(
        &self,
        session: &Session,
        worktree_override: Option<&Path>,
    ) -> Result<Vec<SkillSummary>, SwitchError> {
        let project_root = if session.project_id.is_some() {
            self.resolve_session_repo_root(session).ok()
        } else {
            None
        };
        let wt = worktree_override.or_else(|| {
            session
                .integration_wt_path
                .as_deref()
                .map(Path::new)
                .filter(|p| p.exists())
        });
        list_skills_with_roots(&self.app_paths, &self.store, project_root.as_deref(), wt)
    }

    pub fn resolve_enabled_skill_read(
        &self,
        session: &Session,
        worktree_override: Option<&Path>,
        skill_id: &str,
        rel_path: Option<&str>,
    ) -> Result<SkillDetail, SwitchError> {
        let slug = skill_id.trim().trim_start_matches('/').to_ascii_lowercase();
        let skills = self.list_session_skills(session, worktree_override)?;
        let Some(skill) = skills.into_iter().find(|s| s.id == slug) else {
            return Err(SwitchError::NotFound(format!("unknown skill: {skill_id}")));
        };
        if !skill.enabled {
            return Err(SwitchError::Validation(format!(
                "skill '/{}' is currently disabled in Settings -> Skills",
                skill.id
            )));
        }
        read_skill_file_from_summary(&skill, rel_path)
    }

    pub async fn emit_skill_activity_pill(
        &self,
        session_id: &str,
        member_handle: &str,
        turn_id: Option<&str>,
        chain_id: Option<&str>,
        skill: &SkillSummary,
    ) {
        let rt = self.runtime(session_id).await;
        let dedupe_scope = turn_id
            .map(str::to_string)
            .or_else(|| chain_id.map(|c| format!("{c}:{member_handle}")))
            .unwrap_or_else(|| member_handle.to_string());
        if !rt
            .recorded_skill_uses
            .lock()
            .await
            .insert((dedupe_scope, skill.id.clone()))
        {
            return;
        }

        let resolved_chain = match chain_id {
            Some(c) => c.to_string(),
            None => {
                self.active_chain_for_member(session_id, member_handle)
                    .await
            }
        };
        if let Ok(pill) = self.store.append_message(NewMessageParams {
            id: None,
            session_id: session_id.to_string(),
            chain_id: resolved_chain,
            parent_msg_id: None,
            sender_kind: SenderKind::System,
            sender_id: member_handle.to_string(),
            recipients: Vec::new(),
            kind: MessageKind::ActivityPill,
            content: format!("@{member_handle} used skill {}", skill.name),
            turn_id: turn_id.map(str::to_string),
            metadata: json!({
                "member": member_handle,
                "skill_id": skill.id,
                "skill_name": skill.name,
            }),
        }) {
            let _ = self.events_tx.send(mh_core::WsEvent::SwitchActivityPill {
                session_id: session_id.to_string(),
                message: pill,
            });
        }
    }

    pub async fn emit_skill_activity_for_path(
        &self,
        session_id: &str,
        member_handle: &str,
        turn_id: Option<&str>,
        target: &Path,
    ) {
        let Ok(session) = self.store.get_session(session_id) else {
            return;
        };
        let wt_override: Option<PathBuf> = self
            .store
            .get_member(session_id, member_handle)
            .ok()
            .and_then(|m| m.worktree_path)
            .map(PathBuf::from);
        let Ok(skills) = self.list_session_skills(&session, wt_override.as_deref()) else {
            return;
        };
        if let Some(matched) = skills
            .into_iter()
            .find(|s| s.enabled && is_path_within(target, Path::new(&s.skill_dir)))
        {
            self.emit_skill_activity_pill(session_id, member_handle, turn_id, None, &matched)
                .await;
        }
    }
}

pub fn session_skill_read_roots(skills: &[SkillSummary]) -> Vec<PathBuf> {
    skills
        .iter()
        .filter(|s| s.enabled)
        .map(|s| PathBuf::from(&s.skill_dir))
        .collect()
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    #[test]
    fn skill_discovery_stays_inside_the_configured_roots() {
        let tmp = tempfile::tempdir().unwrap();
        let project = tmp.path().join("project");
        let outside = tmp.path().join("elsewhere");
        let local_root = project.join(".agents/skills");
        let paths = AppPaths {
            config_dir: tmp.path().join("config"),
            data_dir: tmp.path().join("data"),
            workspaces_dir: tmp.path().join("workspaces"),
        };
        let store = Store::open_in_memory().unwrap();
        std::fs::create_dir_all(local_root.join("linked-file")).unwrap();
        std::fs::create_dir_all(local_root.join("local-skill")).unwrap();
        std::fs::create_dir_all(paths.skills_dir()).unwrap();
        std::fs::create_dir(&outside).unwrap();
        std::fs::write(
            outside.join("SKILL.md"),
            "# A file outside the skills roots",
        )
        .unwrap();
        std::fs::write(local_root.join("local-skill/SKILL.md"), "# Local skill").unwrap();
        symlink(&outside, local_root.join("linked-directory")).unwrap();
        symlink(&outside, paths.skills_dir().join("external-skill")).unwrap();
        symlink(
            outside.join("SKILL.md"),
            local_root.join("linked-file/SKILL.md"),
        )
        .unwrap();

        let skills = list_skills_with_roots(&paths, &store, Some(&project), None).unwrap();
        assert_eq!(
            skills
                .iter()
                .map(|skill| skill.id.as_str())
                .collect::<Vec<_>>(),
            ["local-skill"]
        );
    }
}
