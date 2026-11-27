use rusqlite::{OptionalExtension, params};
use serde_json::Value;

use mh_core::{
    DiffStat, TrajectoryDetailLevel, TrajectoryEvent, TrajectoryEventKind, TurnStatus,
    TurnTrajectory,
};

use crate::{Store, StoreError, now_millis};

#[derive(Debug, Clone)]
pub struct StartTrajectoryParams {
    pub turn_id: Option<String>,
    pub session_id: String,
    pub member_handle: String,
    pub adapter_id: String,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub worktree_branch: Option<String>,
}

#[derive(Debug, Clone)]
pub struct PaginatedTrajectoryRead {
    pub trajectory: TurnTrajectory,
    pub detail: TrajectoryDetailLevel,
    pub page: u32,
    pub has_more: bool,
    pub events: Vec<TrajectoryEvent>,
}

impl Store {
    pub fn start_trajectory(&self, p: StartTrajectoryParams) -> Result<TurnTrajectory, StoreError> {
        let turn_id = p
            .turn_id
            .unwrap_or_else(|| format!("turn_{}", uuid::Uuid::new_v4().simple()));
        let started_at = now_millis();
        let conn = self.conn()?;
        conn.execute(
            "INSERT INTO turn_trajectories (
                turn_id, session_id, member_handle, adapter_id, model, effort,
                worktree_branch, status, summary, touched_files, diff_stat_json, started_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'running', '', '[]', '{}', ?8)",
            params![
                turn_id,
                p.session_id,
                p.member_handle,
                p.adapter_id,
                p.model,
                p.effort,
                p.worktree_branch,
                started_at,
            ],
        )?;
        Ok(TurnTrajectory {
            turn_id,
            session_id: p.session_id,
            member_handle: p.member_handle,
            adapter_id: p.adapter_id,
            model: p.model,
            effort: p.effort,
            worktree_branch: p.worktree_branch,
            head_commit_sha: None,
            status: TurnStatus::Running,
            summary: String::new(),
            touched_files: Vec::new(),
            diff_stat: DiffStat::default(),
            started_at,
            finished_at: None,
        })
    }

    pub fn append_trajectory_events(
        &self,
        turn_id: &str,
        events: &[(TrajectoryEventKind, Value)],
    ) -> Result<Vec<TrajectoryEvent>, StoreError> {
        if events.is_empty() {
            return Ok(Vec::new());
        }
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        let now = now_millis();
        let mut inserted = Vec::with_capacity(events.len());
        {
            let mut stmt = tx.prepare(
                "INSERT INTO trajectory_events (turn_id, event_kind, payload_json, created_at)
                 SELECT ?1, ?2, ?3, ?4
                 WHERE ?2 <> 'skill' OR NOT EXISTS (
                     SELECT 1 FROM trajectory_events WHERE turn_id = ?1 AND event_kind = 'skill'
                     AND json_extract(payload_json, '$.skill_id') = json_extract(?3, '$.skill_id')
                 )",
            )?;
            for (kind, payload) in events {
                let payload_json = serde_json::to_string(payload)?;
                if stmt.execute(params![turn_id, kind.as_str(), payload_json, now])? == 0 {
                    continue;
                }
                let id = tx.last_insert_rowid();
                inserted.push(TrajectoryEvent {
                    id,
                    turn_id: turn_id.to_string(),
                    event_kind: *kind,
                    payload: payload.clone(),
                    created_at: now,
                });
            }
        }
        tx.commit()?;
        Ok(inserted)
    }

    pub fn finish_trajectory(
        &self,
        turn_id: &str,
        status: TurnStatus,
        summary: &str,
        touched_files: &[String],
        diff_stat: &DiffStat,
        head_commit_sha: Option<&str>,
    ) -> Result<TurnTrajectory, StoreError> {
        let finished_at = now_millis();
        let touched_json = serde_json::to_string(touched_files)?;
        let stat_json = serde_json::to_string(diff_stat)?;
        let conn = self.conn()?;
        conn.execute(
            "UPDATE turn_trajectories
             SET status = ?1, summary = ?2, touched_files = ?3,
                 diff_stat_json = ?4, head_commit_sha = COALESCE(?5, head_commit_sha),
                 finished_at = ?6
             WHERE turn_id = ?7",
            params![
                status.as_str(),
                summary,
                touched_json,
                stat_json,
                head_commit_sha,
                finished_at,
                turn_id,
            ],
        )?;
        drop(conn);
        self.get_trajectory(turn_id)
    }

    pub fn get_trajectory(&self, turn_id: &str) -> Result<TurnTrajectory, StoreError> {
        let conn = self.conn()?;
        conn.query_row(
            "SELECT turn_id, session_id, member_handle, adapter_id, model, effort,
                    worktree_branch, head_commit_sha, status, summary, touched_files,
                    diff_stat_json, started_at, finished_at
             FROM turn_trajectories WHERE turn_id = ?1",
            params![turn_id],
            map_trajectory_row,
        )
        .optional()?
        .ok_or_else(|| StoreError::NotFound(format!("trajectory {turn_id}")))
    }

    pub fn list_trajectories(
        &self,
        session_id: &str,
        member_handle: Option<&str>,
        limit: u32,
    ) -> Result<Vec<TurnTrajectory>, StoreError> {
        let conn = self.conn()?;
        let limit_i = i64::from(limit.clamp(1, 200));
        if let Some(handle) = member_handle {
            let mut stmt = conn.prepare(
                "SELECT turn_id, session_id, member_handle, adapter_id, model, effort,
                        worktree_branch, head_commit_sha, status, summary, touched_files,
                        diff_stat_json, started_at, finished_at
                 FROM turn_trajectories
                 WHERE session_id = ?1 AND member_handle = ?2
                 ORDER BY started_at DESC
                 LIMIT ?3",
            )?;
            let rows = stmt.query_map(params![session_id, handle, limit_i], map_trajectory_row)?;
            rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
        } else {
            let mut stmt = conn.prepare(
                "SELECT turn_id, session_id, member_handle, adapter_id, model, effort,
                        worktree_branch, head_commit_sha, status, summary, touched_files,
                        diff_stat_json, started_at, finished_at
                 FROM turn_trajectories
                 WHERE session_id = ?1
                 ORDER BY started_at DESC
                 LIMIT ?2",
            )?;
            let rows = stmt.query_map(params![session_id, limit_i], map_trajectory_row)?;
            rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
        }
    }

    pub fn list_trajectory_events(
        &self,
        turn_id: &str,
    ) -> Result<Vec<TrajectoryEvent>, StoreError> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT id, turn_id, event_kind, payload_json, created_at
             FROM trajectory_events
             WHERE turn_id = ?1
             ORDER BY id ASC",
        )?;
        let rows = stmt.query_map(params![turn_id], map_event_row)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Reads trajectory events filtered by detail level and bounded to `max_bytes` (default 16 KB)
    /// per page so peer agents can inspect trajectories safely without context overflow.
    pub fn read_trajectory_paginated(
        &self,
        turn_id: &str,
        detail: TrajectoryDetailLevel,
        page: u32,
        max_bytes: usize,
    ) -> Result<PaginatedTrajectoryRead, StoreError> {
        let trajectory = self.get_trajectory(turn_id)?;
        if detail == TrajectoryDetailLevel::Summary {
            return Ok(PaginatedTrajectoryRead {
                trajectory,
                detail,
                page: 0,
                has_more: false,
                events: Vec::new(),
            });
        }

        let all = self.list_trajectory_events(turn_id)?;
        let filtered: Vec<TrajectoryEvent> = all
            .into_iter()
            .filter(|ev| match detail {
                TrajectoryDetailLevel::Summary => false,
                TrajectoryDetailLevel::ToolCalls => matches!(
                    ev.event_kind,
                    TrajectoryEventKind::ToolCall
                        | TrajectoryEventKind::Terminal
                        | TrajectoryEventKind::Error
                        | TrajectoryEventKind::Plan
                        | TrajectoryEventKind::Skill
                ),
                TrajectoryDetailLevel::Diffs => {
                    matches!(ev.event_kind, TrajectoryEventKind::Diff)
                }
                TrajectoryDetailLevel::Full => true,
            })
            .collect();

        // Partition into <= max_bytes pages
        let budget = max_bytes.max(1024);
        let mut pages: Vec<Vec<TrajectoryEvent>> = Vec::new();
        let mut cur_page: Vec<TrajectoryEvent> = Vec::new();
        let mut cur_bytes = 0usize;

        for ev in filtered {
            let sz = serde_json::to_string(&ev)?.len();
            if !cur_page.is_empty() && cur_bytes + sz > budget {
                pages.push(std::mem::take(&mut cur_page));
                cur_bytes = 0;
            }
            cur_bytes += sz;
            cur_page.push(ev);
        }
        if !cur_page.is_empty() {
            pages.push(cur_page);
        }

        let idx = page as usize;
        let has_more = idx + 1 < pages.len();
        let events = pages.get(idx).cloned().unwrap_or_default();

        Ok(PaginatedTrajectoryRead {
            trajectory,
            detail,
            page,
            has_more,
            events,
        })
    }
}

