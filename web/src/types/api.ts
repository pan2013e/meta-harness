export type ThemeId = "obsidian" | "tokyo-night" | "catppuccin" | "daylight";

export type SkillSource = "builtin" | "local" | "git" | "custom" | "project";

export interface SkillSummary {
  id: string;
  name: string;
  description: string;
  source: SkillSource;
  origin?: string | null;
  enabled: boolean;
  builtin_modified: boolean;
  skill_dir: string;
  skill_md_path: string;
  files: string[];
  updated_at: number;
}

export interface SkillDetail {
  skill: SkillSummary;
  file_path: string;
  content: string;
}

export type IsolationMode = "worktree_per_member" | "shared_cwd";
export type PermissionPolicy = "worktree_smart" | "strict" | "autonomous_sandbox";
export type MemberRole = "orchestrator" | "executor" | "both";
export type MemberStatus =
  | "hibernated"
  | "spawning"
  | "idle"
  | "thinking"
  | "running_tool"
  | "consulting"
  | "waiting_permission"
  | "error";

export type SenderKind = "user" | "member" | "orchestrator" | "system";
export type MessageKind =
  | "text"
  | "dispatch_card"
  | "consult_thread"
  | "activity_pill"
  | "permission_card"
  | "merge_notice"
  | "system_notice";

export type TurnStatus = "running" | "completed" | "cancelled" | "failed";
export type TrajectoryEventKind =
  | "thought"
  | "plan"
  | "tool_call"
  | "diff"
  | "terminal"
  | "error";
export type PromoteMode = "merge" | "squash" | "patch";

export interface Project {
  id: string;
  name: string;
  root_path: string;
  is_git_repo: boolean;
  created_at: number;
  last_opened_at: number;
}

export interface Session {
  id: string;
  project_id: string | null;
  title: string;
  base_branch: string | null;
  base_commit_sha: string | null;
  integration_branch: string | null;
  integration_wt_path: string | null;
  isolation_mode: IsolationMode;
  orchestrator_member: string | null;
  confirm_before_dispatch: boolean;
  max_hops: number;
  permission_policy: PermissionPolicy;
  working_summary: string;
  archived_at?: number | null;
  created_at: number;
  updated_at: number;
}

export interface SessionMember {
  session_id: string;
  handle: string;
  adapter_id: string;
  display_name: string;
  role: MemberRole;
  persona_prompt: string;
  worktree_branch: string | null;
  worktree_path: string | null;
  acp_session_id: string | null;
  active_model: string | null;
  active_effort: string | null;
  last_seen_msg_seq: number;
}

export interface PermissionRule {
  id: string;
  session_id: string;
  tool_kind: string;
  pattern: string;
  decision: string;
  created_at: number;
}

export interface ChatMessage {
  seq: number;
  id: string;
  session_id: string;
  chain_id: string;
  parent_msg_id: string | null;
  sender_kind: SenderKind;
  sender_id: string;
  recipients: string[];
  kind: MessageKind;
  content: string;
  turn_id: string | null;
  metadata: Record<string, unknown>;
  created_at: number;
}

export interface DiffStat {
  files_changed: number;
  insertions: number;
  deletions: number;
}

export interface TurnTrajectory {
  turn_id: string;
  session_id: string;
  member_handle: string;
  adapter_id: string;
  model: string | null;
  effort: string | null;
  worktree_branch: string | null;
  head_commit_sha: string | null;
  status: TurnStatus;
  summary: string;
  touched_files: string[];
  diff_stat: DiffStat;
  started_at: number;
  finished_at: number | null;
}

export interface TrajectoryEvent {
  id: number;
  turn_id: string;
  event_kind: TrajectoryEventKind;
  payload: Record<string, unknown>;
  created_at: number;
}

export interface QuotaWindow {
  id: string;
  label: string;
  remaining_pct: number;
  resets_at: number | null;
  window_mins: number | null;
}

