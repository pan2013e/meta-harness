use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::Duration;

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::Mutex;

#[derive(Default)]
struct State {
    cwd: Option<PathBuf>,
    mcp_server: Option<Value>,
    session_id: Option<String>,
    model: String,
    effort: String,
    cancel_requested: bool,
}

/// Runs the deterministic offline Mock ACP Agent over stdio.
pub async fn run_mock_acp_agent_stdio() {
    let stdin = tokio::io::stdin();
    let stdout = Arc::new(Mutex::new(tokio::io::stdout()));
    let state = Arc::new(Mutex::new(State {
        model: "mock-fast".into(),
        effort: "medium".into(),
        ..Default::default()
    }));
    let req_seq = Arc::new(AtomicI64::new(1000));
    let pending_client_calls = Arc::new(Mutex::new(HashMap::<
        i64,
        tokio::sync::oneshot::Sender<Value>,
    >::new()));

    let mut lines = BufReader::new(stdin).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(msg) = serde_json::from_str::<Value>(trimmed) else {
            continue;
        };

        if msg.get("method").is_none() && msg.get("id").is_some() {
            if let Some(id) = msg.get("id").and_then(|v| v.as_i64())
                && let Some(tx) = pending_client_calls.lock().await.remove(&id)
            {
                let _ = tx.send(msg);
            }
            continue;
        }

        let id = msg.get("id").cloned();
        let method = msg
            .get("method")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        let params = msg.get("params").cloned().unwrap_or(Value::Null);

        match method {
            "initialize" => {
                if let Some(id) = id {
                    write_json(
                        &stdout,
                        json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "result": {
                                "protocolVersion": 1,
                                "agentInfo": { "name": "mock-acp-agent", "version": "0.1.0" },
                                "agentCapabilities": { "loadSession": true }
                            }
                        }),
                    )
                    .await;
                }
            }
            "session/new" | "session/load" => {
                let cwd = params
                    .get("cwd")
                    .and_then(|v| v.as_str())
                    .map(PathBuf::from);
                let mcp = params
                    .get("mcpServers")
                    .and_then(|v| v.as_array())
                    .and_then(|a| a.first())
                    .cloned();
                let sid = params
                    .get("sessionId")
                    .and_then(|v| v.as_str())
                    .map(str::to_string)
                    .unwrap_or_else(|| format!("mock_{}", uuid::Uuid::new_v4().simple()));
                {
                    let mut st = state.lock().await;
                    st.cwd = cwd;
                    st.mcp_server = mcp;
                    st.session_id = Some(sid.clone());
                }
                if let Some(id) = id {
                    let st = state.lock().await;
                    write_json(
                        &stdout,
                        json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "result": {
                                "sessionId": sid,
                                "configOptions": [
                                    {
                                        "id": "model",
                                        "category": "model",
                                        "name": "Model",
                                        "currentValue": st.model,
                                        "options": [
                                            { "value": "mock-fast", "name": "Mock Fast" },
                                            { "value": "mock-pro", "name": "Mock Pro" }
                                        ]
                                    },
                                    {
                                        "id": "reasoning_effort",
                                        "category": "thought_level",
                                        "name": "Reasoning Effort",
                                        "currentValue": st.effort,
                                        "options": [
                                            { "value": "low", "name": "Low" },
                                            { "value": "medium", "name": "Medium" },
                                            { "value": "high", "name": "High" }
                                        ]
                                    }
                                ]
                            }
                        }),
                    )
                    .await;
                }
            }
            "session/set_config_option" => {
                let cid = params
                    .get("configId")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();
                let val = params
                    .get("value")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();
                {
                    let mut st = state.lock().await;
                    if cid == "model" {
                        st.model = val.to_string();
                    } else if cid == "reasoning_effort" {
                        st.effort = val.to_string();
                    }
                }
                if let Some(id) = id {
                    write_json(&stdout, json!({ "jsonrpc": "2.0", "id": id, "result": {} })).await;
                }
            }
            "session/cancel" => {
                state.lock().await.cancel_requested = true;
            }
            "session/prompt" => {
                let st_clone = Arc::clone(&state);
                let out_clone = Arc::clone(&stdout);
                let req_seq_clone = Arc::clone(&req_seq);
                let pending_clone = Arc::clone(&pending_client_calls);
                tokio::spawn(async move {
                    handle_prompt(
                        id,
                        params,
                        st_clone,
                        out_clone,
                        req_seq_clone,
                        pending_clone,
                    )
                    .await;
                });
            }
            _ => {
                if let Some(id) = id {
                    write_json(&stdout, json!({ "jsonrpc": "2.0", "id": id, "result": {} })).await;
                }
            }
        }
    }
}

