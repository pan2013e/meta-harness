use mh_core::{AdapterModelPref, GlobalPersona, SessionMember};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};

use crate::{AdapterQuotaPref, Store, StoreError, memory};

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct AppPreferences {
    pub global_personas: Vec<GlobalPersona>,
    pub adapter_defaults: Vec<AdapterModelPref>,
    pub quota_preferences: Vec<AdapterQuotaPref>,
}

impl Store {
    pub fn list_global_personas(&self) -> Result<Vec<GlobalPersona>, StoreError> {
        let conn = self.conn()?;
        list_personas(&conn)
    }

    pub fn save_global_persona(&self, persona: &GlobalPersona) -> Result<(), StoreError> {
        let conn = self.conn()?;
        write_persona(&conn, persona)
    }

    pub fn delete_global_persona(&self, handle: &str) -> Result<(), StoreError> {
        self.conn()?
            .execute("DELETE FROM global_personas WHERE handle = ?1", [handle])?;
        Ok(())
    }

    pub fn add_global_persona_to_session(
        &self,
        session_id: &str,
        handle: &str,
    ) -> Result<SessionMember, StoreError> {
        let changed = self.conn()?.execute(
            "INSERT INTO session_members (session_id, handle, adapter_id, display_name, role,
                                          persona_prompt, active_model, active_effort)
             SELECT s.id, p.handle, p.adapter_id, p.display_name, 'executor',
                    p.persona_prompt, p.active_model, p.active_effort
             FROM global_personas p CROSS JOIN sessions s
             WHERE p.handle = ?1 AND s.id = ?2 AND s.archived_at IS NULL
               AND NOT EXISTS (SELECT 1 FROM session_members
                   WHERE session_id = s.id AND handle = p.handle COLLATE NOCASE)",
            params![handle, session_id],
        )?;
        if changed == 0 {
            return Err(StoreError::Conflict(
                "The persona is already in this session, or the persona or session is unavailable."
                    .into(),
            ));
        }
        self.get_member(session_id, handle)
    }

    pub fn export_preferences(&self) -> Result<AppPreferences, StoreError> {
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        let prefs = AppPreferences {
            global_personas: list_personas(&tx)?,
            adapter_defaults: memory::list_model_prefs(&tx)?,
            quota_preferences: memory::list_quota_prefs(&tx)?,
        };
        tx.commit()?;
        Ok(prefs)
    }

    pub fn import_preferences(&self, prefs: &AppPreferences) -> Result<(), StoreError> {
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        for persona in &prefs.global_personas {
            write_persona(&tx, persona)?;
        }
        for pref in &prefs.adapter_defaults {
            memory::write_model_pref(&tx, pref)?;
        }
        for pref in &prefs.quota_preferences {
            memory::write_quota_pref(&tx, pref)?;
        }
        tx.commit()?;
        Ok(())
    }
}

fn list_personas(conn: &Connection) -> Result<Vec<GlobalPersona>, StoreError> {
    let mut stmt = conn.prepare(
        "SELECT handle, adapter_id, display_name, persona_prompt, active_model, active_effort
         FROM global_personas ORDER BY handle",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(GlobalPersona {
            handle: row.get(0)?,
            adapter_id: row.get(1)?,
            display_name: row.get(2)?,
            persona_prompt: row.get(3)?,
            active_model: row.get(4)?,
            active_effort: row.get(5)?,
        })
    })?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

fn write_persona(conn: &Connection, persona: &GlobalPersona) -> Result<(), StoreError> {
    conn.execute(
        "INSERT INTO global_personas (handle, adapter_id, display_name, persona_prompt, active_model, active_effort)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(handle) DO UPDATE SET adapter_id = excluded.adapter_id,
             display_name = excluded.display_name, persona_prompt = excluded.persona_prompt,
             active_model = excluded.active_model, active_effort = excluded.active_effort",
        params![persona.handle, persona.adapter_id, persona.display_name,
            persona.persona_prompt, persona.active_model, persona.active_effort],
    )?;
    Ok(())
}
