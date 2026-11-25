use rusqlite::{OptionalExtension, params};

use mh_core::Project;

use crate::{Store, StoreError, now_millis};

impl Store {
    pub fn upsert_project(
        &self,
        name: &str,
        root_path: &str,
        is_git_repo: bool,
    ) -> Result<Project, StoreError> {
        let now = now_millis();
        let conn = self.conn()?;

        if let Some(existing) = conn
            .query_row(
                "SELECT id, name, root_path, is_git_repo, created_at, last_opened_at FROM projects WHERE root_path = ?1",
                params![root_path],
                map_project_row,
            )
            .optional()?
        {
            conn.execute(
                "UPDATE projects SET name = ?1, is_git_repo = ?2, last_opened_at = ?3 WHERE id = ?4",
                params![name, i32::from(is_git_repo), now, existing.id],
            )?;
            return Ok(Project {
                name: name.to_string(),
                is_git_repo,
                last_opened_at: now,
                ..existing
            });
        }

        let id = format!("proj_{}", uuid::Uuid::new_v4().simple());
        conn.execute(
            "INSERT INTO projects (id, name, root_path, is_git_repo, created_at, last_opened_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![id, name, root_path, i32::from(is_git_repo), now, now],
        )?;

        Ok(Project {
            id,
            name: name.to_string(),
            root_path: root_path.to_string(),
            is_git_repo,
            created_at: now,
            last_opened_at: now,
        })
    }

    pub fn list_projects(&self) -> Result<Vec<Project>, StoreError> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT id, name, root_path, is_git_repo, created_at, last_opened_at
             FROM projects
             ORDER BY last_opened_at DESC",
        )?;
        let rows = stmt.query_map([], map_project_row)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn get_project(&self, id: &str) -> Result<Project, StoreError> {
        let conn = self.conn()?;
        conn.query_row(
            "SELECT id, name, root_path, is_git_repo, created_at, last_opened_at
             FROM projects WHERE id = ?1",
            params![id],
            map_project_row,
        )
        .optional()?
        .ok_or_else(|| StoreError::NotFound(format!("project {id}")))
    }

    pub fn set_project_git_repo(&self, id: &str, is_git_repo: bool) -> Result<Project, StoreError> {
        let conn = self.conn()?;
        let changed = conn.execute(
            "UPDATE projects SET is_git_repo = ?1 WHERE id = ?2",
            params![i32::from(is_git_repo), id],
        )?;
        if changed == 0 {
            return Err(StoreError::NotFound(format!("project {id}")));
        }
        drop(conn);
        self.get_project(id)
    }

    pub fn delete_project(&self, id: &str) -> Result<(), StoreError> {
        let conn = self.conn()?;
        let changed = conn.execute("DELETE FROM projects WHERE id = ?1", params![id])?;
        if changed == 0 {
            return Err(StoreError::NotFound(format!("project {id}")));
        }
        Ok(())
    }
}

fn map_project_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Project> {
    Ok(Project {
        id: row.get(0)?,
        name: row.get(1)?,
        root_path: row.get(2)?,
        is_git_repo: row.get::<_, i32>(3)? != 0,
        created_at: row.get(4)?,
        last_opened_at: row.get(5)?,
    })
}
