use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

#[derive(Debug, Serialize, Deserialize)]
pub struct BridgeCallPayload {
    pub session_id: String,
    pub caller: String,
    pub action: String,
    #[serde(default)]
    pub tool_name: Option<String>,
    #[serde(default)]
    pub arguments: Value,
}

/// Runs the stdio MCP JSON-RPC 2.0 server bridge, proxying tool listing and invocations
/// to the local Meta-Harness server authenticated by `MH_BRIDGE_TOKEN`.
pub async fn run_mcp_bridge(url: String, session_id: String, caller: String) -> anyhow::Result<()> {
    let token = std::env::var("MH_BRIDGE_TOKEN").unwrap_or_default();
    let http = reqwest::Client::builder().no_proxy().build()?;
    let stdin = tokio::io::stdin();
    let mut stdout = tokio::io::stdout();
    let mut lines = BufReader::new(stdin).lines();

    while let Ok(Some(line)) = lines.next_line().await {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(msg) = serde_json::from_str::<Value>(trimmed) else {
            continue;
        };

        let id = msg.get("id").cloned();
        let method = msg
            .get("method")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        let params = msg.get("params").cloned().unwrap_or(Value::Null);

        let Some(id_val) = id else {
            // Notifications (like `notifications/initialized`) require no response
            continue;
        };

        let resp = match method {
            "initialize" => json!({
                "jsonrpc": "2.0",
                "id": id_val,
                "result": {
                    "protocolVersion": "2024-11-05",
                    "capabilities": { "tools": {} },
                    "serverInfo": {
                        "name": "meta-harness-switch",
                        "version": env!("CARGO_PKG_VERSION")
                    }
                }
            }),
            "ping" => json!({ "jsonrpc": "2.0", "id": id_val, "result": {} }),
            "tools/list" => {
                let payload = BridgeCallPayload {
                    session_id: session_id.clone(),
                    caller: caller.clone(),
                    action: "list".into(),
                    tool_name: None,
                    arguments: Value::Null,
                };
                match post_internal(&http, &url, &token, &payload).await {
                    Ok(val) => json!({
                        "jsonrpc": "2.0",
                        "id": id_val,
                        "result": { "tools": val.get("tools").cloned().unwrap_or(json!([])) }
                    }),
                    Err(e) => json!({
                        "jsonrpc": "2.0",
                        "id": id_val,
                        "error": { "code": -32000, "message": e }
                    }),
                }
            }
            "tools/call" => {
                let tool_name = params
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string();
                let arguments = params.get("arguments").cloned().unwrap_or(json!({}));
                let payload = BridgeCallPayload {
                    session_id: session_id.clone(),
                    caller: caller.clone(),
                    action: "call".into(),
                    tool_name: Some(tool_name),
                    arguments,
                };
                match post_internal(&http, &url, &token, &payload).await {
                    Ok(val) => {
                        let is_err = val
                            .get("isError")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(false);
                        let text = if is_err {
                            val.get("error")
                                .and_then(|v| v.as_str())
                                .unwrap_or("tool error")
                                .to_string()
                        } else {
                            serde_json::to_string_pretty(&val).unwrap_or_else(|_| val.to_string())
                        };
                        json!({
                            "jsonrpc": "2.0",
                            "id": id_val,
                            "result": {
                                "content": [{ "type": "text", "text": text }],
                                "isError": is_err
                            }
                        })
                    }
                    Err(e) => json!({
                        "jsonrpc": "2.0",
                        "id": id_val,
                        "result": {
                            "content": [{ "type": "text", "text": e }],
                            "isError": true
                        }
                    }),
                }
            }
            other => json!({
                "jsonrpc": "2.0",
                "id": id_val,
                "error": { "code": -32601, "message": format!("method not supported: {other}") }
            }),
        };

        let out_line = serde_json::to_string(&resp)?;
        stdout.write_all(out_line.as_bytes()).await?;
        stdout.write_all(b"\n").await?;
        stdout.flush().await?;
    }

    Ok(())
}

async fn post_internal(
    http: &reqwest::Client,
    url: &str,
    token: &str,
    payload: &BridgeCallPayload,
) -> Result<Value, String> {
    let resp = http
        .post(url)
        .bearer_auth(token)
        .json(payload)
        .send()
        .await
        .map_err(|e| format!("failed to reach switch endpoint: {e}"))?;

    let status = resp.status();
    let body: Value = resp
        .json()
        .await
        .map_err(|e| format!("invalid JSON from switch: {e}"))?;
    if !status.is_success() {
        return Err(body
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("switch HTTP error")
            .to_string());
    }
    Ok(body)
}