async fn handle_prompt(
    id: Option<Value>,
    params: Value,
    state: Arc<Mutex<State>>,
    stdout: Arc<Mutex<tokio::io::Stdout>>,
    req_seq: Arc<AtomicI64>,
    pending_calls: Arc<Mutex<HashMap<i64, tokio::sync::oneshot::Sender<Value>>>>,
) {
    let prompt_text = params
        .get("prompt")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|b| b.get("text").and_then(|t| t.as_str()))
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default();

    let (sid, cwd, mcp_spec, model, effort) = {
        let mut st = state.lock().await;
        st.cancel_requested = false;
        (
            st.session_id
                .clone()
                .unwrap_or_else(|| "mock_sess_1".into()),
            st.cwd.clone().unwrap_or_else(|| PathBuf::from(".")),
            st.mcp_server.clone(),
            st.model.clone(),
            st.effort.clone(),
        )
    };

    let directive_text = strip_context_blocks(&prompt_text);

    let routing = prompt_text.contains("<recipient_routing_request>");
    if routing || prompt_text.contains("<session_title_request>") {
        let reply = if routing {
            let input = prompt_text
                .lines()
                .find_map(|line| serde_json::from_str::<Value>(line).ok());
            let recipient = input
                .as_ref()
                .and_then(|input| input.get("candidates")?.as_array()?.last())
                .and_then(Value::as_str);
            json!({ "recipients": recipient.into_iter().collect::<Vec<_>>() }).to_string()
        } else {
            let user_line = prompt_text
                .lines()
                .rev()
                .find(|line| !line.trim().is_empty() && !line.trim_start().starts_with('<'))
                .unwrap_or("Mock Session");
            let words: Vec<&str> = user_line.split_whitespace().take(4).collect();
            if words.is_empty() {
                "Mock Session Title".to_string()
            } else {
                words.join(" ")
            }
        };
        emit_update(
            &stdout,
            &sid,
            "agent_message_chunk",
            json!({ "type": "text", "text": reply }),
        )
        .await;
        if let Some(id) = id {
            write_json(
                &stdout,
                json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": { "stopReason": "end_turn" }
                }),
            )
            .await;
        }
        return;
    }

    if directive_text.contains("[MOCK:429]") {
        if let Some(id) = id {
            write_json(
                &stdout,
                json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": { "code": 429, "message": "429 rate limit exceeded on subscription" }
                }),
            )
            .await;
        }
        return;
    }

    if let Some(ms_str) = extract_tag_attr(&directive_text, "[MOCK:SLEEP", "ms") {
        let ms: u64 = ms_str.parse().unwrap_or(2000);
        let gate =
            extract_tag_attr(&directive_text, "[MOCK:SLEEP", "until").map(|path| cwd.join(path));
        let steps = (ms / 25).max(1);
        for _ in 0..steps {
            if gate.as_ref().is_some_and(|path| path.is_file()) {
                break;
            }
            if state.lock().await.cancel_requested {
                if let Some(id) = id {
                    write_json(
                        &stdout,
                        json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "result": { "stopReason": "cancelled" }
                        }),
                    )
                    .await;
                }
                return;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }

    emit_update(
        &stdout,
        &sid,
        "agent_thought_chunk",
        json!({ "type": "text", "text": format!("Mock planning on {model}:{effort}...") }),
    )
    .await;

    if let (Some(path), Some(content)) = (
        extract_tag_attr(&directive_text, "[MOCK:WRITE", "path"),
        extract_tag_attr(&directive_text, "[MOCK:WRITE", "content"),
    ) {
        let _ = call_client(
            &stdout,
            &req_seq,
            &pending_calls,
            "fs/write_text_file",
            json!({
                "sessionId": sid,
                "path": path,
                "content": content.replace("\\n", "\n"),
            }),
        )
        .await;
    }

    if let Some(cmd_str) = extract_tag_attr(&directive_text, "[MOCK:SH", "cmd") {
        let create_resp = call_client(
            &stdout,
            &req_seq,
            &pending_calls,
            "terminal/create",
            json!({ "sessionId": sid, "command": cmd_str }),
        )
        .await;
        if let Some(tid) = create_resp
            .get("result")
            .and_then(|r| r.get("terminalId"))
            .and_then(|v| v.as_str())
        {
            let _ = call_client(
                &stdout,
                &req_seq,
                &pending_calls,
                "terminal/wait_for_exit",
                json!({ "sessionId": sid, "terminalId": tid }),
            )
            .await;
            let _ = call_client(
                &stdout,
                &req_seq,
                &pending_calls,
                "terminal/release",
                json!({ "sessionId": sid, "terminalId": tid }),
            )
            .await;
        }
    }

    if let Some(conflict_path) = extract_tag_attr(&directive_text, "[MOCK:RESOLVE_CONFLICT", "path")
    {
        let content = extract_tag_attr(&directive_text, "[MOCK:RESOLVE_CONFLICT", "content")
            .unwrap_or_else(|| "resolved content\n".into())
            .replace("\\n", "\n");
        let full = cwd.join(&conflict_path);
        let _ = tokio::fs::write(&full, content).await;
        let _ = Command::new("git")
            .arg("-C")
            .arg(&cwd)
            .args(["add", "-A"])
            .status()
            .await;
        let _ = Command::new("git")
            .arg("-C")
            .arg(&cwd)
            .env("GIT_AUTHOR_NAME", "MockOrch")
            .env("GIT_COMMITTER_NAME", "MockOrch")
            .env("GIT_AUTHOR_EMAIL", "mock@local")
            .env("GIT_COMMITTER_EMAIL", "mock@local")
            .args(["commit", "-m", "chore: resolve conflict"])
            .status()
            .await;
    }

    let mut reply = format!("Completed on {model}:{effort}.");

    if let Some(tool) = extract_tag_attr(&directive_text, "[MOCK:MCP", "tool") {
        let args_json = extract_tag_json(&directive_text, "[MOCK:MCP", "args").unwrap_or(json!({}));
        if let Some(spec) = mcp_spec.as_ref() {
            let allowed = if extract_tag_attr(&directive_text, "[MOCK:MCP", "permission").as_deref()
                == Some("required")
            {
                let server = spec["name"].as_str().unwrap_or_default();
                let name = format!("mcp_{server}_{tool}");
                let response = call_client(
                    &stdout,
                    &req_seq,
                    &pending_calls,
                    "session/request_permission",
                    json!({
                        "sessionId": sid,
                        "toolCall": {
                            "name": name,
                            "kind": "other",
                            "title": format!("Run {name}"),
                            "rawInput": &args_json,
                        },
                        "options": [
                            {"optionId": "always", "kind": "allow_always"},
                            {"optionId": "once", "kind": "allow_once"},
                            {"optionId": "reject", "kind": "reject_once"}
                        ],
                    }),
                )
                .await;
                response["result"]["outcome"]["optionId"] == "once"
            } else {
                true
            };
            if allowed {
                match invoke_mcp_bridge(spec, &cwd, &tool, args_json).await {
                    Ok(res) => reply = format!("MCP {tool} ok: {res}"),
                    Err(e) => reply = format!("MCP {tool} err: {e}"),
                }
            } else {
                reply = format!("MCP {tool} permission declined");
            }
        }
    }

    if directive_text.contains("[MOCK:ORCH_AUTO]")
        && let Some(spec) = mcp_spec.as_ref()
    {
        let quotas = invoke_mcp_bridge(spec, &cwd, "quota_get_status", json!({}))
            .await
            .unwrap_or(Value::Null);
        let target = pick_highest_headroom_member(&quotas, "codex");
        let (m, e) = if directive_text.to_ascii_lowercase().contains("typo") {
            ("mock-fast", "low")
        } else {
            ("mock-pro", "high")
        };
        let brief = extract_user_prompt_section(&directive_text);
        let _ = invoke_mcp_bridge(
            spec,
            &cwd,
            "orchestrator_dispatch",
            json!({
                "rationale": format!("Selected @{target} with {m}:{e} based on headroom"),
                "dispatches": [{
                    "target": target,
                    "model": m,
                    "effort": e,
                    "task_brief": brief,
                    "mode": "parallel"
                }]
            }),
        )
        .await;
        reply = format!("Dispatched task to @{target} ({m}:{e}).");
    }

    if (prompt_text.contains("<memory_curation_request>")
        || directive_text.contains("[MOCK:CURATE]"))
        && let Some(spec) = mcp_spec.as_ref()
    {
        let curated_member = prompt_text
            .split("A job by @")
            .nth(1)
            .and_then(|s| s.split_whitespace().next())
            .unwrap_or("codex")
            .to_string();
        let _ = invoke_mcp_bridge(
            spec,
            &cwd,
            "memory_curate_job",
            json!({
                "member_handle": curated_member,
                "task_brief": "Mock curated job",
                "curated_summary": format!("@{curated_member} completed module update cleanly"),
                "touched_modules": ["src/lib.rs"],
                "outcome": "completed"
            }),
        )
        .await;
        reply = "Curated episodic memory.".into();
    }

    if let Some(custom_reply) = extract_tag_attr(&directive_text, "[MOCK:REPLY", "text") {
        reply = custom_reply;
    }

    emit_update(
        &stdout,
        &sid,
        "agent_message_chunk",
        json!({ "type": "text", "text": reply }),
    )
    .await;

    if let Some(id) = id {
        write_json(
            &stdout,
            json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": { "stopReason": "end_turn" }
            }),
        )
        .await;
    }
}

