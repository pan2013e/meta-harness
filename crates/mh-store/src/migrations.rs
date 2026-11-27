use rusqlite::Connection;

use crate::StoreError;

const MIGRATION_V1: &str = r#"
CREATE TABLE IF NOT EXISTS schema_migrations (
    version INTEGER PRIMARY KEY
);

CREATE TABLE IF NOT EXISTS projects (
    id              TEXT PRIMARY KEY,
    name            TEXT NOT NULL,
    root_path       TEXT NOT NULL UNIQUE,
    is_git_repo     INTEGER NOT NULL DEFAULT 1,
    created_at      INTEGER NOT NULL,
    last_opened_at  INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS sessions (
    id                      TEXT PRIMARY KEY,
    project_id              TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    title                   TEXT NOT NULL,
    base_branch             TEXT,
    base_commit_sha         TEXT,
    integration_branch      TEXT,
    integration_wt_path     TEXT,
    isolation_mode          TEXT NOT NULL DEFAULT 'worktree_per_member',
    orchestrator_member     TEXT,
    confirm_before_dispatch INTEGER NOT NULL DEFAULT 0,
    max_hops                INTEGER NOT NULL DEFAULT 6,
    permission_policy       TEXT NOT NULL DEFAULT 'worktree_smart',
    working_summary         TEXT NOT NULL DEFAULT '',
    created_at              INTEGER NOT NULL,
    updated_at              INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS session_members (
    session_id          TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    handle              TEXT NOT NULL,
    adapter_id          TEXT NOT NULL,
    display_name        TEXT NOT NULL,
    role                TEXT NOT NULL DEFAULT 'executor',
    persona_prompt      TEXT NOT NULL DEFAULT '',
    worktree_branch     TEXT,
    worktree_path       TEXT,
    acp_session_id      TEXT,
    active_model        TEXT,
    active_effort       TEXT,
    last_seen_msg_seq   INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (session_id, handle)
);

CREATE TABLE IF NOT EXISTS session_permission_rules (
    id              TEXT PRIMARY KEY,
    session_id      TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    tool_kind       TEXT NOT NULL,
    pattern         TEXT NOT NULL,
    decision        TEXT NOT NULL,
    created_at      INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS messages (
    seq             INTEGER PRIMARY KEY AUTOINCREMENT,
    id              TEXT NOT NULL UNIQUE,
    session_id      TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    chain_id        TEXT NOT NULL,
    parent_msg_id   TEXT,
    sender_kind     TEXT NOT NULL,
    sender_id       TEXT NOT NULL,
    recipients_json TEXT NOT NULL DEFAULT '[]',
    kind            TEXT NOT NULL,
    content         TEXT NOT NULL,
    turn_id         TEXT,
    metadata_json   TEXT NOT NULL DEFAULT '{}',
    created_at      INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_messages_session_seq ON messages(session_id, seq);

CREATE TABLE IF NOT EXISTS turn_trajectories (
    turn_id         TEXT PRIMARY KEY,
    session_id      TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    member_handle   TEXT NOT NULL,
    adapter_id      TEXT NOT NULL,
    model           TEXT,
    effort          TEXT,
    worktree_branch TEXT,
    head_commit_sha TEXT,
    status          TEXT NOT NULL,
    summary         TEXT NOT NULL DEFAULT '',
    touched_files   TEXT NOT NULL DEFAULT '[]',
    diff_stat_json  TEXT NOT NULL DEFAULT '{}',
    started_at      INTEGER NOT NULL,
    finished_at     INTEGER
);
CREATE INDEX IF NOT EXISTS idx_traj_session_started ON turn_trajectories(session_id, started_at DESC);

CREATE TABLE IF NOT EXISTS trajectory_events (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    turn_id         TEXT NOT NULL REFERENCES turn_trajectories(turn_id) ON DELETE CASCADE,
    event_kind      TEXT NOT NULL,
    payload_json    TEXT NOT NULL,
    created_at      INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_traj_events_turn ON trajectory_events(turn_id, id);

CREATE TABLE IF NOT EXISTS episodic_jobs (
    id              TEXT PRIMARY KEY,
    project_id      TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    session_id      TEXT REFERENCES sessions(id) ON DELETE SET NULL,
    turn_id         TEXT,
    member_handle   TEXT NOT NULL,
    adapter_id      TEXT NOT NULL,
    model           TEXT,
    effort          TEXT,
    task_brief      TEXT NOT NULL,
    curated_summary TEXT NOT NULL,
    touched_modules TEXT NOT NULL DEFAULT '[]',
    outcome         TEXT NOT NULL,
    curated_by      TEXT NOT NULL DEFAULT 'orchestrator',
    pinned          INTEGER NOT NULL DEFAULT 0,
    created_at      INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_episodic_jobs_project ON episodic_jobs(project_id, pinned DESC, created_at DESC);

CREATE VIRTUAL TABLE IF NOT EXISTS episodic_jobs_fts USING fts5(
    job_id UNINDEXED,
    project_id UNINDEXED,
    member_handle,
    adapter_id,
    task_brief,
    curated_summary,
    touched_modules
);

CREATE TABLE IF NOT EXISTS project_facts (
    id              TEXT PRIMARY KEY,
    project_id      TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    category        TEXT NOT NULL,
    summary         TEXT NOT NULL,
    author_id       TEXT NOT NULL,
    created_at      INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_project_facts_proj ON project_facts(project_id, created_at DESC);

CREATE TABLE IF NOT EXISTS adapter_quota_prefs (
    adapter_id         TEXT PRIMARY KEY,
    plan_label         TEXT,
    exact_probe_opt_in INTEGER NOT NULL DEFAULT 0,
    manual_5h_pct      REAL,
    manual_weekly_pct  REAL,
    updated_at         INTEGER NOT NULL
);
"#;

const MIGRATION_V2: &str = r#"
CREATE TABLE IF NOT EXISTS adapter_model_prefs (
    adapter_id     TEXT PRIMARY KEY,
    default_model  TEXT,
    default_effort TEXT,
    updated_at     INTEGER NOT NULL
);
"#;

pub fn configure_connection(conn: &Connection) -> Result<(), StoreError> {
    conn.execute_batch(
        "PRAGMA journal_mode = WAL;
         PRAGMA synchronous = NORMAL;
         PRAGMA foreign_keys = ON;
         PRAGMA busy_timeout = 5000;",
    )?;
    Ok(())
}

pub fn apply_migrations(conn: &Connection) -> Result<(), StoreError> {
    configure_connection(conn)?;
    conn.execute_batch(MIGRATION_V1)?;
    conn.execute(
        "INSERT OR IGNORE INTO schema_migrations (version) VALUES (1)",
        [],
    )?;
    conn.execute_batch(MIGRATION_V2)?;
    conn.execute(
        "INSERT OR IGNORE INTO schema_migrations (version) VALUES (2)",
        [],
    )?;
    let current_ver: i64 = conn
        .query_row(
            "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);
    if current_ver < 3 {
        conn.execute_batch(
            "PRAGMA foreign_keys = OFF;
             BEGIN;
             CREATE TABLE sessions_v3 (
                 id                      TEXT PRIMARY KEY,
                 project_id              TEXT REFERENCES projects(id) ON DELETE CASCADE,
                 title                   TEXT NOT NULL,
                 base_branch             TEXT,
                 base_commit_sha         TEXT,
                 integration_branch      TEXT,
                 integration_wt_path     TEXT,
                 isolation_mode          TEXT NOT NULL DEFAULT 'worktree_per_member',
                 orchestrator_member     TEXT,
                 confirm_before_dispatch INTEGER NOT NULL DEFAULT 0,
                 max_hops                INTEGER NOT NULL DEFAULT 6,
                 permission_policy       TEXT NOT NULL DEFAULT 'worktree_smart',
                 working_summary         TEXT NOT NULL DEFAULT '',
                 archived_at             INTEGER,
                 created_at              INTEGER NOT NULL,
                 updated_at              INTEGER NOT NULL
             );
             INSERT INTO sessions_v3 (
                 id, project_id, title, base_branch, base_commit_sha,
                 integration_branch, integration_wt_path, isolation_mode,
                 orchestrator_member, confirm_before_dispatch, max_hops,
                 permission_policy, working_summary, archived_at, created_at, updated_at
             )
             SELECT
                 id, project_id, title, base_branch, base_commit_sha,
                 integration_branch, integration_wt_path, isolation_mode,
                 orchestrator_member, confirm_before_dispatch, max_hops,
                 permission_policy, working_summary, NULL, created_at, updated_at
             FROM sessions;
             DROP TABLE sessions;
             ALTER TABLE sessions_v3 RENAME TO sessions;
             CREATE INDEX IF NOT EXISTS idx_sessions_project ON sessions(project_id, updated_at DESC);
             INSERT OR IGNORE INTO schema_migrations (version) VALUES (3);
             COMMIT;
             PRAGMA foreign_keys = ON;",
        )?;
    }
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS global_personas (
             handle         TEXT PRIMARY KEY COLLATE NOCASE,
             adapter_id     TEXT NOT NULL,
             display_name   TEXT NOT NULL,
             persona_prompt TEXT NOT NULL,
             active_model   TEXT,
             active_effort  TEXT
         );
         INSERT OR IGNORE INTO schema_migrations (version) VALUES (4);",
    )?;
    if current_ver < 5 {
        conn.execute_batch(
            "BEGIN;
             CREATE TABLE session_harness_conversations (
                 session_id     TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
                 adapter_id     TEXT NOT NULL,
                 acp_session_id TEXT NOT NULL,
                 PRIMARY KEY (session_id, adapter_id, acp_session_id)
             );
             INSERT OR IGNORE INTO session_harness_conversations
                 SELECT session_id, adapter_id, acp_session_id FROM session_members
                 WHERE acp_session_id IS NOT NULL;
             CREATE TRIGGER track_harness_conversation
             AFTER UPDATE OF acp_session_id ON session_members
             WHEN NEW.acp_session_id IS NOT NULL AND NEW.acp_session_id IS NOT OLD.acp_session_id
             BEGIN
                 INSERT OR IGNORE INTO session_harness_conversations
                     VALUES (NEW.session_id, NEW.adapter_id, NEW.acp_session_id);
             END;
             INSERT INTO schema_migrations (version) VALUES (5);
             COMMIT;",
        )?;
    }
    if current_ver < 6 {
        conn.execute_batch(
            "BEGIN;
             CREATE TABLE IF NOT EXISTS skill_states (
                 state_key      TEXT PRIMARY KEY COLLATE NOCASE,
                 source         TEXT NOT NULL DEFAULT 'local',
                 origin         TEXT,
                 enabled        INTEGER NOT NULL DEFAULT 1,
                 builtin_hash   TEXT,
                 updated_at     INTEGER NOT NULL
             );
             INSERT OR IGNORE INTO schema_migrations (version) VALUES (6);
             COMMIT;",
        )?;
    }
    if current_ver < 7 {
        conn.execute_batch(
            "BEGIN;
             CREATE TABLE message_images (
                 message_id TEXT NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
                 image_index INTEGER NOT NULL,
                 mime_type TEXT NOT NULL,
                 bytes BLOB NOT NULL,
                 PRIMARY KEY (message_id, image_index)
             );
             CREATE INDEX idx_messages_session_chain ON messages(session_id, chain_id);
             INSERT INTO schema_migrations (version) VALUES (7);
             COMMIT;",
        )?;
    }
    if current_ver < 8 {
        conn.execute_batch(
            "BEGIN;
             ALTER TABLE project_facts ADD COLUMN updated_at INTEGER NOT NULL DEFAULT 0;
             ALTER TABLE project_facts ADD COLUMN superseded_at INTEGER;
             UPDATE project_facts SET updated_at = created_at;
             CREATE INDEX idx_project_facts_updated ON project_facts(project_id, updated_at DESC);
             CREATE TABLE notification_preferences (
                 singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
                 approval_requests INTEGER NOT NULL,
                 turn_completions INTEGER NOT NULL
             );
             INSERT INTO schema_migrations (version) VALUES (8);
             COMMIT;",
        )?;
    }
    Ok(())
}
