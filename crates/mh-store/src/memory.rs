use std::collections::HashSet;

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

use mh_core::{AdapterModelPref, EpisodicJob, ProjectFact};

use crate::{Store, StoreError, now_millis};

#[derive(Debug, Clone)]
pub struct RecordEpisodicJobParams {
    pub project_id: String,
    pub session_id: Option<String>,
    pub turn_id: Option<String>,
    pub member_handle: String,
    pub adapter_id: String,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub task_brief: String,
    pub curated_summary: String,
    pub touched_modules: Vec<String>,
    pub outcome: String,
    pub curated_by: String,
    pub pinned: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdapterQuotaPref {
    pub adapter_id: String,
    pub plan_label: Option<String>,
    pub exact_probe_opt_in: bool,
    pub manual_5h_pct: Option<f32>,
    pub manual_weekly_pct: Option<f32>,
}

impl Store {
    pub fn record_curated_job(
        &self,
        p: RecordEpisodicJobParams,
    ) -> Result<EpisodicJob, StoreError> {
        let id = format!("job_{}", uuid::Uuid::new_v4().simple());
        let created_at = now_millis();
        let modules_json = serde_json::to_string(&p.touched_modules)?;
        let modules_fts = p.touched_modules.join(" ");

        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        tx.execute(
            "INSERT INTO episodic_jobs (
                id, project_id, session_id, turn_id, member_handle, adapter_id,
                model, effort, task_brief, curated_summary, touched_modules,
                outcome, curated_by, pinned, created_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
            params![
                id,
                p.project_id,
                p.session_id,
                p.turn_id,
                p.member_handle,
                p.adapter_id,
                p.model,
                p.effort,
                p.task_brief,
                p.curated_summary,
                modules_json,
                p.outcome,
                p.curated_by,
                i32::from(p.pinned),
                created_at,
            ],
        )?;
        tx.execute(
            "INSERT INTO episodic_jobs_fts (
                job_id, project_id, member_handle, adapter_id,
                task_brief, curated_summary, touched_modules
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                id,
                p.project_id,
                p.member_handle,
                p.adapter_id,
                p.task_brief,
                p.curated_summary,
                modules_fts,
            ],
        )?;
        tx.commit()?;

        Ok(EpisodicJob {
            id,
            project_id: p.project_id,
            session_id: p.session_id,
            turn_id: p.turn_id,
            member_handle: p.member_handle,
            adapter_id: p.adapter_id,
            model: p.model,
            effort: p.effort,
            task_brief: p.task_brief,
            curated_summary: p.curated_summary,
            touched_modules: p.touched_modules,
            outcome: p.outcome,
            curated_by: p.curated_by,
            pinned: p.pinned,
            created_at,
        })
    }