async fn invoke_mcp_bridge(
    spec: &Value,
    cwd: &Path,
    tool_name: &str,
    arguments: Value,
) -> Result<Value, String> {
    let command = spec
        .get("command")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "missing mcp command".to_string())?;
    let args: Vec<String> = spec
        .get("args")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();

    let mut cmd = Command::new(command);
    cmd.args(&args)
        .current_dir(cwd)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped());
    if let Some(envs) = spec.get("env").and_then(|v| v.as_array()) {
        for e in envs {
            if let (Some(k), Some(v)) = (
                e.get("name").and_then(|x| x.as_str()),
                e.get("value").and_then(|x| x.as_str()),
            ) {
                cmd.env(k, v);
            }
        }
    }

    let mut child = cmd.spawn().map_err(|e| e.to_string())?;
    let mut c_in = child.stdin.take().ok_or("no mcp stdin")?;
    let c_out = child.stdout.take().ok_or("no mcp stdout")?;

    let req1 = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": { "name": "mock-acp-agent", "version": "0.1.0" }
        }
    });
    let req2 = json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/call",
        "params": {
            "name": tool_name,
            "arguments": arguments
        }
    });

    c_in.write_all(format!("{req1}\n{req2}\n").as_bytes())
        .await
        .map_err(|e| e.to_string())?;
    c_in.flush().await.map_err(|e| e.to_string())?;
    drop(c_in);

    let mut lines = BufReader::new(c_out).lines();
    let mut call_resp = Value::Null;
    while let Ok(Some(l)) = lines.next_line().await {
        if let Ok(v) = serde_json::from_str::<Value>(l.trim())
            && v.get("id").and_then(|x| x.as_i64()) == Some(2)
        {
            call_resp = v;
            break;
        }
    }
    let _ = child.kill().await;
    if let Some(err) = call_resp.get("error") {
        return Err(err.to_string());
    }
    Ok(call_resp.get("result").cloned().unwrap_or(Value::Null))
}

