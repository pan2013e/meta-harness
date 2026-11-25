import { create } from "zustand";
import { api, setBearerToken, WsHubClient } from "../api/client";
import type {
  AdapterConfig,
  AdapterProbeStatus,
  ChatMessage,
  EpisodicJob,
  MemberStatus,
  PendingPermissionView,
  PermissionRule,
  Project,
  ProjectFact,
  QuotaSnapshot,
  Session,
  SessionDiffReport,
  SessionMember,
  TrajectoryEvent,
  TurnTrajectory,
  WsEvent,
} from "../types/api";

export type DrawerTab = "trajectory" | "memory" | "settings";

interface HopPausedBanner {
  chain_id: string;
  hops_used: number;
  max_hops: number;
  pending_targets: string[];
}

interface AppState {
  authRequired: boolean;
  authenticated: boolean;
  bootstrapping: boolean;
  errorBanner: string | null;

  adapters: Record<string, AdapterConfig>;
  probes: AdapterProbeStatus[];
  quotas: Record<string, QuotaSnapshot>;

  projects: Project[];
  activeProjectId: string | null;
  sessions: Session[];
  activeSession: Session | null;
  members: SessionMember[];
  memberStatuses: Record<string, { status: MemberStatus; turnId: string | null }>;
  permissionRules: PermissionRule[];
  pendingPermissions: PendingPermissionView[];
  sessionDiff: SessionDiffReport | null;
  hopPaused: HopPausedBanner | null;

  messages: ChatMessage[];
  trajectories: TurnTrajectory[];
  turnCounters: Record<
    string,
    { tool_calls: number; file_edits: number; thoughts: number }
  >;

  drawerOpen: boolean;
  drawerTab: DrawerTab;
  inspectedTurnId: string | null;
  inspectedTrajectory: TurnTrajectory | null;
  inspectedEvents: TrajectoryEvent[];

  memoryJobs: EpisodicJob[];
  memoryFacts: ProjectFact[];

  wsClient: WsHubClient | null;

  init: () => Promise<void>;
  authenticateWithToken: (token: string) => Promise<void>;
  clearError: () => void;
  setError: (msg: string) => void;

  selectProject: (projectId: string) => Promise<void>;
  createAndSelectProject: (rootPath: string, name?: string) => Promise<void>;
  gitInitActiveProject: () => Promise<void>;

  selectSession: (sessionId: string) => Promise<void>;
  createAndSelectSession: (title: string, orchestrator?: string | null) => Promise<void>;
  deleteSessionById: (sessionId: string) => Promise<void>;
  refreshSessionDiff: () => Promise<void>;

  sendMessage: (content: string) => Promise<void>;
  stopAll: () => Promise<void>;
  continueHopChain: (chainId: string) => Promise<void>;
  inspectTurn: (turnId: string) => Promise<void>;
  openDrawerTab: (tab: DrawerTab) => void;
  closeDrawer: () => void;

  refreshMemory: (query?: string) => Promise<void>;
  refreshQuotas: () => Promise<void>;
  handleWsEvent: (ev: WsEvent) => void;
}

