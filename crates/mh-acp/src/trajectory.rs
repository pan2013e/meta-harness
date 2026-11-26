use serde_json::{Value, json};
use similar::TextDiff;

use mh_core::TrajectoryEventKind;

pub(crate) type TrajectoryUpdate = (TrajectoryEventKind, Value);

/// Strips Meta-Harness internal worktree path prefixes (`.../.meta-harness/worktrees/<sid>/_integration/`)
/// so filenames in diffs and trajectories stay compact and readable across OSes.
pub fn clean_worktree_display_path(raw: &str) -> String {
    let slash = raw.replace('\\', "/");
    if let Some((_, rest)) = slash.split_once(".meta-harness/worktrees/") {
        let mut parts = rest.splitn(3, '/');
        let _sid = parts.next();
        let _wt_dir = parts.next();
        if let Some(rel) = parts.next()
            && !rel.is_empty()
        {
            return rel.to_string();
        }
    }
    slash
}

/// ACP tool updates are patches to an existing tool call. Codex additionally streams
/// terminal output through ACP metadata, using the same tool call ID for each chunk.
pub(crate) fn parse_tool_update(update: &Value, is_new: bool) -> Vec<TrajectoryUpdate> {
    let id = update.get("toolCallId").and_then(Value::as_str);
    let meta = update.get("_meta").unwrap_or(&Value::Null);
    let mut events = Vec::new();

    if let Some(info) = meta.get("terminal_info") {
        events.push((
            TrajectoryEventKind::Terminal,
            json!({
                "tool_call_id": id, "terminal_id": info.get("terminal_id"), "cwd": info.get("cwd"),
            }),
        ));
    }
    if let Some(delta) = meta
        .get("terminal_output_delta")
        .or_else(|| meta.get("terminal_output"))
        && let Some(text) = delta
            .get("data")
            .and_then(Value::as_str)
            .filter(|text| !text.is_empty())
    {
        events.push((
            TrajectoryEventKind::Terminal,
            json!({
                "tool_call_id": id,
                "terminal_id": delta.get("terminal_id"),
                "output_delta": text,
            }),
        ));
    }
    if let Some(exit) = meta.get("terminal_exit") {
        events.push((
            TrajectoryEventKind::Terminal,
            json!({
                "tool_call_id": id,
                "terminal_id": exit.get("terminal_id"),
                "exit_code": exit.get("exit_code"),
                "signal": exit.get("signal"),
                "status": "exited",
            }),
        ));
    }

    let mut tool = serde_json::Map::new();
    for field in [
        "title",
        "name",
        "kind",
        "status",
        "rawInput",
        "rawOutput",
        "locations",
    ] {
        if let Some(value) = update.get(field).filter(|value| !value.is_null()) {
            tool.insert(field.into(), value.clone());
        }
    }
    if let Some(content) = update.get("content").and_then(Value::as_array) {
        let mut other = Vec::new();
        for item in content {
            if let Some(diff) = parse_diff(item, id) {
                events.push((TrajectoryEventKind::Diff, diff));
            } else {
                other.push(item.clone());
            }
        }
        if !other.is_empty() {
            tool.insert("content".into(), Value::Array(other));
        }
    }
    if is_new || !tool.is_empty() {
        if let Some(id) = id {
            tool.insert("toolCallId".into(), json!(id));
        }
        events.insert(0, (TrajectoryEventKind::ToolCall, Value::Object(tool)));
    }
    events
}

fn parse_diff(item: &Value, tool_call_id: Option<&str>) -> Option<Value> {
    if item.get("type").and_then(Value::as_str) != Some("diff") {
        return None;
    }
    let raw_path = item.get("path")?.as_str()?;
    let path = clean_worktree_display_path(raw_path);
    let old = item
        .get("oldText")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let new = item
        .get("newText")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let from = format!("a/{path}");
    let to = format!("b/{path}");
    let body = TextDiff::from_lines(old, new)
        .unified_diff()
        .header(&from, &to)
        .to_string();
    let diff = format!("diff --git {from} {to}\n{body}");
    Some(json!({"tool_call_id": tool_call_id, "path": path, "diff": diff}))
}

pub(crate) fn push_update(updates: &mut Vec<TrajectoryUpdate>, next: TrajectoryUpdate) {
    if next.0 == TrajectoryEventKind::Terminal
        && let Some((TrajectoryEventKind::Terminal, last)) = updates.last_mut()
        && last.get("terminal_id") == next.1.get("terminal_id")
        && last.get("tool_call_id") == next.1.get("tool_call_id")
        && let Some(text) = next.1.get("output_delta").and_then(Value::as_str)
        && let Some(Value::String(previous)) = last.get_mut("output_delta")
    {
        previous.push_str(text);
    } else {
        updates.push(next);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codex_output_chunks_are_terminal_events_and_keep_their_tool_link() {
        let mut updates = Vec::new();
        for text in ["a\r\n", "b"] {
            for update in parse_tool_update(
                &json!({
                    "sessionUpdate": "tool_call_update", "toolCallId": "exec-1",
                    "_meta": {"terminal_output_delta": {"terminal_id": "exec-1", "data": text}},
                }),
                false,
            ) {
                push_update(&mut updates, update);
            }
        }
        assert_eq!(
            updates,
            vec![(
                TrajectoryEventKind::Terminal,
                json!({
                    "tool_call_id": "exec-1", "terminal_id": "exec-1", "output_delta": "a\r\nb",
                })
            )]
        );
    }

    #[test]
    fn acp_diff_content_becomes_a_file_edit_and_tool_status_survives() {
        let events = parse_tool_update(
            &json!({
                "toolCallId": "edit-1", "status": "completed", "content": [
                    {"type": "diff", "path": "src/main.rs", "oldText": "before\n", "newText": "after\n"},
                    {"type": "content", "content": {"type": "text", "text": "Done"}},
                ]
            }),
            false,
        );
        assert_eq!(events[0].0, TrajectoryEventKind::ToolCall);
        assert_eq!(events[0].1["status"], "completed");
        assert_eq!(events[1].0, TrajectoryEventKind::Diff);
        assert!(
            events[1].1["diff"]
                .as_str()
                .unwrap()
                .contains("-before\n+after")
        );
    }
}
