use serde_json::Value;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AcpError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("core error: {0}")]
    Core(#[from] mh_core::CoreError),
    #[error("command not found on PATH: {0}")]
    CommandNotFound(String),
    #[error("invalid harness profile: {0}")]
    InvalidProfile(String),
    #[error("git command failed: {0}")]
    Git(String),
    #[error("git merge conflict in files: {conflicting_files:?}")]
    GitConflict {
        conflicting_files: Vec<String>,
        diff_preview: String,
    },
    #[error("permission denied: {0}")]
    PermissionDenied(String),
    #[error("ACP RPC error ({code}): {message}")]
    Rpc {
        code: i64,
        message: String,
        data: Option<Value>,
    },
    #[error("rate limited: {0}")]
    RateLimited(String),
    #[error("turn cancelled")]
    Cancelled,
    #[error("timeout: {0}")]
    Timeout(String),
    #[error("agent process exited unexpectedly: {0}")]
    ProcessExited(String),
}
