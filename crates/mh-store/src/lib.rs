pub mod error;
pub mod memory;
pub mod messages;
pub mod migrations;
pub mod projects;
pub mod sessions;
pub mod trajectories;

use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard};

use rusqlite::Connection;

pub use error::StoreError;
pub use memory::{AdapterQuotaPref, RecordEpisodicJobParams};
pub use messages::NewMessageParams;
pub use sessions::{CreateSessionParams, PatchSessionParams, UpsertMemberParams};
pub use trajectories::{PaginatedTrajectoryRead, StartTrajectoryParams};

#[derive(Clone)]
pub struct Store {
    conn: Arc<Mutex<Connection>>,
}

impl Store {
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(path)?;
        migrations::apply_migrations(&conn)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    pub fn open_in_memory() -> Result<Self, StoreError> {
        let conn = Connection::open_in_memory()?;
        migrations::apply_migrations(&conn)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    pub(crate) fn conn(&self) -> Result<MutexGuard<'_, Connection>, StoreError> {
        self.conn
            .lock()
            .map_err(|e| StoreError::Conflict(format!("database mutex poisoned: {e}")))
    }
}

pub(crate) fn now_millis() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

#[cfg(test)]
mod tests {
    use super::*;
    use mh_core::{
        IsolationMode, MemberRole, MessageKind, PermissionPolicy, SenderKind,
        TrajectoryDetailLevel, TrajectoryEventKind, TurnStatus,
    };
    use serde_json::json;

    #[test]
    fn store_crud_deltas_trajectories_and_fts5_memory() {
        let store = Store::open_in_memory().unwrap();
        let proj = store.upsert_project("demo", "/tmp/demo", true).unwrap();

        let sess = store
            .create_session(CreateSessionParams {
                id: None,
                project_id: proj.id.clone(),
                title: "Auth work".into(),
                base_branch: Some("main".into()),
                base_commit_sha: Some("abc123".into()),
                integration_branch: Some("mh/s-1".into()),
                integration_wt_path: Some(
                    "/tmp/demo/.meta-harness/worktrees/s-1/_integration".into(),
                ),
                isolation_mode: IsolationMode::WorktreePerMember,
                orchestrator_member: Some("claude".into()),
                confirm_before_dispatch: false,
                max_hops: 6,
                permission_policy: PermissionPolicy::WorktreeSmart,
            })
            .unwrap();

        store
            .upsert_member(UpsertMemberParams {
                session_id: sess.id.clone(),
                handle: "codex".into(),
                adapter_id: "codex".into(),
                display_name: "Codex".into(),
                role: MemberRole::Executor,
                persona_prompt: String::new(),
                worktree_branch: Some("mh/s-1/codex".into()),
                worktree_path: Some("/tmp/demo/.meta-harness/worktrees/s-1/_exec_codex".into()),
            })
            .unwrap();

        let m1 = store
            .append_message(NewMessageParams {
                id: None,
                session_id: sess.id.clone(),
                chain_id: "c1".into(),
                parent_msg_id: None,
                sender_kind: SenderKind::User,
                sender_id: "user".into(),
                recipients: vec!["codex".into()],
                kind: MessageKind::Text,
                content: "Implement JWT".into(),
                turn_id: None,
                metadata: json!({}),
            })
            .unwrap();

        let m2 = store
            .append_message(NewMessageParams {
                id: None,
                session_id: sess.id.clone(),
                chain_id: "c1".into(),
                parent_msg_id: None,
                sender_kind: SenderKind::Member,
                sender_id: "codex".into(),
                recipients: vec![],
                kind: MessageKind::Text,
                content: "Done".into(),
                turn_id: Some("t1".into()),
                metadata: json!({}),
            })
            .unwrap();

        let deltas = store.list_unseen_deltas(&sess.id, m1.seq, 50).unwrap();
        assert_eq!(deltas.len(), 1);
        assert_eq!(deltas[0].id, m2.id);

        let traj = store
            .start_trajectory(StartTrajectoryParams {
                turn_id: Some("t1".into()),
                session_id: sess.id.clone(),
                member_handle: "codex".into(),
                adapter_id: "codex".into(),
                model: Some("gpt-5.4".into()),
                effort: Some("high".into()),
                worktree_branch: Some("mh/s-1/codex".into()),
            })
            .unwrap();
        store
            .append_trajectory_events(
                &traj.turn_id,
                &[(
                    TrajectoryEventKind::Diff,
                    json!({ "path": "src/auth/jwt.rs", "diff": "+fn verify() {}" }),
                )],
            )
            .unwrap();
        store
            .finish_trajectory(
                &traj.turn_id,
                TurnStatus::Completed,
                "Implemented JWT verifier",
                &["src/auth/jwt.rs".into()],
                &mh_core::DiffStat {
                    files_changed: 1,
                    insertions: 10,
                    deletions: 0,
                },
                Some("def456"),
            )
            .unwrap();

        let read_diffs = store
            .read_trajectory_paginated(&traj.turn_id, TrajectoryDetailLevel::Diffs, 0, 16384)
            .unwrap();
        assert_eq!(read_diffs.events.len(), 1);

        store
            .record_curated_job(RecordEpisodicJobParams {
                project_id: proj.id.clone(),
                session_id: Some(sess.id.clone()),
                turn_id: Some(traj.turn_id),
                member_handle: "codex".into(),
                adapter_id: "codex".into(),
                model: Some("gpt-5.4".into()),
                effort: Some("high".into()),
                task_brief: "Implement JWT middleware".into(),
                curated_summary: "Codex added JWT token verifier in src/auth/jwt.rs".into(),
                touched_modules: vec!["src/auth/jwt.rs".into()],
                outcome: "completed".into(),
                curated_by: "claude".into(),
                pinned: false,
            })
            .unwrap();

        // Symbols like '@' and '/' must not crash FTS5
        let found = store
            .recall_jobs(&proj.id, Some("@codex src/auth/jwt.rs"), 5)
            .unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].member_handle, "codex");
    }
}
