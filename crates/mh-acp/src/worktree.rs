use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use mh_core::{DiffStat, PromoteMode};

use crate::AcpError;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoStatus {
    pub is_git_repo: bool,
    pub head_branch: Option<String>,
    pub head_sha: Option<String>,
    pub is_dirty: bool,
}

#[derive(Debug, Clone)]
pub struct IntegrationWorktree {
    pub branch: String,
    pub path: PathBuf,
    pub base_branch: Option<String>,
    pub base_commit_sha: String,
}

#[derive(Debug, Clone)]
pub struct MemberWorktree {
    pub branch: String,
    pub path: PathBuf,
}

#[derive(Debug, Clone)]
pub enum MergeOutcome {
    NoChanges,
    Clean {
        merged_sha: String,
        diff_stat: DiffStat,
        touched_files: Vec<String>,
    },
    Conflict {
        conflicting_files: Vec<String>,
        diff_preview: String,
        member_branch: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionDiffReport {
    pub integration_branch: String,
    pub base_branch: String,
    pub ahead_commits: u32,
    pub diff_stat: DiffStat,
    pub unified_diff: String,
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromoteResult {
    pub mode: PromoteMode,
    pub target_branch: String,
    pub summary: String,
    pub new_head_sha: Option<String>,
}

pub fn sanitize_ref_component(raw: &str) -> String {
    let s: String = raw
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();
    if s.is_empty() {
        "member".to_string()
    } else {
        s
    }
}

async fn git_output(dir: &Path, args: &[&str]) -> Result<std::process::Output, AcpError> {
    Ok(crate::platform::background_command("git")
        .arg("-C")
        .arg(dunce::simplified(dir))
        .args(args)
        .output()
        .await?)
}

async fn run_git(dir: &Path, args: &[&str]) -> Result<(bool, String, String), AcpError> {
    let output = git_output(dir, args).await?;
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    Ok((output.status.success(), stdout, stderr))
}

async fn run_git_with_author(
    dir: &Path,
    args: &[&str],
) -> Result<(bool, String, String), AcpError> {
    let dir_clean = dunce::simplified(dir);
    let mut cmd = crate::platform::background_command("git");
    cmd.arg("-C").arg(dir_clean).args(args);

    // Ensure automated commits succeed even if user hasn't set global git user.name/email in CI
    let has_name = run_git(dir, &["config", "user.name"])
        .await
        .map(|(ok, out, _)| ok && !out.is_empty())
        .unwrap_or(false);
    let has_email = run_git(dir, &["config", "user.email"])
        .await
        .map(|(ok, out, _)| ok && !out.is_empty())
        .unwrap_or(false);

    if !has_name {
        cmd.env("GIT_AUTHOR_NAME", "Meta-Harness")
            .env("GIT_COMMITTER_NAME", "Meta-Harness");
    }
    if !has_email {
        cmd.env("GIT_AUTHOR_EMAIL", "meta-harness@local")
            .env("GIT_COMMITTER_EMAIL", "meta-harness@local");
    }

    let output = cmd.output().await?;
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    Ok((output.status.success(), stdout, stderr))
}

pub async fn inspect_repo(project_root: &Path) -> RepoStatus {
    let Ok((inside_ok, inside_out, _)) =
        run_git(project_root, &["rev-parse", "--is-inside-work-tree"]).await
    else {
        return RepoStatus {
            is_git_repo: false,
            head_branch: None,
            head_sha: None,
            is_dirty: false,
        };
    };

    if !inside_ok || inside_out != "true" {
        return RepoStatus {
            is_git_repo: false,
            head_branch: None,
            head_sha: None,
            is_dirty: false,
        };
    }

    let head_branch = run_git(project_root, &["rev-parse", "--abbrev-ref", "HEAD"])
        .await
        .ok()
        .filter(|(ok, s, _)| *ok && !s.is_empty() && s != "HEAD")
        .map(|(_, s, _)| s);

    let head_sha = run_git(project_root, &["rev-parse", "HEAD"])
        .await
        .ok()
        .filter(|(ok, s, _)| *ok && !s.is_empty())
        .map(|(_, s, _)| s);

    let is_dirty = run_git(project_root, &["status", "--porcelain"])
        .await
        .map(|(ok, s, _)| ok && !s.is_empty())
        .unwrap_or(false);

    RepoStatus {
        is_git_repo: true,
        head_branch,
        head_sha,
        is_dirty,
    }
}

pub async fn init_git_repo(project_root: &Path) -> Result<RepoStatus, AcpError> {
    let (ok, _, err) = run_git(project_root, &["init", "-b", "main"]).await?;
    if !ok {
        let (ok2, _, err2) = run_git(project_root, &["init"]).await?;
        if !ok2 {
            return Err(AcpError::Git(format!("git init failed: {err} / {err2}")));
        }
        let _ = run_git(project_root, &["checkout", "-b", "main"]).await;
    }

    // Ensure `.meta-harness/` is excluded before committing
    ensure_git_exclude(project_root).await?;

    let status = inspect_repo(project_root).await;
    if status.head_sha.is_none() {
        let _ = run_git(project_root, &["add", "-A"]).await;
        let (c_ok, _, c_err) = run_git_with_author(
            project_root,
            &[
                "commit",
                "--allow-empty",
                "-m",
                "chore: initialize repository",
            ],
        )
        .await?;
        if !c_ok {
            return Err(AcpError::Git(format!("initial commit failed: {c_err}")));
        }
    }

    Ok(inspect_repo(project_root).await)
}

async fn ensure_git_exclude(project_root: &Path) -> Result<(), AcpError> {
    let (ok, common_dir_str, err) =
        run_git(project_root, &["rev-parse", "--git-common-dir"]).await?;
    if !ok {
        return Err(AcpError::Git(format!("git-common-dir failed: {err}")));
    }
    let common_dir = if Path::new(&common_dir_str).is_absolute() {
        PathBuf::from(common_dir_str)
    } else {
        project_root.join(common_dir_str)
    };
    let info_dir = common_dir.join("info");
    tokio::fs::create_dir_all(&info_dir).await?;
    let exclude_path = info_dir.join("exclude");

    let existing = tokio::fs::read_to_string(&exclude_path)
        .await
        .unwrap_or_default();
    if !existing.lines().any(|l| l.trim() == ".meta-harness/") {
        let mut updated = existing;
        if !updated.is_empty() && !updated.ends_with('\n') {
            updated.push('\n');
        }
        updated.push_str(".meta-harness/\n");
        tokio::fs::write(&exclude_path, updated).await?;
    }
    Ok(())
}

pub async fn ensure_session_integration_worktree(
    project_root: &Path,
    session_id: &str,
) -> Result<IntegrationWorktree, AcpError> {
    let status = inspect_repo(project_root).await;
    if !status.is_git_repo {
        return Err(AcpError::Git(
            "project is not a git repository; initialize git or use shared folder mode".into(),
        ));
    }
    let base_sha = match status.head_sha {
        Some(sha) => sha,
        None => {
            let (ok, _, err) = run_git_with_author(
                project_root,
                &[
                    "commit",
                    "--allow-empty",
                    "-m",
                    "chore: initial empty base commit",
                ],
            )
            .await?;
            if !ok {
                return Err(AcpError::Git(format!("failed to create base HEAD: {err}")));
            }
            inspect_repo(project_root)
                .await
                .head_sha
                .ok_or_else(|| AcpError::Git("repository still has no HEAD commit".into()))?
        }
    };

    ensure_git_exclude(project_root).await?;

    let sid = sanitize_ref_component(session_id);
    let branch = format!("mh/{sid}/integration");
    let wt_path = project_root
        .join(".meta-harness")
        .join("worktrees")
        .join(&sid)
        .join("_integration");

    if !wt_path.join(".git").exists() {
        if let Some(parent) = wt_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let wt_str = dunce::simplified(&wt_path).to_string_lossy().into_owned();
        let branch_ref = format!("refs/heads/{branch}");
        let branch_exists = run_git(project_root, &["rev-parse", "--verify", &branch_ref])
            .await
            .is_ok_and(|(ok, _, _)| ok);
        let (ok, _, err) = if branch_exists {
            run_git(project_root, &["worktree", "add", &wt_str, &branch]).await?
        } else {
            run_git(
                project_root,
                &["worktree", "add", "-B", &branch, &wt_str, &base_sha],
            )
            .await?
        };
        if !ok {
            return Err(AcpError::Git(format!(
                "failed to create integration worktree: {err}"
            )));
        }
    }

    Ok(IntegrationWorktree {
        branch,
        path: dunce::canonicalize(&wt_path).unwrap_or(wt_path),
        base_branch: status.head_branch,
        base_commit_sha: base_sha,
    })
}

fn session_worktree_dir(project_root: &Path, session_id: &str) -> PathBuf {
    project_root
        .join(".meta-harness")
        .join("worktrees")
        .join(sanitize_ref_component(session_id))
}

pub fn member_worktree_path(project_root: &Path, session_id: &str, handle: &str) -> PathBuf {
    session_worktree_dir(project_root, session_id)
        .join(format!("_exec_{}", sanitize_ref_component(handle)))
}

pub async fn ensure_member_worktree(
    project_root: &Path,
    session_id: &str,
    integration_branch: &str,
    handle: &str,
) -> Result<MemberWorktree, AcpError> {
    let sid = sanitize_ref_component(session_id);
    let h = sanitize_ref_component(handle);
    let branch = format!("mh/{sid}/exec-{h}");
    let wt_path = member_worktree_path(project_root, session_id, handle);

    if !wt_path.join(".git").exists() {
        if let Some(parent) = wt_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let wt_str = dunce::simplified(&wt_path).to_string_lossy().into_owned();
        let branch_ref = format!("refs/heads/{branch}");
        let (branch_exists, _, _) = run_git(
            project_root,
            &["show-ref", "--verify", "--quiet", &branch_ref],
        )
        .await?;
        let (ok, _, err) = if branch_exists {
            run_git(project_root, &["worktree", "add", &wt_str, &branch]).await?
        } else {
            run_git(
                project_root,
                &[
                    "worktree",
                    "add",
                    "-b",
                    &branch,
                    &wt_str,
                    integration_branch,
                ],
            )
            .await?
        };
        if !ok {
            return Err(AcpError::Git(format!(
                "failed to create member worktree for {handle}: {err}"
            )));
        }
    } else {
        let _ = sync_member_worktree(&wt_path, integration_branch).await;
    }

    Ok(MemberWorktree {
        branch,
        path: dunce::canonicalize(&wt_path).unwrap_or(wt_path),
    })
}

pub async fn sync_member_worktree(
    member_wt: &Path,
    integration_branch: &str,
) -> Result<(), AcpError> {
    let (ok, dirty, _) = run_git(member_wt, &["status", "--porcelain"]).await?;
    if !ok || !dirty.is_empty() {
        return Ok(());
    }
    let (ff_ok, _, _) = run_git(member_wt, &["merge", "--ff-only", integration_branch]).await?;
    if !ff_ok {
        let (m_ok, _, _) =
            run_git_with_author(member_wt, &["merge", "--no-edit", integration_branch]).await?;
        if !m_ok {
            let _ = run_git(member_wt, &["merge", "--abort"]).await;
        }
    }
    Ok(())
}

/// Auto-commits uncommitted edits in `member_wt` onto `member_branch` and attempts a
/// zero-token Git merge into `integration_wt`.
pub async fn integrate_member_turn(
    integration_wt: &Path,
    integration_branch: &str,
    member_wt: &Path,
    member_branch: &str,
    handle: &str,
    summary: &str,
    keep_conflict_for_orchestrator: bool,
) -> Result<MergeOutcome, AcpError> {
    let (_, dirty, _) = run_git(member_wt, &["status", "--porcelain"]).await?;
    if !dirty.is_empty() {
        let (add_ok, _, add_err) = run_git(member_wt, &["add", "-A"]).await?;
        if !add_ok {
            return Err(AcpError::Git(format!("git add failed: {add_err}")));
        }
        let short_sum: String = summary
            .lines()
            .next()
            .unwrap_or("worktree updates")
            .chars()
            .take(64)
            .collect();
        let commit_msg = if short_sum.trim().is_empty() {
            format!("mh({handle}): turn updates")
        } else {
            format!("mh({handle}): {short_sum}")
        };
        let (commit_ok, _, commit_err) =
            run_git_with_author(member_wt, &["commit", "-m", &commit_msg]).await?;
        if !commit_ok && !commit_err.contains("nothing to commit") {
            return Err(AcpError::Git(format!("git commit failed: {commit_err}")));
        }
    }

    let range = format!("{integration_branch}..{member_branch}");
    let (cnt_ok, cnt_out, _) = run_git(integration_wt, &["rev-list", "--count", &range]).await?;
    let ahead: u32 = if cnt_ok {
        cnt_out.parse().unwrap_or(0)
    } else {
        0
    };
    if ahead == 0 {
        return Ok(MergeOutcome::NoChanges);
    }

    let (diff_stat, touched_files) =
        compute_numstat_between(integration_wt, integration_branch, member_branch).await;

    let (merge_ok, _, merge_err) =
        run_git_with_author(integration_wt, &["merge", "--no-edit", member_branch]).await?;

    if merge_ok {
        let (_, merged_sha, _) = run_git(integration_wt, &["rev-parse", "HEAD"]).await?;
        return Ok(MergeOutcome::Clean {
            merged_sha,
            diff_stat,
            touched_files,
        });
    }

    let (_, conflict_out, _) =
        run_git(integration_wt, &["diff", "--name-only", "--diff-filter=U"]).await?;
    let conflicting_files: Vec<String> = conflict_out
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect();

    if conflicting_files.is_empty() {
        // Non-conflict merge failure (e.g., dirty integration worktree)
        let _ = run_git(integration_wt, &["merge", "--abort"]).await;
        return Err(AcpError::Git(format!("merge failed: {merge_err}")));
    }

    let (_, diff_raw, _) = run_git(integration_wt, &["diff"]).await?;
    let diff_preview: String = diff_raw.chars().take(12_000).collect();

    if !keep_conflict_for_orchestrator {
        let _ = run_git(integration_wt, &["merge", "--abort"]).await;
    }

    Ok(MergeOutcome::Conflict {
        conflicting_files,
        diff_preview,
        member_branch: member_branch.to_string(),
    })
}

pub async fn get_head_sha(wt_path: &Path) -> Option<String> {
    run_git(wt_path, &["rev-parse", "HEAD"])
        .await
        .ok()
        .filter(|(ok, s, _)| *ok && !s.is_empty())
        .map(|(_, s, _)| s)
}

pub async fn compute_numstat_between(
    repo_dir: &Path,
    from_ref: &str,
    to_ref: &str,
) -> (DiffStat, Vec<String>) {
    let range = format!("{from_ref}..{to_ref}");
    let Ok((ok, out, _)) = run_git(repo_dir, &["diff", "--numstat", &range]).await else {
        return (DiffStat::default(), Vec::new());
    };
    if !ok {
        return (DiffStat::default(), Vec::new());
    }
    parse_numstat_lines(&out)
}

/// Auto-commits any uncommitted changes in `wt_path` and returns the resulting commit SHA,
/// cumulative diff stats against the prior HEAD, and list of touched files.
pub async fn commit_worktree_if_dirty(
    wt_path: &Path,
    handle: &str,
    summary: &str,
) -> Result<Option<(String, DiffStat, Vec<String>)>, AcpError> {
    let (ok, dirty, err) = run_git(wt_path, &["status", "--porcelain"]).await?;
    if !ok {
        return Err(AcpError::Git(format!(
            "git status failed in {}: {err}",
            wt_path.display()
        )));
    }
    if dirty.is_empty() {
        return Ok(None);
    }

    let _ = ensure_git_exclude(wt_path).await;
    let prev_sha = get_head_sha(wt_path).await;

    let (add_ok, _, add_err) = run_git(wt_path, &["add", "-A"]).await?;
    if !add_ok {
        return Err(AcpError::Git(format!("git add failed: {add_err}")));
    }

    let short_sum: String = summary
        .lines()
        .next()
        .unwrap_or("worktree updates")
        .chars()
        .take(64)
        .collect();
    let commit_msg = if short_sum.trim().is_empty() {
        format!("mh({handle}): turn updates")
    } else {
        format!("mh({handle}): {short_sum}")
    };

    let (commit_ok, _, commit_err) =
        run_git_with_author(wt_path, &["commit", "-m", &commit_msg]).await?;
    if !commit_ok {
        if commit_err.contains("nothing to commit") {
            return Ok(None);
        }
        return Err(AcpError::Git(format!("git commit failed: {commit_err}")));
    }

    let new_sha = get_head_sha(wt_path).await.unwrap_or_default();
    let (stat, files) = if let Some(prev) = prev_sha.as_deref()
        && !new_sha.is_empty()
    {
        compute_numstat_between(wt_path, prev, &new_sha).await
    } else {
        (DiffStat::default(), Vec::new())
    };

    Ok(Some((new_sha, stat, files)))
}

/// Saves every checkout before archiving, including shells opened without an ACP turn.
pub async fn checkpoint_session_worktrees(
    project_root: &Path,
    session_id: &str,
    title: &str,
) -> Result<(), AcpError> {
    let mut entries =
        match tokio::fs::read_dir(session_worktree_dir(project_root, session_id)).await {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error.into()),
        };
    while let Some(entry) = entries.next_entry().await? {
        if entry.file_type().await?.is_dir() && entry.path().join(".git").is_file() {
            commit_worktree_if_dirty(&entry.path(), "archive", title).await?;
        }
    }
    Ok(())
}

/// Resets an integration worktree to match a newly promoted commit SHA so subsequent
/// session diffs measure cleanly from the promoted state.
pub async fn reset_worktree_to_sha(wt_path: &Path, sha: &str) -> Result<(), AcpError> {
    let (ok, _, err) = run_git(wt_path, &["reset", "--hard", sha]).await?;
    if !ok {
        return Err(AcpError::Git(format!(
            "failed to reset worktree to {sha}: {err}"
        )));
    }
    Ok(())
}

pub async fn get_session_diff(
    project_root: &Path,
    integration_wt: &Path,
    base_branch: Option<&str>,
    base_commit_sha: Option<&str>,
    integration_branch: &str,
) -> Result<SessionDiffReport, AcpError> {
    // Keep the review aligned with the session's recorded baseline and patch promotion.
    let resolved_base_sha = if let Some(sha) = base_commit_sha {
        sha.to_string()
    } else if let Some(br) = base_branch {
        match run_git(project_root, &["rev-parse", "--verify", br]).await {
            Ok((true, sha, _)) if !sha.is_empty() => sha,
            _ => "HEAD".to_string(),
        }
    } else {
        "HEAD".to_string()
    };

    let range = format!("{resolved_base_sha}..{integration_branch}");
    let (cnt_ok, cnt_out, _) = run_git(integration_wt, &["rev-list", "--count", &range]).await?;
    let ahead_commits: u32 = if cnt_ok {
        cnt_out.parse().unwrap_or(0)
    } else {
        0
    };

    let (raw_diff, diff_stat) = diff_worktree_snapshot(integration_wt, &resolved_base_sha).await?;

    const MAX_DIFF_BYTES: usize = 256 * 1024;
    let truncated = raw_diff.len() > MAX_DIFF_BYTES;
    let unified_diff = if truncated {
        raw_diff[..raw_diff.floor_char_boundary(MAX_DIFF_BYTES)].to_string()
    } else {
        raw_diff
    };

    Ok(SessionDiffReport {
        integration_branch: integration_branch.to_string(),
        base_branch: base_branch.unwrap_or(&resolved_base_sha).to_string(),
        ahead_commits,
        diff_stat,
        unified_diff,
        truncated,
    })
}

async fn diff_worktree_snapshot(
    worktree: &Path,
    base: &str,
) -> Result<(String, DiffStat), AcpError> {
    let untracked = checked_diff_git(
        worktree,
        None,
        &["ls-files", "--others", "--exclude-standard", "-z"],
    )
    .await?;
    let alternate = if untracked.is_empty() {
        None
    } else {
        Some(tempfile::tempdir()?)
    };
    let snapshot = alternate.as_ref().map(tempfile::TempDir::path);
    let mut args = vec!["diff", "--no-ext-diff"];
    if let Some(path) = snapshot {
        let source_objects = checked_diff_git(
            worktree,
            None,
            &[
                "rev-parse",
                "--path-format=absolute",
                "--git-path",
                "objects",
            ],
        )
        .await?;
        let info = path.join("objects/info");
        tokio::fs::create_dir_all(&info).await?;
        tokio::fs::write(info.join("alternates"), source_objects).await?;
        // Borrow existing Git objects and keep the snapshot's new objects in the temporary directory.
        checked_diff_git(worktree, Some(path), &["read-tree", "HEAD"]).await?;
        checked_diff_git(worktree, Some(path), &["add", "-A", "--"]).await?;
        args.push("--cached");
    }
    let mut stat_args = args.clone();
    args.extend([base, "--"]);
    stat_args.extend(["--numstat", base, "--"]);
    let (patch, stat) = tokio::try_join!(
        checked_diff_git(worktree, snapshot, &args),
        checked_diff_git(worktree, snapshot, &stat_args),
    )?;
    Ok((patch, parse_numstat_lines(&stat).0))
}

async fn checked_diff_git(
    worktree: &Path,
    snapshot: Option<&Path>,
    args: &[&str],
) -> Result<String, AcpError> {
    let mut command = crate::platform::background_command("git");
    command
        .arg("-C")
        .arg(dunce::simplified(worktree))
        .env("GIT_OPTIONAL_LOCKS", "0")
        .args(args);
    if let Some(path) = snapshot {
        command.env("GIT_INDEX_FILE", dunce::simplified(&path.join("index")));
        command.env(
            "GIT_OBJECT_DIRECTORY",
            dunce::simplified(&path.join("objects")),
        );
    }
    let out = command.output().await?;
    if !out.status.success() {
        return Err(AcpError::Git(
            String::from_utf8_lossy(&out.stderr).into_owned(),
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

fn parse_numstat_lines(out: &str) -> (DiffStat, Vec<String>) {
    let mut stat = DiffStat::default();
    let mut files = Vec::new();
    for line in out.lines() {
        let parts: Vec<&str> = line.split('\t').collect();
        if parts.len() == 3 {
            let ins = parts[0].parse::<u32>().unwrap_or(0);
            let del = parts[1].parse::<u32>().unwrap_or(0);
            stat.files_changed += 1;
            stat.insertions += ins;
            stat.deletions += del;
            files.push(parts[2].to_string());
        }
    }
    (stat, files)
}

pub async fn promote_session(
    project_root: &Path,
    base_branch: &str,
    base_commit_sha: &str,
    integration_branch: &str,
    mode: PromoteMode,
) -> Result<PromoteResult, AcpError> {
    let status = inspect_repo(project_root).await;
    if !status.is_git_repo {
        return Err(AcpError::Git("project is not a git repository".into()));
    }
    if status.is_dirty {
        return Err(AcpError::Git(
            "main checkout has uncommitted changes; commit or stash them before promoting session changes".into(),
        ));
    }

    if status.head_branch.as_deref() != Some(base_branch) {
        let current = status.head_branch.as_deref().unwrap_or("detached HEAD");
        return Err(AcpError::Git(format!(
            "check out the session base branch '{base_branch}' before promoting (currently on '{current}')"
        )));
    }
    let target_branch = base_branch.to_string();

    match mode {
        PromoteMode::Merge => {
            let (ok, _, err) =
                run_git_with_author(project_root, &["merge", "--no-edit", integration_branch])
                    .await?;
            if !ok {
                let _ = run_git(project_root, &["merge", "--abort"]).await;
                return Err(AcpError::Git(format!("merge promotion failed: {err}")));
            }
            let sha = get_head_sha(project_root).await;
            Ok(PromoteResult {
                mode,
                target_branch: target_branch.clone(),
                summary: format!("Merged {integration_branch} into {target_branch}"),
                new_head_sha: sha,
            })
        }
        PromoteMode::Squash => {
            let (ok, _, err) =
                run_git(project_root, &["merge", "--squash", integration_branch]).await?;
            if !ok {
                let _ = run_git(project_root, &["reset", "--merge"]).await;
                return Err(AcpError::Git(format!("squash merge failed: {err}")));
            }
            let (unchanged, _, _) = run_git(project_root, &["diff", "--cached", "--quiet"]).await?;
            if unchanged {
                return Ok(PromoteResult {
                    mode,
                    target_branch,
                    summary: "No changes to squash".into(),
                    new_head_sha: get_head_sha(project_root).await,
                });
            }
            let msg = format!("feat(meta-harness): squash session {integration_branch}");
            let (c_ok, _, c_err) =
                run_git_with_author(project_root, &["commit", "-m", &msg]).await?;
            if !c_ok {
                let _ = run_git(project_root, &["reset", "--merge"]).await;
                return Err(AcpError::Git(format!("squash commit failed: {c_err}")));
            }
            let sha = get_head_sha(project_root).await;
            Ok(PromoteResult {
                mode,
                target_branch: target_branch.clone(),
                summary: format!("Squash-merged {integration_branch} into {target_branch}"),
                new_head_sha: sha,
            })
        }
        PromoteMode::Patch => {
            let range = format!("{base_commit_sha}..{integration_branch}");
            let diff_out = git_output(project_root, &["diff", "--binary", &range, "--"]).await?;
            if !diff_out.status.success() {
                return Err(AcpError::Git(
                    String::from_utf8_lossy(&diff_out.stderr).into_owned(),
                ));
            }
            if diff_out.stdout.is_empty() {
                return Ok(PromoteResult {
                    mode,
                    target_branch,
                    summary: "No patch changes to apply".into(),
                    new_head_sha: get_head_sha(project_root).await,
                });
            }

            let mut apply = crate::platform::background_command("git")
                .arg("-C")
                .arg(dunce::simplified(project_root))
                .args(["apply", "-"])
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()?;
            let mut stdin = apply
                .stdin
                .take()
                .ok_or_else(|| AcpError::Git("git apply is missing stdin".into()))?;
            let (written, apply_res) = tokio::join!(
                async move {
                    use tokio::io::AsyncWriteExt;
                    stdin.write_all(&diff_out.stdout).await
                },
                apply.wait_with_output()
            );
            let apply_res = apply_res?;
            if !apply_res.status.success() {
                return Err(AcpError::Git(format!(
                    "git apply failed: {}",
                    String::from_utf8_lossy(&apply_res.stderr)
                )));
            }
            written?;
            Ok(PromoteResult {
                mode,
                target_branch,
                summary: format!("Applied uncommitted patch from {integration_branch}"),
                new_head_sha: None,
            })
        }
    }
}

/// Removes `.meta-harness/worktrees/<sid>` checkouts on disk while preserving
/// session Git branches (`refs/heads/mh/<sid>/*`) so archived sessions can be restored.
pub async fn prune_session_worktree_checkouts(
    project_root: &Path,
    session_id: &str,
) -> Result<(), AcpError> {
    let status = inspect_repo(project_root).await;
    if !status.is_git_repo {
        return Ok(());
    }

    let session_wt_dir = session_worktree_dir(project_root, session_id);

    if session_wt_dir.exists() {
        for attempt in 0..6 {
            if let Ok(mut rd) = tokio::fs::read_dir(&session_wt_dir).await {
                while let Ok(Some(entry)) = rd.next_entry().await {
                    let p = entry.path();
                    if entry.file_type().await?.is_dir() && p.join(".git").is_file() {
                        let p_str = dunce::simplified(&p).to_string_lossy().into_owned();
                        let _ =
                            run_git(project_root, &["worktree", "remove", "--force", &p_str]).await;
                    }
                }
            }
            if tokio::fs::remove_dir(&session_wt_dir).await.is_ok() || !session_wt_dir.exists() {
                break;
            }
            if attempt < 5 {
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
        }
    }

    if session_wt_dir.exists() {
        return Err(AcpError::Git(format!(
            "could not remove all session worktrees under {}",
            session_wt_dir.display()
        )));
    }

    let _ = run_git(project_root, &["worktree", "prune"]).await;
    Ok(())
}

pub async fn prune_session_worktrees(
    project_root: &Path,
    session_id: &str,
) -> Result<(), AcpError> {
    prune_session_worktree_checkouts(project_root, session_id).await?;
    let status = inspect_repo(project_root).await;
    if !status.is_git_repo {
        return Ok(());
    }

    let sid = sanitize_ref_component(session_id);
    let ref_prefix = format!("refs/heads/mh/{sid}/");
    if let Ok((ok, branches, _)) = run_git(
        project_root,
        &["for-each-ref", "--format=%(refname:short)", &ref_prefix],
    )
    .await
        && ok
    {
        for br in branches.lines().map(str::trim).filter(|b| !b.is_empty()) {
            let _ = run_git(project_root, &["branch", "-D", br]).await;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn diff_includes_untracked_work_and_excludes_later_main_commits() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        init_git_repo(root).await.unwrap();
        let int = ensure_session_integration_worktree(root, "review")
            .await
            .unwrap();
        std::fs::write(root.join("upstream.txt"), "base advanced\n").unwrap();
        commit_worktree_if_dirty(root, "test", "Advance main")
            .await
            .unwrap();
        std::fs::write(int.path.join("draft.txt"), "session work\n").unwrap();
        let objects_before = run_git(root, &["count-objects"]).await.unwrap().1;

        let diff = get_session_diff(
            root,
            &int.path,
            int.base_branch.as_deref(),
            Some(&int.base_commit_sha),
            &int.branch,
        )
        .await
        .unwrap();
        assert_eq!(diff.diff_stat.files_changed, 1);
        assert!(diff.unified_diff.contains("+++ b/draft.txt"));
        assert!(!diff.unified_diff.contains("upstream.txt"));
        assert_eq!(
            run_git(root, &["count-objects"]).await.unwrap().1,
            objects_before
        );
        assert_eq!(
            run_git(&int.path, &["status", "--porcelain"])
                .await
                .unwrap()
                .1,
            "?? draft.txt"
        );
    }

    #[tokio::test]
    async fn promotion_checks_the_base_branch_and_patch_leaves_index_clean() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        std::fs::write(root.join("source.txt"), "old value\n").unwrap();
        init_git_repo(root).await.unwrap();
        let int = ensure_session_integration_worktree(root, "promotion")
            .await
            .unwrap();
        let base = int.base_branch.as_deref().unwrap();
        std::fs::write(int.path.join("source.txt"), "new value \n").unwrap();
        assert!(
            run_git_with_author(&int.path, &["commit", "-am", "Edit source"])
                .await
                .unwrap()
                .0
        );

        let diff = get_session_diff(
            root,
            &int.path,
            Some(base),
            Some(&int.base_commit_sha),
            &int.branch,
        )
        .await
        .unwrap();
        assert!(diff.unified_diff.ends_with("+new value \n"));

        assert!(run_git(root, &["checkout", "-b", "other"]).await.unwrap().0);
        let result = promote_session(
            root,
            base,
            &int.base_commit_sha,
            &int.branch,
            PromoteMode::Merge,
        )
        .await;
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("currently on 'other'")
        );
        assert_eq!(
            get_head_sha(root).await.as_deref(),
            Some(int.base_commit_sha.as_str())
        );

        assert!(run_git(root, &["checkout", base]).await.unwrap().0);
        promote_session(
            root,
            base,
            &int.base_commit_sha,
            &int.branch,
            PromoteMode::Patch,
        )
        .await
        .unwrap();
        assert!(
            run_git(root, &["diff", "--cached", "--quiet"])
                .await
                .unwrap()
                .0
        );
        assert!(
            run_git(root, &["diff", "--quiet"])
                .await
                .is_ok_and(|(ok, _, _)| !ok)
        );
        assert_eq!(
            get_head_sha(root).await.as_deref(),
            Some(int.base_commit_sha.as_str())
        );
    }

    #[tokio::test]
    async fn worktree_isolation_clean_merge_and_conflict_detection() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        std::fs::write(root.join("shared.txt"), "line1\nline2\nline3\n").unwrap();

        let status = init_git_repo(root).await.unwrap();
        assert!(status.is_git_repo);
        let base_sha = status.head_sha.unwrap();

        let int = ensure_session_integration_worktree(root, "s_test1")
            .await
            .unwrap();
        let wt_a = ensure_member_worktree(root, "s_test1", &int.branch, "codex")
            .await
            .unwrap();
        let wt_b = ensure_member_worktree(root, "s_test1", &int.branch, "claude")
            .await
            .unwrap();

        // Parallel disjoint edits auto-merge cleanly
        std::fs::write(wt_a.path.join("a.txt"), "from codex\n").unwrap();
        std::fs::write(wt_b.path.join("b.txt"), "from claude\n").unwrap();

        let res_a = integrate_member_turn(
            &int.path,
            &int.branch,
            &wt_a.path,
            &wt_a.branch,
            "codex",
            "add a.txt",
            true,
        )
        .await
        .unwrap();
        assert!(matches!(res_a, MergeOutcome::Clean { .. }));

        let res_b = integrate_member_turn(
            &int.path,
            &int.branch,
            &wt_b.path,
            &wt_b.branch,
            "claude",
            "add b.txt",
            true,
        )
        .await
        .unwrap();
        assert!(matches!(res_b, MergeOutcome::Clean { .. }));
        assert!(int.path.join("a.txt").exists());
        assert!(int.path.join("b.txt").exists());

        // Main checkout remains untouched until promote
        assert!(!root.join("a.txt").exists());

        // Overlapping edits produce MergeOutcome::Conflict
        sync_member_worktree(&wt_a.path, &int.branch).await.unwrap();
        sync_member_worktree(&wt_b.path, &int.branch).await.unwrap();
        std::fs::write(wt_a.path.join("shared.txt"), "codex-line1\nline2\nline3\n").unwrap();
        std::fs::write(wt_b.path.join("shared.txt"), "claude-line1\nline2\nline3\n").unwrap();

        let _ = integrate_member_turn(
            &int.path,
            &int.branch,
            &wt_a.path,
            &wt_a.branch,
            "codex",
            "edit shared",
            true,
        )
        .await
        .unwrap();
        let conflict = integrate_member_turn(
            &int.path,
            &int.branch,
            &wt_b.path,
            &wt_b.branch,
            "claude",
            "conflicting shared",
            false,
        )
        .await
        .unwrap();
        assert!(matches!(conflict, MergeOutcome::Conflict { .. }));

        // Promote clean state via Squash
        let promoted = promote_session(
            root,
            int.base_branch.as_deref().unwrap_or("main"),
            &base_sha,
            &int.branch,
            PromoteMode::Squash,
        )
        .await
        .unwrap();
        assert!(promoted.new_head_sha.is_some());
        assert!(root.join("a.txt").exists());
        assert!(root.join("b.txt").exists());

        prune_session_worktrees(root, "s_test1").await.unwrap();
        assert!(!root.join(".meta-harness/worktrees/s_test1").exists());
    }

    #[tokio::test]
    async fn dirty_commit_and_squash_promote_reset_clears_diff() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        std::fs::write(root.join("README.md"), "# Init\n").unwrap();
        init_git_repo(root).await.unwrap();

        let int = ensure_session_integration_worktree(root, "s_orch")
            .await
            .unwrap();
        let base = int.base_branch.as_deref().unwrap_or("main");

        // Orchestrator writes directly inside `_integration`
        std::fs::write(int.path.join("index.html"), "<h1>Hi</h1>\n").unwrap();
        let committed = commit_worktree_if_dirty(&int.path, "codex", "Build index page")
            .await
            .unwrap();
        assert!(committed.is_some());

        let before = get_session_diff(
            root,
            &int.path,
            Some(base),
            Some(&int.base_commit_sha),
            &int.branch,
        )
        .await
        .unwrap();
        assert_eq!(before.ahead_commits, 1);
        assert_eq!(before.diff_stat.files_changed, 1);
        assert!(before.unified_diff.contains("index.html"));

        let promoted = promote_session(
            root,
            base,
            &int.base_commit_sha,
            &int.branch,
            PromoteMode::Squash,
        )
        .await
        .unwrap();
        let new_sha = promoted.new_head_sha.unwrap();
        reset_worktree_to_sha(&int.path, &new_sha).await.unwrap();

        let after = get_session_diff(root, &int.path, Some(base), Some(&new_sha), &int.branch)
            .await
            .unwrap();
        assert_eq!(after.ahead_commits, 0);
        assert_eq!(after.diff_stat.files_changed, 0);
        assert!(after.unified_diff.trim().is_empty());
    }
}