export interface QuotaSnapshot {
  adapter_id: string;
  plan_label: string | null;
  window_5h_remaining_pct: number | null;
  window_5h_resets_at: number | null;
  weekly_remaining_pct: number | null;
  weekly_resets_at: number | null;
  local_5h_tokens_est: number;
  rate_limited_until: number | null;
  exact_probe_opt_in: boolean;
  source:
    | "local_log_estimate"
    | "live_probe"
    | "user_calibrated"
    | "cooldown429"
    | "unavailable";
  windows?: QuotaWindow[];
  credits_label?: string | null;
  shared_profile_usage?: boolean;
  status_note?: string | null;
  updated_at: number;
}

export interface AdapterConfig {
  display_name: string;
  accent_color: string;
  enabled: boolean;
  command: string;
  args: string[];
  quota_provider: string;
  exact_quota_probe_opt_in: boolean;
  profile_dir?: string | null;
  characteristics: {
    strengths: string[];
    latency_tier: string;
    context_window_tier: string;
  };
}

export interface HarnessCliVersion {
  installed: string | null;
  published: string | null;
  can_upgrade: boolean;
  can_sign_in: boolean;
}

export interface HarnessOperation {
  kind: "sign_in" | "upgrade";
  status: "running" | "completed" | "failed";
  output: string;
  accepts_input: boolean;
}

export interface ConfigOptionChoice {
  value: string;
  label: string;
}

export interface DiscoveredConfigOption {
  id: string;
  category: string | null;
  label: string | null;
  current_value: string | null;
  choices: ConfigOptionChoice[];
}

export interface ModelChoiceDetail {
  value: string;
  label: string;
  description: string;
  is_default: boolean;
  default_effort: string | null;
  supported_efforts: ConfigOptionChoice[];
}

export interface AdapterModelCatalog {
  adapter_id: string;
  models: ConfigOptionChoice[];
  model_details?: ModelChoiceDetail[];
  efforts: ConfigOptionChoice[];
  default_context_window: number;
}

export interface StorageSummary {
  total_bytes: number;
  database_bytes: number;
  active_worktrees_bytes: number;
  active_worktrees_count: number;
  standalone_workspaces_bytes: number;
  standalone_count: number;
  archived_sessions_count: number;
  archived_bytes: number;
  cache_and_orphans_bytes: number;
  orphan_dirs_count: number;
  data_dir: string;
  workspaces_dir: string;
}

export interface StorageCleanupResult {
  harness_warnings?: string[];
  action: "clear_caches" | "delete_archived";
  removed_sessions: number;
  removed_dirs: number;
  freed_bytes: number;
}

export interface AdapterModelPref {
  adapter_id: string;
  default_model: string | null;
  default_effort: string | null;
}

export interface GlobalPersona {
  handle: string;
  adapter_id: string;
  display_name: string;
  persona_prompt: string;
  active_model: string | null;
  active_effort: string | null;
}

export interface ModelSelection {
  model: string | null;
  effort: string | null;
}

export interface MessageOptions {
  model?: string | null;
  effort?: string | null;
  recipient_overrides?: Record<string, ModelSelection>;
}

export interface SettingsFile {
  version: number;
  preferences: {
    global_personas: GlobalPersona[];
    adapter_defaults: AdapterModelPref[];
    quota_preferences: Array<{
      adapter_id: string;
      plan_label: string | null;
      exact_probe_opt_in: boolean;
      manual_5h_pct: number | null;
      manual_weekly_pct: number | null;
    }>;
  };
  theme?: ThemeId;
}

export interface MemberContextBreakdown {
  handle: string;
  adapter_id: string;
  model: string | null;
  used_tokens: number;
  context_window: number;
  is_exact: boolean;
  system_and_persona_tokens: number;
  memory_and_telemetry_tokens: number;
  chat_history_tokens: number;
  agent_and_tool_tokens: number;
}

