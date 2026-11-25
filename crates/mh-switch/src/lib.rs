pub mod error;
pub mod mcp;
pub mod orchestrator;
pub mod router;

pub use error::SwitchError;
pub use mcp::{execute_mcp_tool, list_switch_tools};
pub use router::{SwitchEngine, TurnPurpose};
