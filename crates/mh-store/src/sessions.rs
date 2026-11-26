use rusqlite::{OptionalExtension, params};

use mh_core::{
    IsolationMode, MemberRole, PermissionPolicy, PermissionRule, Session, SessionMember,
};

use crate::{Store, StoreError, now_millis};

#[derive(Debug, Clone)]
pub struct CreateSessionParams {
    pub id: Option<String>,
    pub project_id: Option<String>,
    pub title: String,
    pub base_branch: Option<String>,
    pub base_commit_sha: Option<String>,
    pub integration_branch: Option<String>,
    pub integration_wt_path: Option<String>,
    pub isolation_mode: IsolationMode,
    pub orchestrator_member: Option<String>,
    pub confirm_before_dispatch: bool,
    pub max_hops: u32,
    pub permission_policy: PermissionPolicy,
}

#[derive(Debug, Clone, Default)]
pub struct PatchSessionParams {
    pub title: Option<String>,
    pub orchestrator_member: Option<Option<String>>,
    pub confirm_before_dispatch: Option<bool>,
    pub max_hops: Option<u32>,
    pub permission_policy: Option<PermissionPolicy>,
    pub working_summary: Option<String>,
    pub archived_at: Option<Option<i64>>,
}

#[derive(Debug, Clone)]
pub struct UpsertMemberParams {
    pub session_id: String,
    pub handle: String,
    pub adapter_id: String,
    pub display_name: String,
    pub role: MemberRole,
    pub persona_prompt: String,
    pub worktree_branch: Option<String>,
    pub worktree_path: Option<String>,
}

