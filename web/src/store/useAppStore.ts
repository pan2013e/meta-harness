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
  ThemeId,
  TrajectoryEvent,
  TurnTrajectory,
  WsEvent,
} from "../types/api";

export type DrawerTab = "trajectory" | "memory" | "settings";

const VALID_THEMES: ThemeId[] = ["obsidian", "tokyo-night", "catppuccin", "daylight"];

function getInitialTheme(): ThemeId {
  try {
    const saved = window.localStorage.getItem("mh_theme") as ThemeId | null;
    if (saved && VALID_THEMES.includes(saved)) return saved;
  } catch {
    // ignore storage access issues
  }
  return "obsidian";
}

function applyThemeToDom(theme: ThemeId) {
  if (typeof document === "undefined") return;
  document.documentElement.setAttribute("data-theme", theme);
  document.documentElement.className = theme === "daylight" ? "light" : "dark";
  try {
    window.localStorage.setItem("mh_theme", theme);
  } catch {
    // ignore
  }
}

function cleanUserPath(raw: string): string {
  const trimmed = raw.trim();
  if (
    trimmed.length >= 2 &&
    ((trimmed.startsWith('"') && trimmed.endsWith('"')) ||
      (trimmed.startsWith("'") && trimmed.endsWith("'")))
  ) {
    return trimmed.slice(1, -1).trim();
  }
  return trimmed;
}

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

  theme: ThemeId;
  sidebarCollapsed: boolean;
  folderModalOpen: boolean;
  settingsOpen: boolean;
  pickingFolder: boolean;
  quotasRefreshing: boolean;

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

  setTheme: (theme: ThemeId) => void;
  toggleSidebar: () => void;
  setFolderModalOpen: (open: boolean) => void;
  setSettingsOpen: (open: boolean) => void;
  openProjectWithNativeOrModal: () => Promise<void>;

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
  refreshQuotasLive: () => Promise<void>;
  handleWsEvent: (ev: WsEvent) => void;
}

const emptySessionState = {
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
  inspectedTurnId: null,
  inspectedTrajectory: null,
  inspectedEvents: [],
};

