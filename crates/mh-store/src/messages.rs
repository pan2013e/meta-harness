use rusqlite::{OptionalExtension, params};
use serde_json::Value;

use mh_core::{ChatMessage, MessageKind, SenderKind};

use crate::{Store, StoreError, now_millis};

#[derive(Debug, Clone)]
pub struct NewMessageParams {
    pub id: Option<String>,
    pub session_id: String,
    pub chain_id: String,
    pub parent_msg_id: Option<String>,
    pub sender_kind: SenderKind,
    pub sender_id: String,
    pub recipients: Vec<String>,
    pub kind: MessageKind,
    pub content: String,
    pub turn_id: Option<String>,
    pub metadata: Value,
}

impl Store {
    pub fn append_message(&self, msg: NewMessageParams) -> Result<ChatMessage, StoreError> {
        let id = msg
            .id
            .unwrap_or_else(|| format!("msg_{}", uuid::Uuid::new_v4().simple()));
        let created_at = now_millis();
        let recipients_json = serde_json::to_string(&msg.recipients)?;
        let metadata_json = serde_json::to_string(&msg.metadata)?;

        let conn = self.conn()?;
        conn.execute(
            "INSERT INTO messages (
                id, session_id, chain_id, parent_msg_id, sender_kind,
                sender_id, recipients_json, kind, content, turn_id,
                metadata_json, created_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                id,
                msg.session_id,
                msg.chain_id,
                msg.parent_msg_id,
                msg.sender_kind.as_str(),
                msg.sender_id,
                recipients_json,
                msg.kind.as_str(),
                msg.content,
                msg.turn_id,
                metadata_json,
                created_at,
            ],
        )?;
        let seq = conn.last_insert_rowid();
        conn.execute(
            "UPDATE sessions SET updated_at = ?1 WHERE id = ?2",
            params![created_at, msg.session_id],
        )?;

        Ok(ChatMessage {
            seq,
            id,
            session_id: msg.session_id,
            chain_id: msg.chain_id,
            parent_msg_id: msg.parent_msg_id,
            sender_kind: msg.sender_kind,
            sender_id: msg.sender_id,
            recipients: msg.recipients,
            kind: msg.kind,
            content: msg.content,
            turn_id: msg.turn_id,
            metadata: msg.metadata,
            created_at,
        })
    }

    pub fn update_message(
        &self,
        id: &str,
        content: &str,
        metadata: Option<Value>,
    ) -> Result<ChatMessage, StoreError> {
        let conn = self.conn()?;
        if let Some(meta) = metadata {
            let meta_json = serde_json::to_string(&meta)?;
            conn.execute(
                "UPDATE messages SET content = ?1, metadata_json = ?2 WHERE id = ?3",
                params![content, meta_json, id],
            )?;
        } else {
            conn.execute(
                "UPDATE messages SET content = ?1 WHERE id = ?2",
                params![content, id],
            )?;
        }
        drop(conn);
        self.get_message(id)
    }

    pub fn get_message(&self, id: &str) -> Result<ChatMessage, StoreError> {
        let conn = self.conn()?;
        conn.query_row(
            "SELECT seq, id, session_id, chain_id, parent_msg_id, sender_kind,
                    sender_id, recipients_json, kind, content, turn_id,
                    metadata_json, created_at
             FROM messages WHERE id = ?1",
            params![id],
            map_message_row,
        )
        .optional()?
        .ok_or_else(|| StoreError::NotFound(format!("message {id}")))
    }

    pub fn list_messages(
        &self,
        session_id: &str,
        before_seq: Option<i64>,
        limit: u32,
    ) -> Result<Vec<ChatMessage>, StoreError> {
        let conn = self.conn()?;
        let limit_i = i64::from(limit.clamp(1, 500));
        let mut rows_vec = if let Some(before) = before_seq {
            let mut stmt = conn.prepare(
                "SELECT seq, id, session_id, chain_id, parent_msg_id, sender_kind,
                        sender_id, recipients_json, kind, content, turn_id,
                        metadata_json, created_at
                 FROM messages
                 WHERE session_id = ?1 AND seq < ?2
                 ORDER BY seq DESC
                 LIMIT ?3",
            )?;
            let mapped = stmt.query_map(params![session_id, before, limit_i], map_message_row)?;
            mapped.collect::<Result<Vec<_>, _>>()?
        } else {
            let mut stmt = conn.prepare(
                "SELECT seq, id, session_id, chain_id, parent_msg_id, sender_kind,
                        sender_id, recipients_json, kind, content, turn_id,
                        metadata_json, created_at
                 FROM messages
                 WHERE session_id = ?1
                 ORDER BY seq DESC
                 LIMIT ?2",
            )?;
            let mapped = stmt.query_map(params![session_id, limit_i], map_message_row)?;
            mapped.collect::<Result<Vec<_>, _>>()?
        };
        rows_vec.reverse();
        Ok(rows_vec)
    }

    /// Returns unseen group-chat messages after `since_seq` in ascending chronological order.
    pub fn list_unseen_deltas(
        &self,
        session_id: &str,
        since_seq: i64,
        limit: u32,
    ) -> Result<Vec<ChatMessage>, StoreError> {
        let conn = self.conn()?;
        let limit_i = i64::from(limit.clamp(1, 200));
        let mut stmt = conn.prepare(
            "SELECT seq, id, session_id, chain_id, parent_msg_id, sender_kind,
                    sender_id, recipients_json, kind, content, turn_id,
                    metadata_json, created_at
             FROM messages
             WHERE session_id = ?1 AND seq > ?2
               AND kind IN ('text', 'consult_thread', 'merge_notice', 'dispatch_card')
             ORDER BY seq ASC
             LIMIT ?3",
        )?;
        let rows = stmt.query_map(params![session_id, since_seq, limit_i], map_message_row)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }
}

fn map_message_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ChatMessage> {
    let recipients_str: String = row.get(7)?;
    let metadata_str: String = row.get(11)?;
    let recipients: Vec<String> = serde_json::from_str(&recipients_str).unwrap_or_default();
    let metadata: Value =
        serde_json::from_str(&metadata_str).unwrap_or_else(|_| Value::Object(Default::default()));

    Ok(ChatMessage {
        seq: row.get(0)?,
        id: row.get(1)?,
        session_id: row.get(2)?,
        chain_id: row.get(3)?,
        parent_msg_id: row.get(4)?,
        sender_kind: SenderKind::parse(&row.get::<_, String>(5)?),
        sender_id: row.get(6)?,
        recipients,
        kind: MessageKind::parse(&row.get::<_, String>(8)?),
        content: row.get(9)?,
        turn_id: row.get(10)?,
        metadata,
        created_at: row.get(12)?,
    })
}