export const useAppStore = create<AppState>((set, get) => ({
  authRequired: false,
  authenticated: true,
  bootstrapping: true,
  errorBanner: null,

  adapters: {},
  probes: [],
  quotas: {},

  projects: [],
  activeProjectId: null,
  sessions: [],
  activeSession: null,
  members: [],
  memberStatuses: {},
  permissionRules: [],
  pendingPermissions: [],
  sessionDiff: null,
  hopPaused: null,

  messages: [],
  trajectories: [],
  turnCounters: {},

  drawerOpen: false,
  drawerTab: "trajectory",
  inspectedTurnId: null,
  inspectedTrajectory: null,
  inspectedEvents: [],

  memoryJobs: [],
  memoryFacts: [],
  wsClient: null,

  clearError: () => set({ errorBanner: null }),
  setError: (msg) => set({ errorBanner: msg }),

  init: async () => {
    try {
      const urlParams = new URLSearchParams(window.location.search);
      const tokenParam = urlParams.get("token");
      if (tokenParam) {
        setBearerToken(tokenParam);
        await api.createAuthSession(tokenParam);
        urlParams.delete("token");
        const cleanUrl =
          window.location.pathname +
          (urlParams.toString() ? `?${urlParams.toString()}` : "");
        window.history.replaceState({}, "", cleanUrl);
      }

      const auth = await api.authStatus();
      if (auth.required && !auth.authenticated) {
        set({
          authRequired: true,
          authenticated: false,
          bootstrapping: false,
        });
        return;
      }

      const [adaptersRes, probeRes, quotaRes, projRes] = await Promise.all([
        api.listAdapters(),
        api.probeAdapters(),
        api.listQuotas(),
        api.listProjects(),
      ]);

      const quotaMap: Record<string, QuotaSnapshot> = {};
      for (const q of quotaRes.quotas) {
        quotaMap[q.adapter_id] = q;
      }

      const ws = new WsHubClient((ev) => get().handleWsEvent(ev));
      ws.connect(null);

      set({
        authRequired: auth.required,
        authenticated: true,
        bootstrapping: false,
        adapters: adaptersRes.adapters,
        probes: probeRes.probes,
        quotas: quotaMap,
        projects: projRes.projects,
        wsClient: ws,
      });

      if (projRes.projects.length > 0) {
        await get().selectProject(projRes.projects[0].id);
      }
    } catch (e) {
      set({
        bootstrapping: false,
        errorBanner: e instanceof Error ? e.message : String(e),
      });
    }
  },

  authenticateWithToken: async (token: string) => {
    setBearerToken(token);
    await api.createAuthSession(token);
    set({ authenticated: true, bootstrapping: true });
    await get().init();
  },

  selectProject: async (projectId: string) => {
    const [{ sessions }, mem] = await Promise.all([
      api.listSessions(projectId),
      api.getMemory(projectId),
    ]);
    set({
      activeProjectId: projectId,
      sessions,
      memoryJobs: mem.jobs,
      memoryFacts: mem.facts,
      activeSession: null,
      members: [],
      messages: [],
      trajectories: [],
      sessionDiff: null,
    });
    if (sessions.length > 0) {
      await get().selectSession(sessions[0].id);
    }
  },

  createAndSelectProject: async (rootPath: string, name?: string) => {
    const { project } = await api.createProject(rootPath, name);
    const { projects } = await api.listProjects();
    set({ projects });
    await get().selectProject(project.id);
    if (get().sessions.length === 0) {
      await get().createAndSelectSession("Main Group Chat");
    }
  },

  gitInitActiveProject: async () => {
    const pid = get().activeProjectId;
    if (!pid) return;
    const { project } = await api.gitInitProject(pid);
    set((s) => ({
      projects: s.projects.map((p) => (p.id === project.id ? project : p)),
    }));
  },

  selectSession: async (sessionId: string) => {
    const [detail, msgsRes, trajRes] = await Promise.all([
      api.getSession(sessionId),
      api.listMessages(sessionId),
      api.listTrajectories(sessionId),
    ]);

    const statusMap: Record<string, { status: MemberStatus; turnId: string | null }> =
      {};
    for (const [h, pair] of Object.entries(detail.statuses)) {
      statusMap[h] = { status: pair[0] as MemberStatus, turnId: pair[1] };
    }

    get().wsClient?.setSession(sessionId);

    set({
      activeSession: detail.session,
      members: detail.members,
      permissionRules: detail.permission_rules,
      pendingPermissions: detail.pending_permissions,
      memberStatuses: statusMap,
      messages: msgsRes.messages,
      trajectories: trajRes.trajectories,
      hopPaused: null,
    });

    await get().refreshSessionDiff();
  },

  createAndSelectSession: async (title: string, orchestrator?: string | null) => {
    const pid = get().activeProjectId;
    if (!pid) return;
    const { session } = await api.createSession(pid, {
      title,
      orchestrator_member: orchestrator,
    });
    const { sessions } = await api.listSessions(pid);
    set({ sessions });
    await get().selectSession(session.id);
  },

  deleteSessionById: async (sessionId: string) => {
    const pid = get().activeProjectId;
    await api.deleteSession(sessionId);
    if (pid) {
      const { sessions } = await api.listSessions(pid);
      set({ sessions });
      if (get().activeSession?.id === sessionId) {
        if (sessions.length > 0) {
          await get().selectSession(sessions[0].id);
        } else {
          set({ activeSession: null, members: [], messages: [], trajectories: [] });
        }
      }
    }
  },

  refreshSessionDiff: async () => {
    const sid = get().activeSession?.id;
    if (!sid) return;
    try {
      const { diff } = await api.getSessionDiff(sid);
      set({ sessionDiff: diff });
    } catch {
      set({ sessionDiff: null });
    }
  },

  sendMessage: async (content: string) => {
    const sid = get().activeSession?.id;
    if (!sid || !content.trim()) return;
    set({ hopPaused: null });
    await api.postMessage(sid, content);
  },

  stopAll: async () => {
    const sid = get().activeSession?.id;
    if (!sid) return;
    await api.cancelSession(sid);
  },

  continueHopChain: async (chainId: string) => {
    const sid = get().activeSession?.id;
    if (!sid) return;
    set({ hopPaused: null });
    await api.continueChain(sid, chainId, 6);
  },

  inspectTurn: async (turnId: string) => {
    const res = await api.getTrajectory(turnId);
    get().wsClient?.subscribeTrajectory(turnId);
    set({
      drawerOpen: true,
      drawerTab: "trajectory",
      inspectedTurnId: turnId,
      inspectedTrajectory: res.trajectory,
      inspectedEvents: res.events,
    });
  },

  openDrawerTab: (tab: DrawerTab) => {
    set({ drawerOpen: true, drawerTab: tab });
  },

  closeDrawer: () => {
    get().wsClient?.subscribeTrajectory(null);
    set({ drawerOpen: false });
  },

  refreshMemory: async (query?: string) => {
    const pid = get().activeProjectId;
    if (!pid) return;
    const mem = await api.getMemory(pid, query);
    set({ memoryJobs: mem.jobs, memoryFacts: mem.facts });
  },

  refreshQuotas: async () => {
    const { quotas } = await api.listQuotas();
    const map: Record<string, QuotaSnapshot> = {};
    for (const q of quotas) map[q.adapter_id] = q;
    set({ quotas: map });
  },

  handleWsEvent: (ev: WsEvent) => {
    const activeSid = get().activeSession?.id;
    switch (ev.type) {
      case "chat.message_created": {
        if (ev.message.session_id !== activeSid) return;
        set((s) => {
          if (s.messages.some((m) => m.id === ev.message.id)) return s;
          return { messages: [...s.messages, ev.message] };
        });
        break;
      }
      case "chat.message_delta": {
        if (ev.session_id !== activeSid) return;
        set((s) => ({
          messages: s.messages.map((m) =>
            m.id === ev.message_id ? { ...m, content: m.content + ev.delta } : m
          ),
        }));
        break;
      }
      case "chat.message_completed": {
        if (ev.message.session_id !== activeSid) return;
        set((s) => ({
          messages: s.messages.some((m) => m.id === ev.message.id)
            ? s.messages.map((m) => (m.id === ev.message.id ? ev.message : m))
            : [...s.messages, ev.message],
        }));
        if (ev.message.session_id) {
          void api.listTrajectories(ev.message.session_id).then((res) => {
            set({ trajectories: res.trajectories });
          });
        }
        break;
      }
      case "switch.consult_updated":
      case "switch.activity_pill": {
        if (ev.session_id !== activeSid) return;
        set((s) => {
          if (s.messages.some((m) => m.id === ev.message.id)) return s;
          return { messages: [...s.messages, ev.message] };
        });
        break;
      }
      case "switch.hop_limit_paused": {
        if (ev.session_id !== activeSid) return;
        set({
          hopPaused: {
            chain_id: ev.chain_id,
            hops_used: ev.hops_used,
            max_hops: ev.max_hops,
            pending_targets: ev.pending_targets,
          },
        });
        break;
      }
      case "member.status_changed": {
        if (ev.session_id !== activeSid) return;
        set((s) => ({
          memberStatuses: {
            ...s.memberStatuses,
            [ev.handle]: { status: ev.status, turnId: ev.turn_id },
          },
          members: s.members.map((m) =>
            m.handle === ev.handle
              ? {
                  ...m,
                  active_model: ev.active_model ?? m.active_model,
                  active_effort: ev.active_effort ?? m.active_effort,
                }
              : m
          ),
        }));
        break;
      }
      case "trajectory.event": {
        if (ev.turn_id === get().inspectedTurnId) {
          set((s) => ({
            inspectedEvents: [...s.inspectedEvents, ev.event],
          }));
        }
        break;
      }
      case "trajectory.counters": {
        if (ev.session_id !== activeSid) return;
        set((s) => ({
          turnCounters: {
            ...s.turnCounters,
            [ev.turn_id]: {
              tool_calls: ev.tool_calls,
              file_edits: ev.file_edits,
              thoughts: ev.thoughts,
            },
          },
        }));
        break;
      }
      case "git.branch_updated": {
        if (ev.session_id === activeSid) {
          void get().refreshSessionDiff();
        }
        break;
      }
      case "permission.requested": {
        if (ev.prompt.session_id !== activeSid) return;
        set((s) => ({
          pendingPermissions: [...s.pendingPermissions, ev.prompt],
        }));
        break;
      }
      case "permission.resolved": {
        if (ev.session_id !== activeSid) return;
        set((s) => ({
          pendingPermissions: s.pendingPermissions.filter(
            (p) => p.request_id !== ev.request_id
          ),
        }));
        break;
      }
      case "quota.updated": {
        set((s) => ({
          quotas: { ...s.quotas, [ev.snapshot.adapter_id]: ev.snapshot },
        }));
        break;
      }
      case "memory.job_curated": {
        if (ev.job.project_id === get().activeProjectId) {
          set((s) => ({
            memoryJobs: [ev.job, ...s.memoryJobs.filter((j) => j.id !== ev.job.id)],
          }));
        }
        break;
      }
    }
  },
}));