export const useAppStore = create<AppState>((set, get) => {
  let initializing = false;
  let projectRequest = 0;
  let sessionRequest = 0;
  let diffRequest = 0;
  let inspectionRequest = 0;
  let memoryRequest = 0;
  let trajectoryRequest = 0;
  let sessionLoad: { id: string; events: WsEvent[] } | null = null;

  const clearSession = () => {
    sessionRequest++;
    inspectionRequest++;
    sessionLoad = null;
    get().wsClient?.subscribeTrajectory(null);
    get().wsClient?.setSession(null);
    set(emptySessionState);
  };

  const reportError = (error: unknown) =>
    set({ errorBanner: error instanceof Error ? error.message : String(error) });

  return {
  authRequired: false,
  authenticated: true,
  bootstrapping: true,
  errorBanner: null,

  theme: getInitialTheme(),
  sidebarCollapsed: typeof window !== "undefined" && window.matchMedia?.("(max-width: 1023px)").matches === true,
  folderModalOpen: false,
  settingsOpen: false,
  pickingFolder: false,
  quotasRefreshing: false,

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

  setTheme: (theme) => {
    applyThemeToDom(theme);
    set({ theme });
  },
  toggleSidebar: () => set((s) => ({ sidebarCollapsed: !s.sidebarCollapsed })),
  setFolderModalOpen: (open) => set({ folderModalOpen: open }),
  setSettingsOpen: (open) => set({ settingsOpen: open }),

  openProjectWithNativeOrModal: async () => {
    if (get().pickingFolder) return;
    set({ pickingFolder: true });
    try {
      const res = await api.pickFolderNative();
      if (res.supported && !res.cancelled && res.path) {
        await get().createAndSelectProject(res.path);
        return;
      }
      if (!res.supported) {
        set({ folderModalOpen: true });
      }
    } catch (e) {
      set({
        folderModalOpen: true,
        errorBanner: e instanceof Error ? e.message : String(e),
      });
    } finally {
      set({ pickingFolder: false });
    }
  },

  clearError: () => set({ errorBanner: null }),
  setError: (msg) => set({ errorBanner: msg }),

  init: async () => {
    if (initializing) return;
    initializing = true;
    applyThemeToDom(get().theme);
    try {
      const urlParams = new URLSearchParams(window.location.search);
      const tokenParam = urlParams.get("token");
      if (tokenParam) {
        urlParams.delete("token");
        const cleanUrl =
          window.location.pathname +
          (urlParams.toString() ? `?${urlParams.toString()}` : "");
        window.history.replaceState({}, "", cleanUrl + window.location.hash);
        await api.createAuthSession(tokenParam);
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

      const [adaptersRes, probeRes, projRes] = await Promise.all([
        api.listAdapters(),
        api.probeAdapters(),
        api.listProjects(),
      ]);

      get().wsClient?.disconnect();
      const ws = new WsHubClient((ev) => get().handleWsEvent(ev), () => {
        const sessionId = get().activeSession?.id;
        if (sessionId && !sessionLoad) void get().selectSession(sessionId);
      });
      ws.connect(null);

      set({
        authRequired: auth.required,
        authenticated: true,
        bootstrapping: false,
        adapters: adaptersRes.adapters,
        probes: probeRes.probes,
        projects: projRes.projects,
        wsClient: ws,
      });

      void get().refreshQuotas().catch((error: unknown) => reportError(error));

      if (projRes.projects.length > 0) {
        await get().selectProject(projRes.projects[0].id);
      }
    } catch (e) {
      const auth = await api.authStatus().catch(() => null);
      set({
        bootstrapping: false,
        ...(auth && { authRequired: auth.required, authenticated: auth.authenticated }),
        errorBanner: e instanceof Error ? e.message : String(e),
      });
    } finally {
      initializing = false;
    }
  },

  authenticateWithToken: async (token: string) => {
    if (get().bootstrapping || !token) return;
    set({ bootstrapping: true, errorBanner: null });
    try {
      await api.createAuthSession(token);
      setBearerToken(null);
      await get().init();
    } catch (error) {
      set({ bootstrapping: false });
      reportError(error);
    }
  },

  selectProject: async (projectId: string) => {
    const request = ++projectRequest;
    const memoryAtStart = ++memoryRequest;
    clearSession();
    set({ activeProjectId: projectId, sessions: [], memoryJobs: [], memoryFacts: [] });
    try {
      const [{ sessions }, mem] = await Promise.all([
        api.listSessions(projectId),
        api.getMemory(projectId),
      ]);
      if (request !== projectRequest) return;
      set({
        sessions,
        ...(memoryAtStart === memoryRequest && { memoryJobs: mem.jobs, memoryFacts: mem.facts }),
      });
      if (sessions.length > 0) await get().selectSession(sessions[0].id);
    } catch (error) {
      if (request === projectRequest) reportError(error);
    }
  },

  createAndSelectProject: async (rootPath: string, name?: string) => {
    const cleaned = cleanUserPath(rootPath);
    if (!cleaned) {
      throw new Error("Please select or enter a valid folder path.");
    }
    const { project } = await api.createProject(cleaned, name);
    const { projects } = await api.listProjects();
    set({ projects, folderModalOpen: false, errorBanner: null });
    await get().selectProject(project.id);
    if (get().activeProjectId === project.id && project.is_git_repo && get().sessions.length === 0) {
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
    const current = get().activeSession?.id === sessionId;
    if (!current) clearSession();
    const request = ++sessionRequest;
    const load = sessionLoad?.id === sessionId ? sessionLoad : { id: sessionId, events: [] as WsEvent[] };
    sessionLoad = load;
    get().wsClient?.setSession(sessionId);
    try {
      const [detail, msgsRes, trajRes] = await Promise.all([
        api.getSession(sessionId),
        api.listMessages(sessionId),
        api.listTrajectories(sessionId),
      ]);
      if (request !== sessionRequest) return;
      const existing = new Map(get().messages.map((message) => [message.id, message]));
      const statusMap = Object.fromEntries(
        Object.entries(detail.statuses).map(([handle, [status, turnId]]) => [handle, { status, turnId }]),
      );
      set({
        activeSession: detail.session,
        members: detail.members,
        permissionRules: detail.permission_rules,
        pendingPermissions: detail.pending_permissions,
        memberStatuses: statusMap,
        messages: msgsRes.messages.map((message) =>
          message.metadata.streaming === true ? existing.get(message.id) ?? message : message,
        ),
        trajectories: trajRes.trajectories,
      });
      sessionLoad = null;
      for (const event of load.events) get().handleWsEvent(event);
      await get().refreshSessionDiff();
    } catch (error) {
      if (request === sessionRequest) {
        sessionLoad = null;
        for (const event of load.events) get().handleWsEvent(event);
        reportError(error);
      }
    }
  },

  createAndSelectSession: async (title: string, orchestrator?: string | null) => {
    const pid = get().activeProjectId;
    if (!pid) return;
    const request = projectRequest;
    const { session } = await api.createSession(pid, {
      title,
      orchestrator_member: orchestrator,
    });
    const { sessions } = await api.listSessions(pid);
    if (request !== projectRequest) return;
    set({ sessions });
    await get().selectSession(session.id);
  },

  deleteSessionById: async (sessionId: string) => {
    const pid = get().activeProjectId;
    const request = projectRequest;
    await api.deleteSession(sessionId);
    if (pid) {
      const { sessions } = await api.listSessions(pid);
      if (request !== projectRequest) return;
      set({ sessions });
      if (get().activeSession?.id === sessionId || sessionLoad?.id === sessionId) {
        if (sessions.length > 0) {
          await get().selectSession(sessions[0].id);
        } else {
          clearSession();
        }
      }
    }
  },

  refreshSessionDiff: async () => {
    const sid = get().activeSession?.id;
    if (!sid) return;
    const request = ++diffRequest;
    const selected = sessionRequest;
    try {
      const { diff } = await api.getSessionDiff(sid);
      if (request === diffRequest && selected === sessionRequest) set({ sessionDiff: diff });
    } catch {
      if (request === diffRequest && selected === sessionRequest) set({ sessionDiff: null });
    }
  },

  sendMessage: async (content: string) => {
    const sid = get().activeSession?.id;
    if (!sid || !content.trim()) return;
    set({ hopPaused: null });
    const { message } = await api.postMessage(sid, content);
    get().handleWsEvent({ type: "chat.message_created", message });
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
    const request = ++inspectionRequest;
    set({ drawerOpen: true, drawerTab: "trajectory", inspectedTurnId: turnId,
      inspectedTrajectory: null, inspectedEvents: [] });
    get().wsClient?.subscribeTrajectory(turnId);
    try {
      const res = await api.getTrajectory(turnId);
      if (request !== inspectionRequest) return;
      set((state) => {
        const events = new Map(res.events.map((event) => [event.id, event]));
        for (const event of state.inspectedEvents) events.set(event.id, event);
        return { inspectedTrajectory: res.trajectory,
          inspectedEvents: [...events.values()].sort((a, b) => a.id - b.id) };
      });
    } catch (error) {
      if (request === inspectionRequest) reportError(error);
    }
  },

  openDrawerTab: (tab: DrawerTab) => {
    const wasOpen = get().drawerOpen;
    set({ drawerOpen: true, drawerTab: tab });
    if (tab === "trajectory" && (!wasOpen || !get().inspectedTurnId)) {
      const activeTurn = Object.values(get().memberStatuses).find(({ turnId }) => turnId)?.turnId;
      const turn = get().inspectedTurnId ?? activeTurn ?? get().trajectories[0]?.turn_id;
      if (turn) void get().inspectTurn(turn);
    }
  },

  closeDrawer: () => {
    inspectionRequest++;
    get().wsClient?.subscribeTrajectory(null);
    set({ drawerOpen: false });
  },

  refreshMemory: async (query?: string) => {
    const pid = get().activeProjectId;
    if (!pid) return;
    const request = ++memoryRequest;
    const mem = await api.getMemory(pid, query);
    if (request === memoryRequest) set({ memoryJobs: mem.jobs, memoryFacts: mem.facts });
  },

  refreshQuotas: async () => {
    const { quotas } = await api.listQuotas();
    for (const snapshot of quotas) get().handleWsEvent({ type: "quota.updated", snapshot });
  },

  refreshQuotasLive: async () => {
    set({ quotasRefreshing: true });
    try {
      const [{ quotas }, { probes }] = await Promise.all([
        api.refreshQuotasLive(),
        api.probeAdapters(),
      ]);
      const map: Record<string, QuotaSnapshot> = {};
      for (const q of quotas) map[q.adapter_id] = q;
      set({ quotas: map, probes });
    } catch (e) {
      set({ errorBanner: e instanceof Error ? e.message : String(e) });
    } finally {
      set({ quotasRefreshing: false });
    }
  },

  handleWsEvent: (ev: WsEvent) => {
    const eventSessionId = "message" in ev ? ev.message.session_id
      : "session_id" in ev ? ev.session_id
      : "prompt" in ev ? ev.prompt.session_id : null;
    if (sessionLoad && eventSessionId === sessionLoad.id) {
      sessionLoad.events.push(ev);
      return;
    }
    const activeSid = get().activeSession?.id;
    switch (ev.type) {
      case "chat.message_created": {
        if (ev.message.session_id !== activeSid) return;
        set((s) => {
          if (s.messages.some((m) => m.id === ev.message.id)) return s;
          return { messages: [...s.messages, ev.message].sort((a, b) => a.seq - b.seq) };
        });
        break;
      }
      case "chat.message_delta": {
        if (ev.session_id !== activeSid) return;
        set((s) => ({
          messages: s.messages.map((m) =>
            m.id === ev.message_id && m.metadata.streaming === true
              ? { ...m, content: m.content + ev.delta } : m
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
          const selected = sessionRequest;
          const request = ++trajectoryRequest;
          void api.listTrajectories(ev.message.session_id).then((res) => {
            if (selected === sessionRequest && request === trajectoryRequest) {
              set({ trajectories: res.trajectories });
            }
          }).catch(() => undefined);
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
        if (ev.session_id === activeSid && ev.turn_id === get().inspectedTurnId) {
          set((s) => s.inspectedEvents.some((event) => event.id === ev.event.id) ? s : ({
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
        set((s) => s.pendingPermissions.some((p) => p.request_id === ev.prompt.request_id) ? s : ({
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
        set((s) => (s.quotas[ev.snapshot.adapter_id]?.updated_at ?? 0) > ev.snapshot.updated_at
          ? s : { quotas: { ...s.quotas, [ev.snapshot.adapter_id]: ev.snapshot } });
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
  };
});