fn pick_highest_headroom_member(quotas: &Value, default_target: &str) -> String {
    let mut best_id = None;
    let mut best_pct = -1.0f64;
    let arr = quotas
        .get("content")
        .and_then(|c| c.as_array())
        .and_then(|a| a.first())
        .and_then(|t| t.get("text"))
        .and_then(|s| s.as_str())
        .and_then(|s| serde_json::from_str::<Value>(s).ok())
        .unwrap_or_else(|| quotas.clone());

    if let Some(list) = arr.get("quotas").and_then(|q| q.as_array()) {
        for item in list {
            let pct = item
                .get("window_5h_remaining_pct")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0);
            let limited = item.get("rate_limited_until").is_some_and(|v| !v.is_null());
            if !limited && pct > best_pct {
                best_pct = pct;
                best_id = item
                    .get("adapter_id")
                    .and_then(|v| v.as_str())
                    .map(str::to_string);
            }
        }
    }
    best_id.unwrap_or_else(|| default_target.to_string())
}

fn extract_user_prompt_section(text: &str) -> String {
    text.lines()
        .find(|l| !l.trim().is_empty() && !l.trim().starts_with('<'))
        .unwrap_or("execute requested task")
        .to_string()
}

async fn call_client(
    stdout: &Arc<Mutex<tokio::io::Stdout>>,
    req_seq: &Arc<AtomicI64>,
    pending: &Arc<Mutex<HashMap<i64, tokio::sync::oneshot::Sender<Value>>>>,
    method: &str,
    params: Value,
) -> Value {
    let id = req_seq.fetch_add(1, Ordering::Relaxed);
    let (tx, rx) = tokio::sync::oneshot::channel();
    pending.lock().await.insert(id, tx);
    write_json(
        stdout,
        json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params
        }),
    )
    .await;
    rx.await.unwrap_or(Value::Null)
}

