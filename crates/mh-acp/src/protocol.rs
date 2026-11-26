use serde::{Deserialize, Serialize};
use serde_json::Value;

use mh_core::{ConfigOptionChoice, DiscoveredConfigOption, PermissionPromptOption};

pub(crate) const CODEX_APP_SERVER_INITIALIZE: &str = concat!(
    r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"clientInfo":{"name":"meta-harness","version":""#,
    env!("CARGO_PKG_VERSION"),
    r#""},"capabilities":{"experimentalApi":true}}}"#,
    "\n",
);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub id: Value,
    pub method: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcNotification {
    pub jsonrpc: String,
    pub method: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcError {
    pub code: i64,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    pub id: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InitializeParams {
    pub protocol_version: u32,
    pub client_info: ClientInfo,
    pub client_capabilities: ClientCapabilities,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientInfo {
    pub name: String,
    pub version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ClientCapabilities {
    pub fs: FsCapabilities,
    pub terminal: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct FsCapabilities {
    pub read_text_file: bool,
    pub write_text_file: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct InitializeResult {
    #[serde(default)]
    pub protocol_version: u32,
    #[serde(default)]
    pub agent_info: Option<Value>,
    #[serde(default)]
    pub agent_capabilities: AgentCapabilities,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AgentCapabilities {
    #[serde(default)]
    pub load_session: bool,
    #[serde(default)]
    pub prompt_capabilities: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpEnvVar {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpServerSpec {
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    #[serde(default)]
    pub env: Vec<McpEnvVar>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewSessionParams {
    pub cwd: String,
    #[serde(default)]
    pub mcp_servers: Vec<McpServerSpec>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoadSessionParams {
    pub session_id: String,
    pub cwd: String,
    #[serde(default)]
    pub mcp_servers: Vec<McpServerSpec>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct NewSessionResult {
    pub session_id: String,
    #[serde(default)]
    pub config_options: Vec<Value>,
    #[serde(default)]
    pub models: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetConfigOptionParams {
    pub session_id: String,
    pub config_id: String,
    pub value: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptParams {
    pub session_id: String,
    pub prompt: Vec<ContentBlock>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    Text { text: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct PromptResult {
    #[serde(default)]
    pub stop_reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FsReadTextFileParams {
    pub session_id: String,
    pub path: String,
    #[serde(default)]
    pub line: Option<u32>,
    #[serde(default)]
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FsWriteTextFileParams {
    pub session_id: String,
    pub path: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalCreateParams {
    pub session_id: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub env: Vec<McpEnvVar>,
    #[serde(default)]
    pub output_byte_limit: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalIdParams {
    pub session_id: String,
    pub terminal_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestPermissionParams {
    pub session_id: String,
    #[serde(default)]
    pub tool_call: Value,
    #[serde(default)]
    pub options: Vec<Value>,
}

/// Normalizes heterogeneous `session/new.configOptions` entries across ACP adapters
/// (`codex-acp`, `claude-agent-acp`, `agy --acp`, mock agent).
pub fn parse_config_options(raw_options: &[Value]) -> Vec<DiscoveredConfigOption> {
    let mut parsed = Vec::new();
    for item in raw_options {
        let Some(id) = item
            .get("id")
            .or_else(|| item.get("configId"))
            .and_then(|v| v.as_str())
        else {
            continue;
        };
        let category = item
            .get("category")
            .and_then(|v| v.as_str())
            .map(str::to_string);
        let label = item
            .get("name")
            .or_else(|| item.get("label"))
            .and_then(|v| v.as_str())
            .map(str::to_string);
        let current_value = item
            .get("currentValue")
            .or_else(|| item.get("value"))
            .and_then(|v| v.as_str())
            .map(str::to_string);

        let mut choices = Vec::new();
        if let Some(arr) = item.get("options").and_then(|v| v.as_array()) {
            for ch in arr {
                if let Some(s) = ch.as_str() {
                    choices.push(ConfigOptionChoice {
                        value: s.to_string(),
                        label: s.to_string(),
                    });
                } else if let Some(val) = ch.get("value").and_then(|v| v.as_str()) {
                    let lbl = ch
                        .get("name")
                        .or_else(|| ch.get("label"))
                        .and_then(|v| v.as_str())
                        .unwrap_or(val);
                    choices.push(ConfigOptionChoice {
                        value: val.to_string(),
                        label: lbl.to_string(),
                    });
                } else if let Some(group_opts) = ch.get("options").and_then(|v| v.as_array()) {
                    for nested in group_opts {
                        if let Some(val) = nested.get("value").and_then(|v| v.as_str()) {
                            let lbl = nested
                                .get("name")
                                .or_else(|| nested.get("label"))
                                .and_then(|v| v.as_str())
                                .unwrap_or(val);
                            choices.push(ConfigOptionChoice {
                                value: val.to_string(),
                                label: lbl.to_string(),
                            });
                        }
                    }
                }
            }
        }

        parsed.push(DiscoveredConfigOption {
            id: id.to_string(),
            category,
            label,
            current_value,
            choices,
        });
    }
    parsed
}

/// Combines `session/new.configOptions` with fallback `session/new.models` state if present.
pub fn parse_new_session_options(res: &NewSessionResult) -> Vec<DiscoveredConfigOption> {
    let mut opts = parse_config_options(&res.config_options);
    let has_model = opts.iter().any(|o| {
        o.category
            .as_deref()
            .is_some_and(|c| c.eq_ignore_ascii_case("model"))
            || o.id.eq_ignore_ascii_case("model")
    });
    if !has_model && let Some(models_val) = res.models.as_ref() {
        let current_value = models_val
            .get("currentModelId")
            .or_else(|| models_val.get("current"))
            .and_then(|v| v.as_str())
            .map(str::to_string);
        let mut choices = Vec::new();
        if let Some(arr) = models_val
            .get("availableModels")
            .or_else(|| models_val.get("models"))
            .and_then(|v| v.as_array())
        {
            for item in arr {
                if let Some(id) = item
                    .get("modelId")
                    .or_else(|| item.get("id"))
                    .and_then(|v| v.as_str())
                {
                    let label = item
                        .get("name")
                        .or_else(|| item.get("label"))
                        .and_then(|v| v.as_str())
                        .unwrap_or(id);
                    choices.push(ConfigOptionChoice {
                        value: id.to_string(),
                        label: label.to_string(),
                    });
                }
            }
        }
        if current_value.is_some() || !choices.is_empty() {
            opts.push(DiscoveredConfigOption {
                id: "model".into(),
                category: Some("model".into()),
                label: Some("Model".into()),
                current_value,
                choices,
            });
        }
    }
    opts
}

/// Extracts text from heterogeneous ACP `session/update` content shapes
/// (`"text"`, `{ "type": "text", "text": "..." }`, or `[ { "type": "text", ... } ]`).
pub fn extract_update_text(content: Option<&Value>) -> String {
    let Some(val) = content else {
        return String::new();
    };
    match val {
        Value::String(s) => s.clone(),
        Value::Object(map) => map
            .get("text")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string(),
        Value::Array(arr) => {
            let mut buf = String::new();
            for item in arr {
                let piece = extract_update_text(Some(item));
                buf.push_str(&piece);
            }
            buf
        }
        _ => String::new(),
    }
}

pub fn parse_permission_options(raw: &[Value]) -> Vec<PermissionPromptOption> {
    let mut out = Vec::new();
    for opt in raw {
        let option_id = opt
            .get("optionId")
            .or_else(|| opt.get("id"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if option_id.is_empty() {
            continue;
        }
        let label = opt
            .get("name")
            .or_else(|| opt.get("label"))
            .and_then(|v| v.as_str())
            .unwrap_or(&option_id)
            .to_string();
        let kind = opt
            .get("kind")
            .and_then(|v| v.as_str())
            .unwrap_or("allow_once")
            .to_string();
        out.push(PermissionPromptOption {
            option_id,
            label,
            kind,
        });
    }
    if out.is_empty() {
        out.push(PermissionPromptOption {
            option_id: "allow_once".into(),
            label: "Allow Once".into(),
            kind: "allow_once".into(),
        });
        out.push(PermissionPromptOption {
            option_id: "reject_once".into(),
            label: "Reject".into(),
            kind: "reject_once".into(),
        });
    }
    out
}
