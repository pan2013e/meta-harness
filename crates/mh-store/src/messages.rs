use rusqlite::{OptionalExtension, params};
use serde_json::Value;

use mh_core::{ChatImage, ChatMessage, MessageKind, SenderKind};

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
        self.append_message_with_images(msg, &[])
    }

    pub fn append_message_with_images(
        &self,
        mut msg: NewMessageParams,
        images: &[ChatImage],
    ) -> Result<ChatMessage, StoreError> {
        let id = msg
            .id
            .unwrap_or_else(|| format!("msg_{}", uuid::Uuid::new_v4().simple()));
        let recipients_json = serde_json::to_string(&msg.recipients)?;
        if !images.is_empty() {
            msg.metadata["images"] = serde_json::json!(
                images
                    .iter()
                    .map(|image| &image.mime_type)
                    .collect::<Vec<_>>()
            );
        }
        let metadata_json = serde_json::to_string(&msg.metadata)?;

        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        let created_at = now_millis();
        tx.execute(
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
        let seq = tx.last_insert_rowid();
        for (index, image) in images.iter().enumerate() {
            tx.execute("INSERT INTO message_images (message_id, image_index, mime_type, bytes) VALUES (?1, ?2, ?3, ?4)",
                params![id, index, image.mime_type, image.bytes])?;
        }
        tx.execute(
            "UPDATE sessions SET updated_at = ?1 WHERE id = ?2",
            params![created_at, msg.session_id],
        )?;
        tx.commit()?;

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

    pub fn get_message_image(
        &self,
        session_id: &str,
        message_id: &str,
        index: u32,
    ) -> Result<Option<ChatImage>, StoreError> {
        Ok(self.conn()?.query_row(
            "SELECT i.mime_type, i.bytes FROM message_images i JOIN messages m ON m.id = i.message_id
             WHERE m.session_id = ?1 AND m.id = ?2 AND i.image_index = ?3",
            params![session_id, message_id, index], map_image_row).optional()?)
    }

    pub fn list_chain_images(
        &self,
        session_id: &str,
        chain_id: &str,
    ) -> Result<Vec<ChatImage>, StoreError> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT i.mime_type, i.bytes FROM message_images i JOIN messages m ON m.id = i.message_id
             WHERE m.session_id = ?1 AND m.chain_id = ?2 AND m.sender_kind = 'user'
             ORDER BY i.image_index",
        )?;
        stmt.query_map(params![session_id, chain_id], map_image_row)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(Into::into)
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

    /// Persists a coalesced streaming update and returns its revision for replay and reconnect.
    pub fn append_message_delta(
        &self,
        id: &str,
        delta: &str,
        replace: bool,
    ) -> Result<u64, StoreError> {
        self.conn()?
            .query_row(
                "UPDATE messages
             SET content = CASE WHEN ?2 THEN ?3 ELSE content || ?3 END,
                 metadata_json = json_set(metadata_json, '$.stream_revision',
                     COALESCE(json_extract(metadata_json, '$.stream_revision'), 0) + 1)
             WHERE id = ?1 AND json_extract(metadata_json, '$.streaming') = 1
             RETURNING json_extract(metadata_json, '$.stream_revision')",
                params![id, replace, delta],
                |row| row.get(0),
            )
            .optional()?
            .ok_or_else(|| StoreError::NotFound(format!("streaming message {id}")))
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

fn map_image_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ChatImage> {
    Ok(ChatImage {
        mime_type: row.get(0)?,
        bytes: row.get(1)?,
    })
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
