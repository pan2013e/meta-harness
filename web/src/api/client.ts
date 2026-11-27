import type {
  AppUpdateCheck,
  AdapterConfig,
  AdapterModelCatalog,
  AdapterModelPref,
  AdapterProbeStatus,
  ChatMessage,
  DiscoveredConfigOption,
  EpisodicJob,
  FsEntry,
  GlobalPersona,
  HarnessCliVersion,
  HarnessOperation,
  IsolationMode,
  MemberRole,
  MemberStatus,
  MessageOptions,
  PendingPermissionView,
  PermissionPolicy,
  PermissionRule,
  Project,
  ProjectFact,
  PromoteMode,
  QuotaSnapshot,
  Session,
  SessionContextReport,
  SessionDiffReport,
  SessionMember,
  SettingsFile,
  SkillDetail,
  SkillSummary,
  StorageCleanupResult,
  StorageSummary,
  TrajectoryEvent,
  TurnTrajectory,
  WsEvent,
} from "../types/api";

async function apiFetch<T>(path: string, init?: RequestInit): Promise<T> {
  const headers = new Headers(init?.headers);
  if (!headers.has("Content-Type") && init?.body) {
    headers.set("Content-Type", "application/json");
  }
  const resp = await fetch(path, {
    ...init,
    headers,
    credentials: "same-origin",
  });
  if (!resp.ok) {
    let msg = `${resp.status} ${resp.statusText}`;
    try {
      const errBody = (await resp.json()) as { error?: string };
      if (errBody.error) msg = errBody.error;
    } catch {
      // ignore non-json errors
    }
    throw new Error(msg);
  }
  return (await resp.json()) as T;
}

function encodeFilePath(relPath: string): string {
  return relPath
    .replace(/\\/g, "/")
    .replace(/^\/+/, "")
    .split("/")
    .filter(Boolean)
    .map(encodeURIComponent)
    .join("/");
}

export function sessionFileUrl(sessionId: string, relPath: string): string {
  return `/api/sessions/${encodeURIComponent(sessionId)}/files/${encodeFilePath(relPath)}`;
}

export function messageImageUrl(sessionId: string, messageId: string, index: number): string {
  return `/api/sessions/${encodeURIComponent(sessionId)}/messages/${encodeURIComponent(messageId)}/images/${index}`;
}

export function sessionPreviewUrl(sessionId: string, grant: string, relPath: string): string {
  return `/preview/${encodeURIComponent(sessionId)}/${encodeURIComponent(grant)}/${encodeFilePath(relPath)}`;
}

export function ptyWsUrl(
  sessionId: string,
  cols: number,
  rows: number,
  target?: string | null
): string {
  const proto = window.location.protocol === "https:" ? "wss:" : "ws:";
  const params = new URLSearchParams({
    cols: String(cols),
    rows: String(rows),
  });
  if (target && target !== "integration") {
    params.set("target", target);
  }
  return `${proto}//${window.location.host}/api/sessions/${encodeURIComponent(
    sessionId
  )}/pty?${params.toString()}`;
}

