use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Validated local project workspace directory.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    pub id: String,
    pub name: String,
    pub root_path: String,
    pub is_git_repo: bool,
    pub created_at: i64,
    pub last_opened_at: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IsolationMode {
    WorktreePerMember,
    SharedCwd,
}

impl IsolationMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::WorktreePerMember => "worktree_per_member",
            Self::SharedCwd => "shared_cwd",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "shared_cwd" => Self::SharedCwd,
            _ => Self::WorktreePerMember,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionPolicy {
    WorktreeSmart,
    Strict,
    AutonomousSandbox,
}

impl PermissionPolicy {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::WorktreeSmart => "worktree_smart",
            Self::Strict => "strict",
            Self::AutonomousSandbox => "autonomous_sandbox",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "strict" => Self::Strict,
            "autonomous_sandbox" => Self::AutonomousSandbox,
            _ => Self::WorktreeSmart,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub project_id: String,
    pub title: String,
    pub base_branch: Option<String>,
    pub base_commit_sha: Option<String>,
    pub integration_branch: Option<String>,
    pub integration_wt_path: Option<String>,
    pub isolation_mode: IsolationMode,
    pub orchestrator_member: Option<String>,
    pub confirm_before_dispatch: bool,
    pub max_hops: u32,
    pub permission_policy: PermissionPolicy,
    pub working_summary: String,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemberRole {
    Orchestrator,
    Executor,
    Both,
}

impl MemberRole {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Orchestrator => "orchestrator",
            Self::Executor => "executor",
            Self::Both => "both",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "orchestrator" => Self::Orchestrator,
            "both" => Self::Both,
            _ => Self::Executor,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemberStatus {
    Hibernated,
    Spawning,
    Idle,
    Thinking,
    RunningTool,
    Consulting,
    WaitingPermission,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMember {
    pub session_id: String,
    pub handle: String,
    pub adapter_id: String,
    pub display_name: String,
    pub role: MemberRole,
    pub persona_prompt: String,
    pub worktree_branch: Option<String>,
    pub worktree_path: Option<String>,
    pub acp_session_id: Option<String>,
    pub active_model: Option<String>,
    pub active_effort: Option<String>,
    pub last_seen_msg_seq: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionRule {
    pub id: String,
    pub session_id: String,
    pub tool_kind: String,
    pub pattern: String,
    pub decision: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SenderKind {
    User,
    Member,
    Orchestrator,
    System,
}

impl SenderKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Member => "member",
            Self::Orchestrator => "orchestrator",
            Self::System => "system",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "user" => Self::User,
            "orchestrator" => Self::Orchestrator,
            "system" => Self::System,
            _ => Self::Member,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageKind {
    Text,
    DispatchCard,
    ConsultThread,
    ActivityPill,
    PermissionCard,
    MergeNotice,
    SystemNotice,
}

impl MessageKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::DispatchCard => "dispatch_card",
            Self::ConsultThread => "consult_thread",
            Self::ActivityPill => "activity_pill",
            Self::PermissionCard => "permission_card",
            Self::MergeNotice => "merge_notice",
            Self::SystemNotice => "system_notice",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "dispatch_card" => Self::DispatchCard,
            "consult_thread" => Self::ConsultThread,
            "activity_pill" => Self::ActivityPill,
            "permission_card" => Self::PermissionCard,
            "merge_notice" => Self::MergeNotice,
            "system_notice" => Self::SystemNotice,
            _ => Self::Text,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub seq: i64,
    pub id: String,
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
    pub created_at: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnStatus {
    Running,
    Completed,
    Cancelled,
    Failed,
}

impl TurnStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Cancelled => "cancelled",
            Self::Failed => "failed",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "completed" => Self::Completed,
            "cancelled" => Self::Cancelled,
            "failed" => Self::Failed,
            _ => Self::Running,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DiffStat {
    pub files_changed: u32,
    pub insertions: u32,
    pub deletions: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnTrajectory {
    pub turn_id: String,
    pub session_id: String,
    pub member_handle: String,
    pub adapter_id: String,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub worktree_branch: Option<String>,
    pub head_commit_sha: Option<String>,
    pub status: TurnStatus,
    pub summary: String,
    pub touched_files: Vec<String>,
    pub diff_stat: DiffStat,
    pub started_at: i64,
    pub finished_at: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrajectoryEventKind {
    Thought,
    Plan,
    ToolCall,
    Diff,
    Terminal,
    Error,
}

impl TrajectoryEventKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Thought => "thought",
            Self::Plan => "plan",
            Self::ToolCall => "tool_call",
            Self::Diff => "diff",
            Self::Terminal => "terminal",
            Self::Error => "error",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "thought" => Self::Thought,
            "plan" => Self::Plan,
            "diff" => Self::Diff,
            "terminal" => Self::Terminal,
            "error" => Self::Error,
            _ => Self::ToolCall,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrajectoryEvent {
    pub id: i64,
    pub turn_id: String,
    pub event_kind: TrajectoryEventKind,
    pub payload: Value,
    pub created_at: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrajectoryDetailLevel {
    Summary,
    ToolCalls,
    Diffs,
    Full,
}

impl TrajectoryDetailLevel {
    pub fn parse(s: &str) -> Self {
        match s {
            "tool_calls" => Self::ToolCalls,
            "diffs" => Self::Diffs,
            "full" => Self::Full,
            _ => Self::Summary,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuotaSource {
    LocalLogEstimate,
    LiveProbe,
    UserCalibrated,
    Cooldown429,
    Unavailable,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuotaWindow {
    pub id: String,
    pub label: String,
    pub remaining_pct: f32,
    pub resets_at: Option<i64>,
    pub window_mins: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuotaSnapshot {
    pub adapter_id: String,
    pub plan_label: Option<String>,
    pub window_5h_remaining_pct: Option<f32>,
    pub window_5h_resets_at: Option<i64>,
    pub weekly_remaining_pct: Option<f32>,
    pub weekly_resets_at: Option<i64>,
    pub local_5h_tokens_est: u64,
    pub rate_limited_until: Option<i64>,
    pub exact_probe_opt_in: bool,
    pub source: QuotaSource,
    #[serde(default)]
    pub windows: Vec<QuotaWindow>,
    #[serde(default)]
    pub credits_label: Option<String>,
    #[serde(default)]
    pub status_note: Option<String>,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EpisodicJob {
    pub id: String,
    pub project_id: String,
    pub session_id: Option<String>,
    pub turn_id: Option<String>,
    pub member_handle: String,
    pub adapter_id: String,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub task_brief: String,
    pub curated_summary: String,
    pub touched_modules: Vec<String>,
    pub outcome: String,
    pub curated_by: String,
    pub pinned: bool,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectFact {
    pub id: String,
    pub project_id: String,
    pub category: String,
    pub summary: String,
    pub author_id: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PromoteMode {
    Merge,
    Squash,
    Patch,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DispatchTaskSpec {
    pub target: String,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub task_brief: String,
    #[serde(default = "default_dispatch_mode")]
    pub mode: String,
}

fn default_dispatch_mode() -> String {
    "parallel".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigOptionChoice {
    pub value: String,
    pub label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveredConfigOption {
    pub id: String,
    pub category: Option<String>,
    pub label: Option<String>,
    pub current_value: Option<String>,
    pub choices: Vec<ConfigOptionChoice>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionPromptOption {
    pub option_id: String,
    pub label: String,
    pub kind: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingPermissionView {
    pub request_id: String,
    pub session_id: String,
    pub member_handle: String,
    pub turn_id: Option<String>,
    pub tool_kind: String,
    pub title: String,
    pub detail: String,
    pub suggested_pattern: Option<String>,
    pub options: Vec<PermissionPromptOption>,
    pub created_at: i64,
}

/// Multiplexed WebSocket event envelope broadcast from `mh-server` to web clients.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WsEvent {
    #[serde(rename = "chat.message_created")]
    ChatMessageCreated { message: ChatMessage },
    #[serde(rename = "chat.message_delta")]
    ChatMessageDelta {
        session_id: String,
        message_id: String,
        turn_id: String,
        member_handle: String,
        delta: String,
    },
    #[serde(rename = "chat.message_completed")]
    ChatMessageCompleted { message: ChatMessage },
    #[serde(rename = "switch.dispatch_decided")]
    SwitchDispatchDecided {
        session_id: String,
        dispatch_id: String,
        message_id: String,
        status: String,
        rationale: String,
        dispatches: Vec<DispatchTaskSpec>,
    },
    #[serde(rename = "switch.consult_updated")]
    SwitchConsultUpdated {
        session_id: String,
        message: ChatMessage,
    },
    #[serde(rename = "switch.activity_pill")]
    SwitchActivityPill {
        session_id: String,
        message: ChatMessage,
    },
    #[serde(rename = "switch.hop_limit_paused")]
    SwitchHopLimitPaused {
        session_id: String,
        chain_id: String,
        hops_used: u32,
        max_hops: u32,
        pending_targets: Vec<String>,
    },
    #[serde(rename = "member.status_changed")]
    MemberStatusChanged {
        session_id: String,
        handle: String,
        status: MemberStatus,
        active_model: Option<String>,
        active_effort: Option<String>,
        turn_id: Option<String>,
    },
    #[serde(rename = "trajectory.event")]
    TrajectoryEvent {
        session_id: String,
        turn_id: String,
        event: TrajectoryEvent,
    },
    #[serde(rename = "trajectory.counters")]
    TrajectoryCounters {
        session_id: String,
        turn_id: String,
        tool_calls: u32,
        file_edits: u32,
        thoughts: u32,
    },
    #[serde(rename = "git.branch_updated")]
    GitBranchUpdated {
        session_id: String,
        integration_branch: String,
        ahead_commits: u32,
        diff_stat: DiffStat,
    },
    #[serde(rename = "git.conflict_escalated")]
    GitConflictEscalated {
        session_id: String,
        member_handle: String,
        conflicting_files: Vec<String>,
    },
    #[serde(rename = "permission.requested")]
    PermissionRequested { prompt: PendingPermissionView },
    #[serde(rename = "permission.resolved")]
    PermissionResolved {
        session_id: String,
        request_id: String,
        decision: String,
    },
    #[serde(rename = "quota.updated")]
    QuotaUpdated { snapshot: QuotaSnapshot },
    #[serde(rename = "memory.job_curated")]
    MemoryJobCurated { job: EpisodicJob },
}
