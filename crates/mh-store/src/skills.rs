use std::collections::HashMap;

use mh_core::SkillSource;
use rusqlite::{OptionalExtension, params};

use crate::{Store, StoreError, now_millis};

#[derive(Debug, Clone)]
pub struct SkillStateRow {
    pub state_key: String,
    pub source: SkillSource,
    pub origin: Option<String>,
    pub enabled: bool,
    pub builtin_hash: Option<String>,
    pub updated_at: i64,
}

impl Store {
    pub fn list_skill_states(&self) -> Result<HashMap<String, SkillStateRow>, StoreError> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT state_key, source, origin, enabled, builtin_hash, updated_at FROM skill_states",
        )?;
        let rows = stmt.query_map([], |row| {
            let source_raw: String = row.get(1)?;
            let enabled_i: i64 = row.get(3)?;
            Ok(SkillStateRow {
                state_key: row.get(0)?,
                source: SkillSource::parse(&source_raw),
                origin: row.get(2)?,
                enabled: enabled_i != 0,
                builtin_hash: row.get(4)?,
                updated_at: row.get(5)?,
            })
        })?;
        let mut map = HashMap::new();
        for item in rows {
            let row = item?;
            map.insert(row.state_key.to_ascii_lowercase(), row);
        }
        Ok(map)
    }

    pub fn get_skill_state(&self, state_key: &str) -> Result<Option<SkillStateRow>, StoreError> {
        let conn = self.conn()?;
        conn.query_row(
            "SELECT state_key, source, origin, enabled, builtin_hash, updated_at
             FROM skill_states WHERE state_key = ?1",
            [state_key],
            |row| {
                let source_raw: String = row.get(1)?;
                let enabled_i: i64 = row.get(3)?;
                Ok(SkillStateRow {
                    state_key: row.get(0)?,
                    source: SkillSource::parse(&source_raw),
                    origin: row.get(2)?,
                    enabled: enabled_i != 0,
                    builtin_hash: row.get(4)?,
                    updated_at: row.get(5)?,
                })
            },
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn upsert_skill_state(&self, row: &SkillStateRow) -> Result<(), StoreError> {
        self.conn()?.execute(
            "INSERT INTO skill_states (state_key, source, origin, enabled, builtin_hash, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(state_key) DO UPDATE SET
                 source = excluded.source,
                 origin = COALESCE(excluded.origin, skill_states.origin),
                 enabled = excluded.enabled,
                 builtin_hash = COALESCE(excluded.builtin_hash, skill_states.builtin_hash),
                 updated_at = excluded.updated_at",
            params![
                row.state_key,
                row.source.as_str(),
                row.origin,
                if row.enabled { 1 } else { 0 },
                row.builtin_hash,
                row.updated_at,
            ],
        )?;
        Ok(())
    }

    pub fn set_skill_enabled(
        &self,
        state_key: &str,
        source: SkillSource,
        origin: Option<&str>,
        enabled: bool,
    ) -> Result<(), StoreError> {
        let now = now_millis();
        self.conn()?.execute(
            "INSERT INTO skill_states (state_key, source, origin, enabled, builtin_hash, updated_at)
             VALUES (?1, ?2, ?3, ?4, NULL, ?5)
             ON CONFLICT(state_key) DO UPDATE SET
                 enabled = excluded.enabled,
                 updated_at = excluded.updated_at",
            params![
                state_key,
                source.as_str(),
                origin,
                if enabled { 1 } else { 0 },
                now,
            ],
        )?;
        Ok(())
    }

    pub fn delete_skill_state(&self, state_key: &str) -> Result<(), StoreError> {
        self.conn()?
            .execute("DELETE FROM skill_states WHERE state_key = ?1", [state_key])?;
        Ok(())
    }
}
