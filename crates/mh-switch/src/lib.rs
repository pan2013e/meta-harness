pub mod error;
pub mod mcp;
pub mod orchestrator;
pub mod router;
pub mod skills;

pub use error::SwitchError;
pub use mcp::{execute_mcp_tool, list_switch_tools};
pub use router::{SwitchEngine, TurnPurpose};
pub use skills::{
    BUILTIN_META_HARNESS_DOCS_FILES, BUILTIN_META_HARNESS_DOCS_ID, ensure_builtin_skills,
    list_skills_with_roots, project_skill_state_key, read_skill_file_from_summary,
    reset_builtin_skill,
};