fn map_trajectory_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<TurnTrajectory> {
    let touched_str: String = row.get(10)?;
    let stat_str: String = row.get(11)?;
    Ok(TurnTrajectory {
        turn_id: row.get(0)?,
        session_id: row.get(1)?,
        member_handle: row.get(2)?,
        adapter_id: row.get(3)?,
        model: row.get(4)?,
        effort: row.get(5)?,
        worktree_branch: row.get(6)?,
        head_commit_sha: row.get(7)?,
        status: TurnStatus::parse(&row.get::<_, String>(8)?),
        summary: row.get(9)?,
        touched_files: serde_json::from_str(&touched_str).unwrap_or_default(),
        diff_stat: serde_json::from_str(&stat_str).unwrap_or_default(),
        started_at: row.get(12)?,
        finished_at: row.get(13)?,
    })
}

fn map_event_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<TrajectoryEvent> {
    let payload_str: String = row.get(3)?;
    let payload =
        serde_json::from_str(&payload_str).unwrap_or_else(|_| Value::Object(Default::default()));
    Ok(TrajectoryEvent {
        id: row.get(0)?,
        turn_id: row.get(1)?,
        event_kind: TrajectoryEventKind::parse(&row.get::<_, String>(2)?),
        payload,
        created_at: row.get(4)?,
    })
}
