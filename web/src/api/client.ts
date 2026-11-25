import type {
  AdapterConfig,
  AdapterProbeStatus,
  ChatMessage,
  EpisodicJob,
  FsEntry,
  IsolationMode,
  MemberRole,
  PendingPermissionView,
  PermissionPolicy,
  PermissionRule,
  Project,
  ProjectFact,
  PromoteMode,
  QuotaSnapshot,
  Session,
  SessionDiffReport,
  SessionMember,
  TrajectoryEvent,
  TurnTrajectory,
  WsEvent,
} from "../types/api";

let bearerToken: string | null = null;

export function setBearerToken(token: string | null) {
  bearerToken = token;
}

async function apiFetch<T>(path: string, init?: RequestInit): Promise<T> {
  const headers = new Headers(init?.headers);
  if (!headers.has("Content-Type") && init?.body) {
    headers.set("Content-Type", "application/json");
  }
  if (bearerToken) {
    headers.set("Authorization", `Bearer ${bearerToken}`);
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

export const api = {
  authStatus: () =>
    apiFetch<{ required: boolean; authenticated: boolean }>("/api/auth/status"),
  createAuthSession: (token: string) =>
    apiFetch<{ authenticated: boolean }>("/api/auth/session", {
      method: "POST",
      body: JSON.stringify({ token }),
    }),
  listFs: (path?: string) =>
    apiFetch<{ current: string; parent: string | null; entries: FsEntry[] }>(
      `/api/fs/list${path ? `?path=${encodeURIComponent(path)}` : ""}`
    ),
  pickFolderNative: () =>
    apiFetch<{ supported: boolean; cancelled: boolean; path: string | null }>(
      "/api/fs/pick-folder",
      { method: "POST" }
    ),
  listAdapters: () =>
    apiFetch<{ adapters: Record<string, AdapterConfig> }>("/api/adapters"),
  probeAdapters: () =>
    apiFetch<{ probes: AdapterProbeStatus[] }>("/api/adapters/probe"),
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
  createProject: (root_path: string, name?: string) =>
    apiFetch<{ project: Project }>("/api/projects", {
      method: "POST",
      body: JSON.stringify({ root_path, name }),
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

  listSessions: (projectId: string) =>
    apiFetch<{ sessions: Session[] }>(
      `/api/projects/${encodeURIComponent(projectId)}/sessions`
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
  getSession: (sessionId: string) =>
    apiFetch<{
      session: Session;
      members: SessionMember[];
      permission_rules: PermissionRule[];
      statuses: Record<string, [string, string | null]>;
      pending_permissions: PendingPermissionView[];
    }>(`/api/sessions/${encodeURIComponent(sessionId)}`),
  patchSession: (
    sessionId: string,
    payload: {
      title?: string;
      orchestrator_member?: string | null;
      confirm_before_dispatch?: boolean;
      max_hops?: number;
      permission_policy?: PermissionPolicy;
    }
  ) =>
    apiFetch<{ session: Session }>(
      `/api/sessions/${encodeURIComponent(sessionId)}`,
      { method: "PATCH", body: JSON.stringify(payload) }
    ),
  deleteSession: (sessionId: string) =>
    apiFetch<{ deleted: boolean }>(
      `/api/sessions/${encodeURIComponent(sessionId)}`,
      { method: "DELETE" }
    ),

  upsertMember: (
    sessionId: string,
    payload: {
      handle: string;
      adapter_id: string;
      display_name?: string;
      role?: MemberRole;
      persona_prompt?: string;
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

  listMessages: (sessionId: string, beforeSeq?: number) =>
    apiFetch<{ messages: ChatMessage[] }>(
      `/api/sessions/${encodeURIComponent(sessionId)}/messages${
        beforeSeq ? `?before_seq=${beforeSeq}` : ""
      }`
    ),
  postMessage: (sessionId: string, content: string) =>
    apiFetch<{ message: ChatMessage }>(
      `/api/sessions/${encodeURIComponent(sessionId)}/messages`,
      { method: "POST", body: JSON.stringify({ content }) }
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
  runSessionCommand: (sessionId: string, command: string) =>
    apiFetch<{ cwd: string; exit_code: number; stdout: string; stderr: string }>(
      `/api/sessions/${encodeURIComponent(sessionId)}/run-command`,
      { method: "POST", body: JSON.stringify({ command }) }
    ),
  openSessionTerminal: (sessionId: string) =>
    apiFetch<{ opened: boolean; cwd: string }>(
      `/api/sessions/${encodeURIComponent(sessionId)}/open-terminal`,
      { method: "POST" }
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
  private onEvent: (ev: WsEvent) => void;

  constructor(onEvent: (ev: WsEvent) => void) {
    this.onEvent = onEvent;
  }

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
    if (this.ws && this.ws.readyState <= WebSocket.OPEN) {
      this.ws.close();
    }
    const proto = window.location.protocol === "https:" ? "wss:" : "ws:";
    const params = new URLSearchParams();
    if (this.sessionId) params.set("session_id", this.sessionId);
    if (bearerToken) params.set("token", bearerToken);
    const url = `${proto}//${window.location.host}/api/ws?${params.toString()}`;
    const socket = new WebSocket(url);

    socket.onopen = () => {
      if (this.subscribedTurn) {
        socket.send(
          JSON.stringify({
            type: "subscribe_trajectory",
            turn_id: this.subscribedTurn,
          })
        );
      }
    };

    socket.onmessage = (msg) => {
      try {
        const ev = JSON.parse(String(msg.data)) as WsEvent;
        this.onEvent(ev);
      } catch {
        // ignore malformed frames
      }
    };

    socket.onclose = () => {
      if (!this.closed) {
        this.reconnectTimer = window.setTimeout(() => this.openSocket(), 1500);
      }
    };
  }
}
