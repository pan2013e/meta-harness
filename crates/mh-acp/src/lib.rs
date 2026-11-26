pub mod broker;
pub mod client;
pub mod error;
pub mod mock_agent;
pub mod platform;
pub mod protocol;
pub mod quota;
pub mod supervisor;
mod trajectory;
pub mod worktree;

pub use broker::{
    BrokerCallbacks, MemberBrokerContext, PermissionResolution, WorkspaceBroker,
    suggest_command_pattern, wildcard_match,
};
pub use client::{AcpClient, PromptTextOutput, TurnStreamListener};
pub use error::AcpError;
pub use platform::{
    ProcessTreeGuard, default_shell, find_cli_binary, open_terminal_in_dir, open_url_in_browser,
    pick_folder_native, probe_executable, resolve_command,
};
pub use protocol::{McpEnvVar, McpServerSpec};
pub use quota::{AdapterProbeStatus, QuotaTracker, probe_installed_adapters};
pub use supervisor::{
    AcpSupervisor, ManagedMemberClient, MemberSpawnSpec, build_mcp_bridge_spec,
    builtin_adapter_catalog, resolve_adapter_command,
};
pub use trajectory::clean_worktree_display_path;
pub use worktree::{
    IntegrationWorktree, MemberWorktree, MergeOutcome, PromoteResult, RepoStatus,
    SessionDiffReport, checkpoint_session_worktrees, commit_worktree_if_dirty,
    ensure_member_worktree, ensure_session_integration_worktree, get_head_sha, get_session_diff,
    init_git_repo, inspect_repo, integrate_member_turn, member_worktree_path, promote_session,
    prune_session_worktree_checkouts, prune_session_worktrees, reset_worktree_to_sha,
    sanitize_ref_component, sync_member_worktree,
};