export interface SessionContextReport {
  session_id: string;
  members: MemberContextBreakdown[];
}

export interface ShellHistoryEntry {
  id: string;
  command: string;
  cwd: string;
  exit_code: number;
  stdout: string;
  stderr: string;
}

export interface AdapterProbeStatus {
  adapter_id: string;
  display_name: string;
  accent_color: string;
  installed: boolean;
  resolved_command: string | null;
  auth_detected: boolean;
  auth_hint: string;
  strengths: string[];
}

export interface EpisodicJob {
  id: string;
  project_id: string;
  session_id: string | null;
  turn_id: string | null;
  member_handle: string;
  adapter_id: string;
  model: string | null;
  effort: string | null;
  task_brief: string;
  curated_summary: string;
  touched_modules: string[];
  outcome: string;
  curated_by: string;
  pinned: boolean;
  created_at: number;
}

export interface ProjectFact {
  id: string;
  project_id: string;
  category: string;
  summary: string;
  author_id: string;
  created_at: number;
}

export interface PermissionPromptOption {
  option_id: string;
  label: string;
  kind: string;
}

export interface PendingPermissionView {
  request_id: string;
  session_id: string;
  member_handle: string;
  turn_id: string | null;
  tool_kind: string;
  title: string;
  detail: string;
  suggested_pattern: string | null;
  options: PermissionPromptOption[];
  created_at: number;
}

export interface DispatchTaskSpec {
  target: string;
  model: string | null;
  effort: string | null;
  task_brief: string;
  mode: string;
}

export interface SessionDiffReport {
  integration_branch: string;
  base_branch: string;
  ahead_commits: number;
  diff_stat: DiffStat;
  unified_diff: string;
  truncated: boolean;
}

export interface FsEntry {
  name: string;
  path: string;
  is_dir: boolean;
  is_git_repo: boolean;
}

export type WsEvent =
  | { type: "chat.message_created"; message: ChatMessage }
  | {
      type: "chat.message_delta";
      session_id: string;
      message_id: string;
      turn_id: string;
      member_handle: string;
      delta: string;
      revision?: number;
      replace?: boolean;
    }
  | { type: "chat.message_completed"; message: ChatMessage }
  | {
      type: "switch.dispatch_decided";
      session_id: string;
      dispatch_id: string;
      message_id: string;
      status: string;
      rationale: string;
      dispatches: DispatchTaskSpec[];
    }
  | { type: "switch.consult_updated"; session_id: string; message: ChatMessage }
  | { type: "switch.activity_pill"; session_id: string; message: ChatMessage }
  | {
      type: "switch.hop_limit_paused";
      session_id: string;
      chain_id: string;
      hops_used: number;
      max_hops: number;
      pending_targets: string[];
    }
  | {
      type: "member.status_changed";
      session_id: string;
      handle: string;
      status: MemberStatus;
      active_model: string | null;
      active_effort: string | null;
      turn_id: string | null;
    }
  | {
      type: "trajectory.event";
      session_id: string;
      turn_id: string;
      event: TrajectoryEvent;
    }
  | {
      type: "trajectory.counters";
      session_id: string;
      turn_id: string;
      tool_calls: number;
      file_edits: number;
      thoughts: number;
    }
  | {
      type: "git.branch_updated";
      session_id: string;
      integration_branch: string;
      ahead_commits: number;
      diff_stat: DiffStat;
    }
  | {
      type: "git.conflict_escalated";
      session_id: string;
      member_handle: string;
      conflicting_files: string[];
    }
  | { type: "permission.requested"; prompt: PendingPermissionView }
  | {
      type: "permission.resolved";
      session_id: string;
      request_id: string;
      decision: string;
    }
  | { type: "quota.updated"; snapshot: QuotaSnapshot }
  | { type: "memory.job_curated"; job: EpisodicJob }
  | { type: "session.updated"; session: Session }
  | { type: "context.updated"; session_id: string; report: SessionContextReport };
