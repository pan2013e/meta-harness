use thiserror::Error;

#[derive(Debug, Error)]
pub enum SwitchError {
    #[error("core error: {0}")]
    Core(#[from] mh_core::CoreError),
    #[error("store error: {0}")]
    Store(#[from] mh_store::StoreError),
    #[error("ACP error: {0}")]
    Acp(#[from] mh_acp::AcpError),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("validation error: {0}")]
    Validation(String),
    #[error("consult cycle detected: {0:?}")]
    CycleDetected(Vec<String>),
    #[error("hop limit reached for chain {chain_id} ({hops_used} hops)")]
    HopLimitReached { chain_id: String, hops_used: u32 },
    #[error("operation timed out: {0}")]
    Timeout(String),
    #[error("operation cancelled")]
    Cancelled,
}