async fn emit_update(
    stdout: &Arc<Mutex<tokio::io::Stdout>>,
    sid: &str,
    update_kind: &str,
    content: Value,
) {
    write_json(
        stdout,
        json!({
            "jsonrpc": "2.0",
            "method": "session/update",
            "params": {
                "sessionId": sid,
                "update": {
                    "sessionUpdate": update_kind,
                    "content": content
                }
            }
        }),
    )
    .await;
}

async fn write_json(stdout: &Arc<Mutex<tokio::io::Stdout>>, val: Value) {
    let mut guard = stdout.lock().await;
    if let Ok(line) = serde_json::to_string(&val) {
        let _ = guard.write_all(line.as_bytes()).await;
        let _ = guard.write_all(b"\n").await;
        let _ = guard.flush().await;
    }
}

fn strip_context_blocks(raw: &str) -> String {
    let mut s = raw.to_string();
    for (open, close) in [
        ("<persona_instructions", "</persona_instructions>"),
        ("<group_chat_updates", "</group_chat_updates>"),
        ("<orchestrator_telemetry>", "</orchestrator_telemetry>"),
    ] {
        while let Some(start) = s.find(open) {
            if let Some(end_rel) = s[start..].find(close) {
                s.replace_range(start..start + end_rel + close.len(), "");
            } else {
                break;
            }
        }
    }
    s
}

fn extract_tag_attr(text: &str, tag: &str, attr: &str) -> Option<String> {
    let start = text.find(tag)?;
    let slice = &text[start..];
    let end = slice.find(']')?;
    let body = &slice[tag.len()..end];

    let quoted_key = format!("{attr}=\"");
    if let Some(k_pos) = body.find(&quoted_key) {
        let rest = &body[k_pos + quoted_key.len()..];
        let q_end = rest.find('"')?;
        return Some(rest[..q_end].to_string());
    }

    let bare_key = format!("{attr}=");
    let k_pos = body.find(&bare_key)?;
    let rest = body[k_pos + bare_key.len()..].trim_start();
    let val: String = rest
        .chars()
        .take_while(|c| !c.is_whitespace() && *c != ']')
        .collect();
    if val.is_empty() { None } else { Some(val) }
}

fn extract_tag_json(text: &str, tag: &str, attr: &str) -> Option<Value> {
    let start = text.find(tag)?;
    let body = &text[start + tag.len()..];
    let key = format!("{attr}=");
    let k_pos = body.find(&key)?;
    if body[..k_pos].contains(']') {
        return None;
    }
    let rest = body[k_pos + key.len()..].trim_start();
    let mut values = serde_json::Deserializer::from_str(rest).into_iter::<Value>();
    let value = values.next()?.ok()?;
    rest[values.byte_offset()..]
        .trim_start()
        .starts_with(']')
        .then_some(value)
}