export const api = {
  buildInfo: (signal?: AbortSignal) =>
    apiFetch<{ version: string; commit: string | null; release_build: boolean }>("/api/health", { signal }),
  checkForUpdate: (kind: "cli" | "desktop") =>
    apiFetch<AppUpdateCheck>(`/api/update?kind=${kind}`),
  estimateDraftTokens: (text: string, signal: AbortSignal) =>
    apiFetch<{ tokens: number }>("/api/context/estimate", { method: "POST", body: JSON.stringify({ text }), signal }),
  authStatus: () =>
    apiFetch<{ required: boolean; authenticated: boolean }>("/api/auth/status"),
  createAuthSession: (token: string) =>
    apiFetch<{ authenticated: boolean }>("/api/auth/session", {
      method: "POST",
      body: JSON.stringify({ token }),
    }),
  listFs: (path?: string, signal?: AbortSignal) =>
    apiFetch<{ current: string; parent: string | null; entries: FsEntry[] }>(
      `/api/fs/list${path ? `?path=${encodeURIComponent(path)}` : ""}`,
      { signal },
    ),
  listAdapters: (refresh = false, summary = false) =>
    apiFetch<{
      adapters: Record<string, AdapterConfig>;
      catalogs?: Record<string, AdapterModelCatalog>;
      defaults?: Record<string, AdapterModelPref>;
      global_personas?: GlobalPersona[];
      saved_profiles?: string[];
    }>(summary ? "/api/adapters?summary=true" : refresh ? "/api/adapters?refresh=true" : "/api/adapters"),
  updateAdapterDefaults: (
    adapterId: string,
    payload: {
      default_model?: string | null;
      default_effort?: string | null;
    }
  ) =>
    apiFetch<{ pref: AdapterModelPref }>(
      `/api/adapters/${encodeURIComponent(adapterId)}/defaults`,
      { method: "PATCH", body: JSON.stringify(payload) }
    ),
  probeAdapters: () =>
    apiFetch<{ probes: AdapterProbeStatus[] }>("/api/adapters/probe"),
  exportSettings: () => apiFetch<SettingsFile>("/api/settings"),
  importSettings: (document: unknown) =>
    apiFetch<SettingsFile>("/api/settings", { method: "POST", body: JSON.stringify(document) }),
  listSkills: (projectId?: string | null) =>
    apiFetch<{ skills: SkillSummary[]; skills_dir: string }>(
      `/api/skills${projectId ? `?project_id=${encodeURIComponent(projectId)}` : ""}`
    ),
  getSkillDetail: (
    id: string,
    opts?: { projectId?: string | null; file?: string }
  ) => {
    const params = new URLSearchParams();
    if (opts?.projectId) params.set("project_id", opts.projectId);
    if (opts?.file) params.set("file", opts.file);
    const qs = params.toString();
    return apiFetch<{ detail: SkillDetail }>(
      `/api/skills/${encodeURIComponent(id)}${qs ? `?${qs}` : ""}`
    );
  },
  createCustomSkill: (payload: {
    id?: string;
    name: string;
    description: string;
    content: string;
  }) =>
    apiFetch<{ skill: SkillSummary }>("/api/skills", {
      method: "POST",
      body: JSON.stringify(payload),
    }),
  updateSkillContent: (
    id: string,
    payload: { file?: string; content: string }
  ) =>
    apiFetch<{ detail: SkillDetail }>(
      `/api/skills/${encodeURIComponent(id)}/content`,
      { method: "PUT", body: JSON.stringify(payload) }
    ),
  patchSkill: (
    id: string,
    payload: { enabled: boolean; project_id?: string | null }
  ) =>
    apiFetch<{ skill: SkillSummary }>(
      `/api/skills/${encodeURIComponent(id)}`,
      { method: "PATCH", body: JSON.stringify(payload) }
    ),
  resetSkill: (id: string) =>
    apiFetch<{ skill: SkillSummary }>(
      `/api/skills/${encodeURIComponent(id)}/reset`,
      { method: "POST" }
    ),
  deleteSkill: (id: string) =>
    apiFetch<{ deleted: boolean }>(`/api/skills/${encodeURIComponent(id)}`, {
      method: "DELETE",
    }),
  installSkillFromFolder: (path: string) =>
    apiFetch<{ installed: SkillSummary[] }>("/api/skills/install-folder", {
      method: "POST",
      body: JSON.stringify({ path }),
    }),
  installSkillFromGit: (url: string, subpath?: string) =>
    apiFetch<{ installed: SkillSummary[] }>("/api/skills/install-git", {
      method: "POST",
      body: JSON.stringify({ url, ...(subpath ? { subpath } : {}) }),
    }),
  saveGlobalPersona: (persona: GlobalPersona) =>
    apiFetch<{ persona: GlobalPersona }>("/api/personas", { method: "POST", body: JSON.stringify(persona) }),
  deleteGlobalPersona: (handle: string) =>
    apiFetch<{ deleted: boolean }>(`/api/personas/${encodeURIComponent(handle)}`, { method: "DELETE" }),
  addGlobalPersonaToSession: (session: string, handle: string) =>
    apiFetch<{ member: SessionMember }>(
      `/api/sessions/${encodeURIComponent(session)}/personas/${encodeURIComponent(handle)}`, { method: "POST" },
    ),
  selectHarnessProfile: (id: string, profile: "default" | "separate") =>
    apiFetch<{ profile_dir: string | null }>(`/api/adapters/${encodeURIComponent(id)}/profile`, {
      method: "PATCH", body: JSON.stringify({ profile }),
    }),
  checkHarnessVersion: (id: string) =>
    apiFetch<{ version: HarnessCliVersion }>(`/api/adapters/${encodeURIComponent(id)}/version`),
  harnessOperation: (id: string) =>
    apiFetch<{ operation: HarnessOperation | null }>(`/api/adapters/${encodeURIComponent(id)}/operation`),
  startHarnessOperation: (id: string, kind: HarnessOperation["kind"], loginMode?: "device_code") =>
    apiFetch<{ operation: HarnessOperation }>(`/api/adapters/${encodeURIComponent(id)}/operation`, {
      method: "POST", body: JSON.stringify({ kind, ...(loginMode && { login_mode: loginMode }) }),
    }),
  cancelHarnessOperation: (id: string) =>
    apiFetch<{ cancelled: boolean }>(`/api/adapters/${encodeURIComponent(id)}/operation`, { method: "DELETE" }),
  sendHarnessSignInInput: (id: string, value: string) =>
    apiFetch<{ accepted: boolean }>(`/api/adapters/${encodeURIComponent(id)}/operation/input`, {
      method: "POST", body: JSON.stringify({ value }),
    }),
  listQuotas: () => apiFetch<{ quotas: QuotaSnapshot[] }>("/api/quotas"),
  refreshQuotasLive: () =>
    apiFetch<{ quotas: QuotaSnapshot[] }>("/api/quotas/refresh", {
      method: "POST",
    }),
  updateQuota: (
    adapterId: string,
    payload: {
      plan_label?: string | null;
      exact_probe_opt_in?: boolean;
      manual_5h_pct?: number | null;
      manual_weekly_pct?: number | null;
      clear_rate_limit?: boolean;
    }
  ) =>
    apiFetch<{ quota: QuotaSnapshot }>(
      `/api/adapters/${encodeURIComponent(adapterId)}/quota`,
      { method: "PATCH", body: JSON.stringify(payload) }
    ),

  listProjects: () => apiFetch<{ projects: Project[] }>("/api/projects"),
  createProject: (root_path: string, name?: string, signal?: AbortSignal) =>
    apiFetch<{ project: Project }>("/api/projects", {
      method: "POST",
      body: JSON.stringify({ root_path, name }),
      signal,
    }),
  gitInitProject: (projectId: string) =>
    apiFetch<{ project: Project }>(
      `/api/projects/${encodeURIComponent(projectId)}/git-init`,
      { method: "POST" }
    ),
  deleteProject: (projectId: string) =>
    apiFetch<{ deleted: boolean }>(
      `/api/projects/${encodeURIComponent(projectId)}`,
      { method: "DELETE" }
    ),

  listSessions: (projectId: string, includeArchived = false) =>
    apiFetch<{ sessions: Session[] }>(
      `/api/projects/${encodeURIComponent(projectId)}/sessions${
        includeArchived ? "?include_archived=true" : ""
      }`
    ),
  listStandaloneSessions: (includeArchived = false) =>
    apiFetch<{ sessions: Session[] }>(
      `/api/sessions${includeArchived ? "?include_archived=true" : ""}`
    ),
  createSession: (
    projectId: string,
    payload: {
      title?: string;
      orchestrator_member?: string | null;
      confirm_before_dispatch?: boolean;
      isolation_mode?: IsolationMode;
      permission_policy?: PermissionPolicy;
    }
  ) =>
    apiFetch<{ session: Session; members: SessionMember[] }>(
      `/api/projects/${encodeURIComponent(projectId)}/sessions`,
      { method: "POST", body: JSON.stringify(payload) }
    ),
  createStandaloneSession: (payload: {
    title?: string;
    orchestrator_member?: string | null;
    confirm_before_dispatch?: boolean;
    isolation_mode?: IsolationMode;
    permission_policy?: PermissionPolicy;
  }) =>
    apiFetch<{ session: Session; members: SessionMember[] }>("/api/sessions", {
      method: "POST",
      body: JSON.stringify(payload),
    }),
  getSession: (sessionId: string) =>
    apiFetch<{
      session: Session;
      members: SessionMember[];
      permission_rules: PermissionRule[];
      statuses: Record<string, [MemberStatus, string | null]>;
      pending_permissions: PendingPermissionView[];
      discovered_options?: Record<string, DiscoveredConfigOption[]>;
      context?: SessionContextReport | null;
    }>(`/api/sessions/${encodeURIComponent(sessionId)}`),
  patchSession: (
    sessionId: string,
    payload: {
      title?: string;
      orchestrator_member?: string | null;
      confirm_before_dispatch?: boolean;
      max_hops?: number;
      permission_policy?: PermissionPolicy;
      archived?: boolean;
    }
  ) =>
    apiFetch<{ session: Session; harness_warnings?: string[] }>(
      `/api/sessions/${encodeURIComponent(sessionId)}`,
      { method: "PATCH", body: JSON.stringify(payload) }
    ),
  compactSession: (sessionId: string) =>
    apiFetch<{ message: ChatMessage }>(
      `/api/sessions/${encodeURIComponent(sessionId)}/compact`,
      { method: "POST" }
    ),
  deleteSession: (sessionId: string) =>
    apiFetch<{ deleted: boolean; harness_warnings?: string[] }>(
      `/api/sessions/${encodeURIComponent(sessionId)}`,
      { method: "DELETE" }
    ),
  getStorage: () => apiFetch<{ storage: StorageSummary }>("/api/storage"),
  cleanupStorage: (action: "clear_caches" | "delete_archived") =>
    apiFetch<{ result: StorageCleanupResult; storage: StorageSummary }>(
      "/api/storage/cleanup",
      { method: "POST", body: JSON.stringify({ action }) }
    ),

  upsertMember: (
    sessionId: string,
    payload: {
      handle: string;
      adapter_id: string;
      display_name?: string;
      role?: MemberRole;
      persona_prompt?: string;
      active_model?: string | null;
      active_effort?: string | null;
    }
  ) =>
    apiFetch<{ member: SessionMember }>(
      `/api/sessions/${encodeURIComponent(sessionId)}/members`,
      { method: "POST", body: JSON.stringify(payload) }
    ),
  deleteMember: (sessionId: string, handle: string) =>
    apiFetch<{ deleted: boolean }>(
      `/api/sessions/${encodeURIComponent(sessionId)}/members/${encodeURIComponent(handle)}`,
      { method: "DELETE" }
    ),
  patchMemberConfig: (
    sessionId: string,
    handle: string,
    payload: { active_model?: string | null; active_effort?: string | null }
  ) =>
    apiFetch<{ member: SessionMember }>(
      `/api/sessions/${encodeURIComponent(sessionId)}/members/${encodeURIComponent(handle)}/config`,
      { method: "PATCH", body: JSON.stringify(payload) }
    ),
  getSessionContext: (sessionId: string) =>
    apiFetch<{ context: SessionContextReport }>(
      `/api/sessions/${encodeURIComponent(sessionId)}/context`
    ),

  listMessages: (sessionId: string, beforeSeq?: number) =>
    apiFetch<{ messages: ChatMessage[] }>(
      `/api/sessions/${encodeURIComponent(sessionId)}/messages${
        beforeSeq ? `?before_seq=${beforeSeq}` : ""
      }`
    ),
  postMessage: (
    sessionId: string,
    content: string,
    opts?: MessageOptions
  ) =>
    apiFetch<{ message: ChatMessage }>(
      `/api/sessions/${encodeURIComponent(sessionId)}/messages`,
      {
        method: "POST",
        body: JSON.stringify({
          content,
          ...(opts?.model ? { model: opts.model } : {}),
          ...(opts?.effort ? { effort: opts.effort } : {}),
          ...(opts?.recipient_overrides ? { recipient_overrides: opts.recipient_overrides } : {}),
          ...(opts?.images?.length ? { images: opts.images } : {}),
        }),
      }
    ),
  cancelSession: (sessionId: string) =>
    apiFetch<{ cancelled: boolean }>(
      `/api/sessions/${encodeURIComponent(sessionId)}/cancel`,
      { method: "POST" }
    ),
  continueChain: (sessionId: string, chainId: string, extraHops = 6) =>
    apiFetch<{ continued: boolean }>(
      `/api/sessions/${encodeURIComponent(sessionId)}/chains/${encodeURIComponent(chainId)}/continue`,
      { method: "POST", body: JSON.stringify({ extra_hops: extraHops }) }
    ),
  confirmDispatch: (sessionId: string, dispatchId: string) =>
    apiFetch<{ dispatched: boolean }>(
      `/api/sessions/${encodeURIComponent(sessionId)}/dispatches/${encodeURIComponent(dispatchId)}/confirm`,
      { method: "POST" }
    ),
  cancelDispatch: (sessionId: string, dispatchId: string) =>
    apiFetch<{ cancelled: boolean }>(
      `/api/sessions/${encodeURIComponent(sessionId)}/dispatches/${encodeURIComponent(dispatchId)}/cancel`,
      { method: "POST" }
    ),
  resolvePermission: (
    sessionId: string,
    reqId: string,
    option_id: string,
    allow_pattern?: string
  ) =>
    apiFetch<{ resolved: boolean }>(
      `/api/sessions/${encodeURIComponent(sessionId)}/permissions/${encodeURIComponent(reqId)}`,
      {
        method: "POST",
        body: JSON.stringify({ option_id, allow_pattern: allow_pattern ?? null }),
      }
    ),
  deletePermissionRule: (sessionId: string, ruleId: string) =>
    apiFetch<{ deleted: boolean }>(
      `/api/sessions/${encodeURIComponent(sessionId)}/permission-rules/${encodeURIComponent(ruleId)}`,
      { method: "DELETE" }
    ),

  getSessionDiff: (sessionId: string) =>
    apiFetch<{ diff: SessionDiffReport | null }>(
      `/api/sessions/${encodeURIComponent(sessionId)}/diff`
    ),
  runSessionCommand: (sessionId: string, command: string, cwd?: string) =>
    apiFetch<{ cwd: string; exit_code: number; stdout: string; stderr: string }>(
      `/api/sessions/${encodeURIComponent(sessionId)}/run-command`,
      { method: "POST", body: JSON.stringify({ command, ...(cwd ? { cwd } : {}) }) }
    ),
  promoteSession: (sessionId: string, mode: PromoteMode) =>
    apiFetch<{
      result: { mode: PromoteMode; target_branch: string; summary: string };
    }>(`/api/sessions/${encodeURIComponent(sessionId)}/promote`, {
      method: "POST",
      body: JSON.stringify({ mode }),
    }),

  listTrajectories: (sessionId: string) =>
    apiFetch<{ trajectories: TurnTrajectory[] }>(
      `/api/trajectories?session_id=${encodeURIComponent(sessionId)}`
    ),
  getTrajectory: (turnId: string) =>
    apiFetch<{ trajectory: TurnTrajectory; events: TrajectoryEvent[] }>(
      `/api/trajectories/${encodeURIComponent(turnId)}`
    ),

  getMemory: (projectId: string, q?: string) =>
    apiFetch<{ jobs: EpisodicJob[]; facts: ProjectFact[] }>(
      `/api/projects/${encodeURIComponent(projectId)}/memory${
        q ? `?q=${encodeURIComponent(q)}` : ""
      }`
    ),
  createFact: (projectId: string, category: string, summary: string) =>
    apiFetch<{ fact: ProjectFact }>(
      `/api/projects/${encodeURIComponent(projectId)}/memory/facts`,
      { method: "POST", body: JSON.stringify({ category, summary }) }
    ),
  deleteFact: (projectId: string, factId: string) =>
    apiFetch<{ deleted: boolean }>(
      `/api/projects/${encodeURIComponent(projectId)}/memory/facts/${encodeURIComponent(factId)}`,
      { method: "DELETE" }
    ),
  pinJob: (jobId: string, pinned: boolean) =>
    apiFetch<{ updated: boolean }>(
      `/api/memory/jobs/${encodeURIComponent(jobId)}`,
      { method: "PATCH", body: JSON.stringify({ pinned }) }
    ),
  deleteJob: (jobId: string) =>
    apiFetch<{ deleted: boolean }>(
      `/api/memory/jobs/${encodeURIComponent(jobId)}`,
      { method: "DELETE" }
    ),
};