    /// Queries project-wide episodic jobs via FTS5 BM25 + pinned/recency fallback.
    pub fn recall_jobs(
        &self,
        project_id: &str,
        query: Option<&str>,
        limit: u32,
    ) -> Result<Vec<EpisodicJob>, StoreError> {
        let conn = self.conn()?;
        let limit_i = i64::from(limit.clamp(1, 50));
        let fts_expr = query.and_then(build_safe_fts_query);

        if let Some(expr) = fts_expr {
            let mut stmt = conn.prepare(
                "SELECT j.id, j.project_id, j.session_id, j.turn_id, j.member_handle, j.adapter_id,
                        j.model, j.effort, j.task_brief, j.curated_summary, j.touched_modules,
                        j.outcome, j.curated_by, j.pinned, j.created_at
                 FROM episodic_jobs_fts f
                 JOIN episodic_jobs j ON j.id = f.job_id
                 WHERE f.project_id = ?1 AND episodic_jobs_fts MATCH ?2
                 ORDER BY j.pinned DESC, bm25(episodic_jobs_fts) ASC, j.created_at DESC
                 LIMIT ?3",
            )?;
            let rows = stmt.query_map(params![project_id, expr, limit_i], map_job_row)?;
            let matches: Vec<EpisodicJob> = rows.collect::<Result<Vec<_>, _>>()?;
            if !matches.is_empty() {
                return Ok(matches);
            }
        }

        let mut stmt = conn.prepare(
            "SELECT id, project_id, session_id, turn_id, member_handle, adapter_id,
                    model, effort, task_brief, curated_summary, touched_modules,
                    outcome, curated_by, pinned, created_at
             FROM episodic_jobs
             WHERE project_id = ?1
             ORDER BY pinned DESC, created_at DESC
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![project_id, limit_i], map_job_row)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn has_job_for_turn(&self, turn_id: &str) -> Result<bool, StoreError> {
        let conn = self.conn()?;
        let exists: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM episodic_jobs WHERE turn_id = ?1)",
            params![turn_id],
            |row| row.get(0),
        )?;
        Ok(exists)
    }

    pub fn set_job_pinned(&self, job_id: &str, pinned: bool) -> Result<(), StoreError> {
        let conn = self.conn()?;
        let changed = conn.execute(
            "UPDATE episodic_jobs SET pinned = ?1 WHERE id = ?2",
            params![i32::from(pinned), job_id],
        )?;
        if changed == 0 {
            return Err(StoreError::NotFound(format!("episodic job {job_id}")));
        }
        Ok(())
    }

    pub fn delete_job(&self, job_id: &str) -> Result<(), StoreError> {
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        tx.execute(
            "DELETE FROM episodic_jobs_fts WHERE job_id = ?1",
            params![job_id],
        )?;
        let changed = tx.execute("DELETE FROM episodic_jobs WHERE id = ?1", params![job_id])?;
        if changed == 0 {
            return Err(StoreError::NotFound(format!("episodic job {job_id}")));
        }
        tx.commit()?;
        Ok(())
    }

    pub fn record_fact(
        &self,
        project_id: &str,
        category: &str,
        summary: &str,
        author_id: &str,
    ) -> Result<ProjectFact, StoreError> {
        let (category, summary) = validate_fact(category, summary)?;
        let conn = self.conn()?;
        if let Some(existing) = conn.query_row(
            "SELECT id, project_id, category, summary, author_id, created_at, updated_at FROM project_facts
             WHERE project_id = ?1 AND superseded_at IS NULL AND category = ?2 COLLATE NOCASE AND trim(summary) = ?3 COLLATE NOCASE
             ORDER BY (author_id = 'user') DESC, updated_at DESC, id LIMIT 1",
            params![project_id, category, summary], map_fact_row,
        ).optional()? {
            return Ok(existing);
        }
        let id = format!("fact_{}", uuid::Uuid::new_v4().simple());
        let created_at = now_millis();
        conn.execute(
            "INSERT INTO project_facts (id, project_id, category, summary, author_id, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)",
            params![id, project_id, category, summary, author_id, created_at],
        )?;
        Ok(ProjectFact {
            id,
            project_id: project_id.to_string(),
            category: category.to_string(),
            summary: summary.to_string(),
            author_id: author_id.to_string(),
            created_at,
            updated_at: created_at,
        })
    }

    pub fn revise_fact(
        &self,
        project_id: &str,
        fact_id: &str,
        category: Option<&str>,
        summary: &str,
        agent: Option<&str>,
        expected_updated_at: i64,
    ) -> Result<ProjectFact, StoreError> {
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        let mut fact = tx.query_row(
            "SELECT id, project_id, category, summary, author_id, created_at, updated_at FROM project_facts
             WHERE project_id = ?1 AND id = ?2 AND superseded_at IS NULL",
            params![project_id, fact_id], map_fact_row,
        ).optional()?.ok_or_else(|| StoreError::NotFound(format!("project fact {fact_id}")))?;
        if fact.author_id == "user" && agent.is_some() {
            return Err(StoreError::Conflict(
                "Change user-authored facts in Project Memory.".into(),
            ));
        }
        if fact.updated_at != expected_updated_at {
            return Err(StoreError::Conflict(
                "This fact changed. Reload it before revising.".into(),
            ));
        }
        let (category, summary) = validate_fact(category.unwrap_or(&fact.category), summary)?;
        let author = agent.unwrap_or("user");
        let updated_at = now_millis().max(fact.updated_at.saturating_add(1));
        hide_duplicate_facts(&tx, &fact, agent.is_none(), updated_at)?;
        tx.execute(
            "UPDATE project_facts SET category = ?1, summary = ?2, author_id = ?3, updated_at = ?4 WHERE project_id = ?5 AND id = ?6",
            params![category, summary, author, updated_at, project_id, fact_id],
        )?;
        tx.commit()?;
        fact.category = category.to_owned();
        fact.summary = summary.to_owned();
        fact.author_id = author.to_owned();
        fact.updated_at = updated_at;
        Ok(fact)
    }

    pub fn list_facts(&self, project_id: &str, limit: u32) -> Result<Vec<ProjectFact>, StoreError> {
        self.recall_facts(project_id, None, limit)
    }

    pub fn recall_facts(
        &self,
        project_id: &str,
        query: Option<&str>,
        limit: u32,
    ) -> Result<Vec<ProjectFact>, StoreError> {
        let conn = self.conn()?;
        let limit_i = i64::from(limit.clamp(1, 100));
        let mut stmt = conn.prepare(
            "SELECT id, project_id, category, summary, author_id, created_at, updated_at
             FROM (SELECT *, ROW_NUMBER() OVER (
                 PARTITION BY category COLLATE NOCASE, trim(summary) COLLATE NOCASE
                 ORDER BY (author_id = 'user') DESC, updated_at DESC, id
             ) AS duplicate_rank FROM project_facts WHERE project_id = ?1 AND superseded_at IS NULL)
             WHERE duplicate_rank = 1 AND (?3 = '' OR instr(lower(summary), lower(?3)) > 0 OR instr(lower(category), lower(?3)) > 0)
             ORDER BY updated_at DESC, id
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(
            params![project_id, limit_i, query.unwrap_or_default().trim()],
            map_fact_row,
        )?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn delete_fact(&self, project_id: &str, fact_id: &str) -> Result<(), StoreError> {
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        let fact = tx.query_row(
            "SELECT id, project_id, category, summary, author_id, created_at, updated_at FROM project_facts WHERE project_id = ?1 AND id = ?2",
            params![project_id, fact_id], map_fact_row,
        ).optional()?.ok_or_else(|| StoreError::NotFound(format!("project fact {fact_id}")))?;
        hide_duplicate_facts(&tx, &fact, true, now_millis())?;
        tx.execute(
            "DELETE FROM project_facts WHERE project_id = ?1 AND id = ?2",
            params![project_id, fact_id],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn upsert_adapter_quota_pref(
        &self,
        adapter_id: &str,
        plan_label: Option<&str>,
        exact_probe_opt_in: bool,
        manual_5h_pct: Option<f32>,
        manual_weekly_pct: Option<f32>,
    ) -> Result<AdapterQuotaPref, StoreError> {
        let pref = AdapterQuotaPref {
            adapter_id: adapter_id.into(),
            plan_label: plan_label.map(str::to_string),
            exact_probe_opt_in,
            manual_5h_pct,
            manual_weekly_pct,
        };
        let conn = self.conn()?;
        write_quota_pref(&conn, &pref)?;
        Ok(pref)
    }

    pub fn get_adapter_quota_pref(
        &self,
        adapter_id: &str,
    ) -> Result<Option<AdapterQuotaPref>, StoreError> {
        self.conn()?.query_row(
            "SELECT adapter_id, plan_label, exact_probe_opt_in, manual_5h_pct, manual_weekly_pct
             FROM adapter_quota_prefs WHERE adapter_id = ?1",
            params![adapter_id],
            query_quota_pref,
        ).optional().map_err(Into::into)
    }

    pub fn list_adapter_quota_prefs(&self) -> Result<Vec<AdapterQuotaPref>, StoreError> {
        let conn = self.conn()?;
        list_quota_prefs(&conn)
    }

    pub fn upsert_adapter_model_pref(
        &self,
        adapter_id: &str,
        default_model: Option<&str>,
        default_effort: Option<&str>,
    ) -> Result<AdapterModelPref, StoreError> {
        let clean = |value: Option<&str>| {
            value
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
        };
        let pref = AdapterModelPref {
            adapter_id: adapter_id.into(),
            default_model: clean(default_model),
            default_effort: clean(default_effort),
        };
        let conn = self.conn()?;
        write_model_pref(&conn, &pref)?;
        Ok(pref)
    }

    pub fn get_adapter_model_pref(
        &self,
        adapter_id: &str,
    ) -> Result<Option<AdapterModelPref>, StoreError> {
        let conn = self.conn()?;
        conn.query_row(
            "SELECT adapter_id, default_model, default_effort
             FROM adapter_model_prefs WHERE adapter_id = ?1",
            params![adapter_id],
            |row| {
                Ok(AdapterModelPref {
                    adapter_id: row.get(0)?,
                    default_model: row.get(1)?,
                    default_effort: row.get(2)?,
                })
            },
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn list_adapter_model_prefs(&self) -> Result<Vec<AdapterModelPref>, StoreError> {
        let conn = self.conn()?;
        list_model_prefs(&conn)
    }
}

fn query_quota_pref(row: &rusqlite::Row<'_>) -> rusqlite::Result<AdapterQuotaPref> {
    Ok(AdapterQuotaPref {
        adapter_id: row.get(0)?,
        plan_label: row.get(1)?,
        exact_probe_opt_in: row.get(2)?,
        manual_5h_pct: row.get(3)?,
        manual_weekly_pct: row.get(4)?,
    })
}

pub(crate) fn list_quota_prefs(conn: &Connection) -> Result<Vec<AdapterQuotaPref>, StoreError> {
    let mut stmt = conn.prepare(
        "SELECT adapter_id, plan_label, exact_probe_opt_in, manual_5h_pct, manual_weekly_pct
         FROM adapter_quota_prefs ORDER BY adapter_id",
    )?;
    let rows = stmt.query_map([], query_quota_pref)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

pub(crate) fn write_quota_pref(
    conn: &Connection,
    pref: &AdapterQuotaPref,
) -> Result<(), StoreError> {
    conn.execute(
        "INSERT INTO adapter_quota_prefs (
            adapter_id, plan_label, exact_probe_opt_in, manual_5h_pct, manual_weekly_pct, updated_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(adapter_id) DO UPDATE SET
            plan_label = excluded.plan_label,
            exact_probe_opt_in = excluded.exact_probe_opt_in,
            manual_5h_pct = excluded.manual_5h_pct,
            manual_weekly_pct = excluded.manual_weekly_pct,
            updated_at = excluded.updated_at",
        params![
            pref.adapter_id,
            pref.plan_label,
            pref.exact_probe_opt_in,
            pref.manual_5h_pct,
            pref.manual_weekly_pct,
            now_millis()
        ],
    )?;
    Ok(())
}

pub(crate) fn list_model_prefs(conn: &Connection) -> Result<Vec<AdapterModelPref>, StoreError> {
    let mut stmt = conn.prepare(
        "SELECT adapter_id, default_model, default_effort FROM adapter_model_prefs ORDER BY adapter_id",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(AdapterModelPref {
            adapter_id: row.get(0)?,
            default_model: row.get(1)?,
            default_effort: row.get(2)?,
        })
    })?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

pub(crate) fn write_model_pref(
    conn: &Connection,
    pref: &AdapterModelPref,
) -> Result<(), StoreError> {
    conn.execute(
        "INSERT INTO adapter_model_prefs (adapter_id, default_model, default_effort, updated_at)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(adapter_id) DO UPDATE SET
            default_model = excluded.default_model,
            default_effort = excluded.default_effort,
            updated_at = excluded.updated_at",
        params![
            pref.adapter_id,
            pref.default_model,
            pref.default_effort,
            now_millis()
        ],
    )?;
    Ok(())
}

fn validate_fact<'a, 'b>(
    category: &'a str,
    summary: &'b str,
) -> Result<(&'a str, &'b str), StoreError> {
    let category = category.trim();
    let summary = summary.trim();
    if category.is_empty() || category.len() > 64 || summary.is_empty() || summary.len() > 4_000 {
        return Err(StoreError::Conflict(
            "Provide a category and a fact of up to 4,000 bytes.".into(),
        ));
    }
    Ok((category, summary))
}

fn hide_duplicate_facts(
    conn: &rusqlite::Connection,
    fact: &ProjectFact,
    include_user_facts: bool,
    timestamp: i64,
) -> Result<(), StoreError> {
    conn.execute(
        "UPDATE project_facts SET superseded_at = ?1
         WHERE project_id = ?2 AND id <> ?3 AND superseded_at IS NULL
           AND category = ?4 COLLATE NOCASE AND trim(summary) = trim(?5) COLLATE NOCASE
           AND (author_id <> 'user' OR ?6)",
        params![
            timestamp,
            fact.project_id,
            fact.id,
            fact.category,
            fact.summary,
            include_user_facts
        ],
    )?;
    Ok(())
}

fn map_fact_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ProjectFact> {
    Ok(ProjectFact {
        id: row.get(0)?,
        project_id: row.get(1)?,
        category: row.get(2)?,
        summary: row.get(3)?,
        author_id: row.get(4)?,
        created_at: row.get(5)?,
        updated_at: row.get(6)?,
    })
}

fn map_job_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<EpisodicJob> {
    let modules_str: String = row.get(10)?;
    Ok(EpisodicJob {
        id: row.get(0)?,
        project_id: row.get(1)?,
        session_id: row.get(2)?,
        turn_id: row.get(3)?,
        member_handle: row.get(4)?,
        adapter_id: row.get(5)?,
        model: row.get(6)?,
        effort: row.get(7)?,
        task_brief: row.get(8)?,
        curated_summary: row.get(9)?,
        touched_modules: serde_json::from_str(&modules_str).unwrap_or_default(),
        outcome: row.get(11)?,
        curated_by: row.get(12)?,
        pinned: row.get::<_, i32>(13)? != 0,
        created_at: row.get(14)?,
    })
}

/// Converts arbitrary text (including `@codex`, `src/auth/jwt-v2.rs`, etc.) into a safe
/// SQLite FTS5 query string so punctuation never causes FTS5 parse errors.
pub fn build_safe_fts_query(raw: &str) -> Option<String> {
    let mut seen = HashSet::new();
    let mut terms = Vec::new();
    for tok in raw.split(|c: char| !c.is_alphanumeric() && c != '_') {
        let lower = tok.to_ascii_lowercase();
        if lower.len() >= 2 && seen.insert(lower.clone()) {
            terms.push(format!("\"{lower}\""));
            if terms.len() >= 12 {
                break;
            }
        }
    }
    if terms.is_empty() {
        None
    } else {
        Some(terms.join(" OR "))
    }
}