impl Store {
    pub fn create_session(&self, params_in: CreateSessionParams) -> Result<Session, StoreError> {
        let now = now_millis();
        let id = params_in
            .id
            .unwrap_or_else(|| format!("s_{}", &uuid::Uuid::new_v4().simple().to_string()[..12]));
        let conn = self.conn()?;
        conn.execute(
            "INSERT INTO sessions (
                id, project_id, title, base_branch, base_commit_sha,
                integration_branch, integration_wt_path, isolation_mode,
                orchestrator_member, confirm_before_dispatch, max_hops,
                permission_policy, working_summary, archived_at, created_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, '', NULL, ?13, ?14)",
            params![
                id,
                params_in.project_id,
                params_in.title,
                params_in.base_branch,
                params_in.base_commit_sha,
                params_in.integration_branch,
                params_in.integration_wt_path,
                params_in.isolation_mode.as_str(),
                params_in.orchestrator_member,
                i32::from(params_in.confirm_before_dispatch),
                params_in.max_hops,
                params_in.permission_policy.as_str(),
                now,
                now,
            ],
        )?;
        drop(conn);
        self.get_session(&id)
    }

    pub fn get_session(&self, id: &str) -> Result<Session, StoreError> {
        let conn = self.conn()?;
        conn.query_row(
            "SELECT id, project_id, title, base_branch, base_commit_sha,
                    integration_branch, integration_wt_path, isolation_mode,
                    orchestrator_member, confirm_before_dispatch, max_hops,
                    permission_policy, working_summary, archived_at, created_at, updated_at
             FROM sessions WHERE id = ?1",
            params![id],
            map_session_row,
        )
        .optional()?
        .ok_or_else(|| StoreError::NotFound(format!("session {id}")))
    }

    pub fn list_sessions(
        &self,
        project_id: Option<&str>,
        include_archived: bool,
    ) -> Result<Vec<Session>, StoreError> {
        let conn = self.conn()?;
        let arch_filter = if include_archived {
            ""
        } else {
            " AND archived_at IS NULL"
        };
        if let Some(pid) = project_id {
            let sql = format!(
                "SELECT id, project_id, title, base_branch, base_commit_sha,
                        integration_branch, integration_wt_path, isolation_mode,
                        orchestrator_member, confirm_before_dispatch, max_hops,
                        permission_policy, working_summary, archived_at, created_at, updated_at
                 FROM sessions WHERE project_id = ?1{arch_filter} ORDER BY updated_at DESC"
            );
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt.query_map(params![pid], map_session_row)?;
            rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
        } else {
            let sql = format!(
                "SELECT id, project_id, title, base_branch, base_commit_sha,
                        integration_branch, integration_wt_path, isolation_mode,
                        orchestrator_member, confirm_before_dispatch, max_hops,
                        permission_policy, working_summary, archived_at, created_at, updated_at
                 FROM sessions WHERE project_id IS NULL{arch_filter} ORDER BY updated_at DESC"
            );
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt.query_map([], map_session_row)?;
            rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
        }
    }

    pub fn list_all_sessions(&self, include_archived: bool) -> Result<Vec<Session>, StoreError> {
        let conn = self.conn()?;
        let where_clause = if include_archived {
            ""
        } else {
            " WHERE archived_at IS NULL"
        };
        let sql = format!(
            "SELECT id, project_id, title, base_branch, base_commit_sha,
                    integration_branch, integration_wt_path, isolation_mode,
                    orchestrator_member, confirm_before_dispatch, max_hops,
                    permission_policy, working_summary, archived_at, created_at, updated_at
             FROM sessions{where_clause} ORDER BY updated_at DESC"
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map([], map_session_row)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn patch_session(
        &self,
        id: &str,
        patch: PatchSessionParams,
    ) -> Result<Session, StoreError> {
        let mut current = self.get_session(id)?;
        if let Some(t) = patch.title {
            current.title = t;
        }
        if let Some(orch) = patch.orchestrator_member {
            current.orchestrator_member = orch;
        }
        if let Some(c) = patch.confirm_before_dispatch {
            current.confirm_before_dispatch = c;
        }
        if let Some(m) = patch.max_hops {
            current.max_hops = m;
        }
        if let Some(p) = patch.permission_policy {
            current.permission_policy = p;
        }
        if let Some(ws) = patch.working_summary {
            current.working_summary = ws;
        }
        if let Some(arch) = patch.archived_at {
            current.archived_at = arch;
        }
        let now = now_millis();

        let conn = self.conn()?;
        conn.execute(
            "UPDATE sessions
             SET title = ?1, orchestrator_member = ?2, confirm_before_dispatch = ?3,
                 max_hops = ?4, permission_policy = ?5, working_summary = ?6, archived_at = ?7, updated_at = ?8
             WHERE id = ?9",
            params![
                current.title,
                current.orchestrator_member,
                i32::from(current.confirm_before_dispatch),
                current.max_hops,
                current.permission_policy.as_str(),
                current.working_summary,
                current.archived_at,
                now,
                id,
            ],
        )?;
        drop(conn);
        self.get_session(id)
    }

    pub fn delete_archived_sessions(&self) -> Result<Vec<Session>, StoreError> {
        let archived: Vec<Session> = self
            .list_all_sessions(true)?
            .into_iter()
            .filter(|s| s.archived_at.is_some())
            .collect();
        let conn = self.conn()?;
        conn.execute("DELETE FROM sessions WHERE archived_at IS NOT NULL", [])?;
        Ok(archived)
    }

    pub fn clear_session_acp_ids(&self, session_id: &str) -> Result<(), StoreError> {
        let conn = self.conn()?;
        conn.execute(
            "UPDATE session_members SET acp_session_id = NULL WHERE session_id = ?1",
            params![session_id],
        )?;
        Ok(())
    }

    pub fn vacuum(&self) -> Result<(), StoreError> {
        let conn = self.conn()?;
        conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE); VACUUM;")?;
        Ok(())
    }

    pub fn update_session_git_state(
        &self,
        id: &str,
        base_branch: Option<&str>,
        base_commit_sha: Option<&str>,
        integration_branch: Option<&str>,
        integration_wt_path: Option<&str>,
        isolation_mode: IsolationMode,
    ) -> Result<Session, StoreError> {
        let conn = self.conn()?;
        let changed = conn.execute(
            "UPDATE sessions
             SET base_branch = ?1, base_commit_sha = ?2, integration_branch = ?3,
                 integration_wt_path = ?4, isolation_mode = ?5, updated_at = ?6
             WHERE id = ?7",
            params![
                base_branch,
                base_commit_sha,
                integration_branch,
                integration_wt_path,
                isolation_mode.as_str(),
                now_millis(),
                id,
            ],
        )?;
        if changed == 0 {
            return Err(StoreError::NotFound(format!("session {id}")));
        }
        drop(conn);
        self.get_session(id)
    }

    pub fn delete_session(&self, id: &str) -> Result<(), StoreError> {
        let conn = self.conn()?;
        let changed = conn.execute("DELETE FROM sessions WHERE id = ?1", params![id])?;
        if changed == 0 {
            return Err(StoreError::NotFound(format!("session {id}")));
        }
        Ok(())
    }

    pub fn upsert_member(&self, m: UpsertMemberParams) -> Result<SessionMember, StoreError> {
        let conn = self.conn()?;
        conn.execute(
            "INSERT INTO session_members (
                session_id, handle, adapter_id, display_name, role,
                persona_prompt, worktree_branch, worktree_path
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(session_id, handle) DO UPDATE SET
                adapter_id = excluded.adapter_id,
                display_name = excluded.display_name,
                role = excluded.role,
                persona_prompt = excluded.persona_prompt,
                worktree_branch = COALESCE(excluded.worktree_branch, session_members.worktree_branch),
                worktree_path = COALESCE(excluded.worktree_path, session_members.worktree_path)",
            params![
                m.session_id,
                m.handle,
                m.adapter_id,
                m.display_name,
                m.role.as_str(),
                m.persona_prompt,
                m.worktree_branch,
                m.worktree_path,
            ],
        )?;
        drop(conn);
        self.get_member(&m.session_id, &m.handle)
    }

    pub fn get_member(&self, session_id: &str, handle: &str) -> Result<SessionMember, StoreError> {
        let conn = self.conn()?;
        conn.query_row(
            "SELECT session_id, handle, adapter_id, display_name, role, persona_prompt,
                    worktree_branch, worktree_path, acp_session_id, active_model, active_effort,
                    last_seen_msg_seq
             FROM session_members WHERE session_id = ?1 AND handle = ?2",
            params![session_id, handle],
            map_member_row,
        )
        .optional()?
        .ok_or_else(|| StoreError::NotFound(format!("member {session_id}/{handle}")))
    }

    pub fn list_members(&self, session_id: &str) -> Result<Vec<SessionMember>, StoreError> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT session_id, handle, adapter_id, display_name, role, persona_prompt,
                    worktree_branch, worktree_path, acp_session_id, active_model, active_effort,
                    last_seen_msg_seq
             FROM session_members WHERE session_id = ?1 ORDER BY handle ASC",
        )?;
        let rows = stmt.query_map(params![session_id], map_member_row)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn delete_member(&self, session_id: &str, handle: &str) -> Result<(), StoreError> {
        let conn = self.conn()?;
        let changed = conn.execute(
            "DELETE FROM session_members WHERE session_id = ?1 AND handle = ?2",
            params![session_id, handle],
        )?;
        if changed == 0 {
            return Err(StoreError::NotFound(format!(
                "member {session_id}/{handle}"
            )));
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn update_member_runtime(
        &self,
        session_id: &str,
        handle: &str,
        acp_session_id: Option<&str>,
        active_model: Option<&str>,
        active_effort: Option<&str>,
        worktree_branch: Option<&str>,
        worktree_path: Option<&str>,
    ) -> Result<(), StoreError> {
        let conn = self.conn()?;
        conn.execute(
            "UPDATE session_members
             SET acp_session_id = COALESCE(?1, acp_session_id),
                 active_model = COALESCE(?2, active_model),
                 active_effort = COALESCE(?3, active_effort),
                 worktree_branch = COALESCE(?4, worktree_branch),
                 worktree_path = COALESCE(?5, worktree_path)
             WHERE session_id = ?6 AND handle = ?7",
            params![
                acp_session_id,
                active_model,
                active_effort,
                worktree_branch,
                worktree_path,
                session_id,
                handle,
            ],
        )?;
        Ok(())
    }

    pub fn update_member_model_effort(
        &self,
        session_id: &str,
        handle: &str,
        active_model: Option<Option<&str>>,
        active_effort: Option<Option<&str>>,
    ) -> Result<SessionMember, StoreError> {
        let mut member = self.get_member(session_id, handle)?;
        if let Some(m) = active_model {
            member.active_model = m
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string);
        }
        if let Some(e) = active_effort {
            member.active_effort = e
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string);
        }
        let conn = self.conn()?;
        conn.execute(
            "UPDATE session_members
             SET active_model = ?1, active_effort = ?2
             WHERE session_id = ?3 AND handle = ?4",
            params![
                member.active_model,
                member.active_effort,
                session_id,
                handle
            ],
        )?;
        drop(conn);
        self.get_member(session_id, handle)
    }

    pub fn advance_member_last_seen(
        &self,
        session_id: &str,
        handle: &str,
        seq: i64,
    ) -> Result<(), StoreError> {
        let conn = self.conn()?;
        conn.execute(
            "UPDATE session_members
             SET last_seen_msg_seq = MAX(last_seen_msg_seq, ?1)
             WHERE session_id = ?2 AND handle = ?3",
            params![seq, session_id, handle],
        )?;
        Ok(())
    }

    pub fn add_permission_rule(
        &self,
        session_id: &str,
        tool_kind: &str,
        pattern: &str,
        decision: &str,
    ) -> Result<PermissionRule, StoreError> {
        let id = format!("perm_{}", uuid::Uuid::new_v4().simple());
        let created_at = now_millis();
        let conn = self.conn()?;
        conn.execute(
            "INSERT INTO session_permission_rules (id, session_id, tool_kind, pattern, decision, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![id, session_id, tool_kind, pattern, decision, created_at],
        )?;
        Ok(PermissionRule {
            id,
            session_id: session_id.to_string(),
            tool_kind: tool_kind.to_string(),
            pattern: pattern.to_string(),
            decision: decision.to_string(),
            created_at,
        })
    }

    pub fn list_permission_rules(
        &self,
        session_id: &str,
    ) -> Result<Vec<PermissionRule>, StoreError> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT id, session_id, tool_kind, pattern, decision, created_at
             FROM session_permission_rules WHERE session_id = ?1 ORDER BY created_at ASC",
        )?;
        let rows = stmt.query_map(params![session_id], |row| {
            Ok(PermissionRule {
                id: row.get(0)?,
                session_id: row.get(1)?,
                tool_kind: row.get(2)?,
                pattern: row.get(3)?,
                decision: row.get(4)?,
                created_at: row.get(5)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn delete_permission_rule(&self, id: &str) -> Result<(), StoreError> {
        let conn = self.conn()?;
        conn.execute(
            "DELETE FROM session_permission_rules WHERE id = ?1",
            params![id],
        )?;
        Ok(())
    }
}

fn map_session_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Session> {
    Ok(Session {
        id: row.get(0)?,
        project_id: row.get(1)?,
        title: row.get(2)?,
        base_branch: row.get(3)?,
        base_commit_sha: row.get(4)?,
        integration_branch: row.get(5)?,
        integration_wt_path: row.get(6)?,
        isolation_mode: IsolationMode::parse(&row.get::<_, String>(7)?),
        orchestrator_member: row.get(8)?,
        confirm_before_dispatch: row.get::<_, i32>(9)? != 0,
        max_hops: row.get(10)?,
        permission_policy: PermissionPolicy::parse(&row.get::<_, String>(11)?),
        working_summary: row.get(12)?,
        archived_at: row.get(13)?,
        created_at: row.get(14)?,
        updated_at: row.get(15)?,
    })
}

fn map_member_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<SessionMember> {
    Ok(SessionMember {
        session_id: row.get(0)?,
        handle: row.get(1)?,
        adapter_id: row.get(2)?,
        display_name: row.get(3)?,
        role: MemberRole::parse(&row.get::<_, String>(4)?),
        persona_prompt: row.get(5)?,
        worktree_branch: row.get(6)?,
        worktree_path: row.get(7)?,
        acp_session_id: row.get(8)?,
        active_model: row.get(9)?,
        active_effort: row.get(10)?,
        last_seen_msg_seq: row.get(11)?,
    })
}