export class WsHubClient {
  private ws: WebSocket | null = null;
  private reconnectTimer: number | null = null;
  private sessionId: string | null = null;
  private subscribedTurn: string | null = null;
  private closed = false;

  constructor(
    private readonly onEvent: (ev: WsEvent) => void,
    private readonly onConnected?: () => void,
    private readonly onSessionExpired?: () => void,
  ) {}

  connect(sessionId: string | null) {
    this.sessionId = sessionId;
    this.closed = false;
    this.openSocket();
  }

  setSession(sessionId: string | null) {
    this.sessionId = sessionId;
    if (this.ws?.readyState === WebSocket.OPEN) {
      this.ws.send(JSON.stringify({ type: "set_session", session_id: sessionId }));
    }
  }

  subscribeTrajectory(turnId: string | null) {
    this.subscribedTurn = turnId;
    if (this.ws?.readyState === WebSocket.OPEN) {
      this.ws.send(
        JSON.stringify({ type: "subscribe_trajectory", turn_id: turnId })
      );
    }
  }

  disconnect() {
    this.closed = true;
    if (this.reconnectTimer !== null) {
      window.clearTimeout(this.reconnectTimer);
      this.reconnectTimer = null;
    }
    this.ws?.close();
    this.ws = null;
  }

  private openSocket() {
    if (this.closed) return;
    this.reconnectTimer = null;
    if (this.ws && this.ws.readyState <= WebSocket.OPEN) {
      this.ws.close();
    }
    const proto = window.location.protocol === "https:" ? "wss:" : "ws:";
    const params = new URLSearchParams();
    if (this.sessionId) params.set("session_id", this.sessionId);
    const url = `${proto}//${window.location.host}/api/ws?${params.toString()}`;
    const socket = new WebSocket(url);
    this.ws = socket;

    socket.onopen = () => {
      if (this.ws !== socket) return;
      this.setSession(this.sessionId);
      this.subscribeTrajectory(this.subscribedTurn);
      this.onConnected?.();
    };

    socket.onmessage = (msg) => {
      if (this.ws !== socket) return;
      try {
        const ev = JSON.parse(String(msg.data)) as WsEvent;
        this.onEvent(ev);
      } catch {
        // ignore malformed frames
      }
    };

    socket.onclose = (event) => {
      if (this.ws === socket && !this.closed) {
        this.ws = null;
        if (event.code === 1008 && event.reason === "Authentication expired") {
          this.closed = true;
          this.onSessionExpired?.();
          return;
        }
        this.reconnectTimer = window.setTimeout(() => this.openSocket(), 1500);
      }
    };
  }
}
