import { create } from "zustand";
import { api, setBearerToken, WsHubClient } from "../api/client";
import type {
  AdapterConfig,
  AdapterModelCatalog,
  AdapterModelPref,
  AdapterProbeStatus,
  ChatMessage,
  DiscoveredConfigOption,
  EpisodicJob,
  MemberStatus,
  PendingPermissionView,
  PermissionPolicy,
  PermissionRule,
  Project,
  ProjectFact,
  QuotaSnapshot,
  Session,
  SessionContextReport,
  SessionDiffReport,
  SessionMember,
  ShellHistoryEntry,
  StorageCleanupResult,
  StorageSummary,
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
  memoryModalOpen: boolean;
  diffModalOpen: boolean;
  shellOpen: boolean;
  pickingFolder: boolean;
  quotasRefreshing: boolean;
  promoteBanner: string | null;
  pendingPermissionPolicy: PermissionPolicy;

  adapters: Record<string, AdapterConfig>;
  adapterCatalogs: Record<string, AdapterModelCatalog>;
  adapterDefaults: Record<string, AdapterModelPref>;
  probes: AdapterProbeStatus[];
  quotas: Record<string, QuotaSnapshot>;

  projects: Project[];
  activeProjectId: string | null;
  sessions: Session[];
  activeSession: Session | null;
  members: SessionMember[];
  memberOptions: Record<string, DiscoveredConfigOption[]>;
  memberStatuses: Record<string, { status: MemberStatus; turnId: string | null }>;
  permissionRules: PermissionRule[];
  pendingPermissions: PendingPermissionView[];
  sessionDiff: SessionDiffReport | null;
  sessionContext: SessionContextReport | null;
  hopPaused: HopPausedBanner | null;

  shellHistory: Record<string, ShellHistoryEntry[]>;
  shellCwd: Record<string, string>;

  messages: ChatMessage[];
  trajectories: TurnTrajectory[];
  turnEvents: Record<string, TrajectoryEvent[]>;
  turnEventSnapshots: Record<string, true>;
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

  storageSummary: StorageSummary | null;
  storageLoading: boolean;

  wsClient: WsHubClient | null;

  setTheme: (theme: ThemeId) => void;
  toggleSidebar: () => void;
  setFolderModalOpen: (open: boolean) => void;
  setSettingsOpen: (open: boolean) => void;
  setMemoryModalOpen: (open: boolean) => void;
  setDiffModalOpen: (open: boolean) => void;
  setShellOpen: (open: boolean) => void;
  setPromoteBanner: (msg: string | null) => void;
  setPendingPermissionPolicy: (policy: PermissionPolicy) => void;
  openProjectWithNativeOrModal: () => Promise<void>;

  init: () => Promise<void>;
  authenticateWithToken: (token: string) => Promise<void>;
  clearError: () => void;
  setError: (msg: string) => void;

  selectProject: (projectId: string | null) => Promise<void>;
  createAndSelectProject: (rootPath: string, name?: string) => Promise<void>;
  gitInitActiveProject: () => Promise<void>;

  selectSession: (sessionId: string) => Promise<void>;
  createAndSelectSession: (
    title?: string,
    orchestrator?: string | null,
    permissionPolicy?: PermissionPolicy
  ) => Promise<Session | null>;
  renameSession: (sessionId: string, title: string) => Promise<void>;
  archiveSession: (sessionId: string, archived: boolean) => Promise<void>;
  deleteSessionById: (sessionId: string) => Promise<void>;
  compactActiveSession: () => Promise<void>;
  refreshSessionDiff: () => Promise<void>;
  refreshSessionContext: () => Promise<void>;

  loadStorage: () => Promise<void>;
  cleanupStorage: (
    action: "clear_caches" | "delete_archived"
  ) => Promise<StorageCleanupResult>;

  updateAdapterDefaults: (
    adapterId: string,
    payload: { default_model?: string | null; default_effort?: string | null }
  ) => Promise<void>;
  updateMemberConfig: (
    handle: string,
    payload: { active_model?: string | null; active_effort?: string | null }
  ) => Promise<void>;

  sendMessage: (
    content: string,
    opts?: { model?: string | null; effort?: string | null }
  ) => Promise<void>;
  runShellCommand: (sessionId: string, command: string) => Promise<void>;
  clearShellHistory: (sessionId: string) => void;
  stopAll: () => Promise<void>;
  continueHopChain: (chainId: string) => Promise<void>;
  inspectTurn: (turnId: string) => Promise<void>;
  loadTurnEvents: (turnId: string) => Promise<void>;
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
  memberOptions: {},
  memberStatuses: {},
  permissionRules: [],
  pendingPermissions: [],
  sessionDiff: null,
  sessionContext: null,
  hopPaused: null,
  promoteBanner: null,
  messages: [],
  trajectories: [],
  turnEvents: {},
  turnEventSnapshots: {},
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
  let memoryRequest = 0;
  let trajectoryRequest = 0;
  let catalogRequest = 0;
  let sessionLoad: { id: string; events: WsEvent[] } | null = null;
  const turnRequests = new Map<string, Promise<void>>();

  const clearSession = () => {
    sessionRequest++;
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
    sidebarCollapsed:
      typeof window !== "undefined" &&
      window.matchMedia?.("(max-width: 1023px)").matches === true,
    folderModalOpen: false,
    settingsOpen: false,
    memoryModalOpen: false,
    diffModalOpen: false,
    shellOpen: false,
    pickingFolder: false,
    quotasRefreshing: false,
    promoteBanner: null,
    pendingPermissionPolicy: "worktree_smart",

    adapters: {},
    adapterCatalogs: {},
    adapterDefaults: {},
    probes: [],
    quotas: {},

    projects: [],
    activeProjectId: null,
    sessions: [],
    activeSession: null,
    members: [],
    memberOptions: {},
    memberStatuses: {},
    permissionRules: [],
    pendingPermissions: [],
    sessionDiff: null,
    sessionContext: null,
    hopPaused: null,

    shellHistory: {},
    shellCwd: {},

    messages: [],
    trajectories: [],
    turnEvents: {},
    turnEventSnapshots: {},
    turnCounters: {},

    drawerOpen: false,
    drawerTab: "trajectory",
    inspectedTurnId: null,
    inspectedTrajectory: null,
    inspectedEvents: [],

    memoryJobs: [],
    memoryFacts: [],
    storageSummary: null,
    storageLoading: false,
    wsClient: null,

    setTheme: (theme) => {
      applyThemeToDom(theme);
      set({ theme });
    },
    toggleSidebar: () => set((s) => ({ sidebarCollapsed: !s.sidebarCollapsed })),
    setFolderModalOpen: (open) => set({ folderModalOpen: open }),
    setSettingsOpen: (open) => set({ settingsOpen: open }),
    setMemoryModalOpen: (open) => set({ memoryModalOpen: open }),
    setDiffModalOpen: (open) => set({ diffModalOpen: open }),
    setShellOpen: (open) => set({ shellOpen: open }),
    setPromoteBanner: (msg) => set({ promoteBanner: msg }),
    setPendingPermissionPolicy: (policy) => set({ pendingPermissionPolicy: policy }),

    openProjectWithNativeOrModal: async () => {
      set({ folderModalOpen: true });
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
          api.listAdapters(false, true),
          api.probeAdapters(),
          api.listProjects(),
        ]);

        get().wsClient?.disconnect();
        const ws = new WsHubClient(
          (ev) => get().handleWsEvent(ev),
          () => {
            const sessionId = get().activeSession?.id;
            if (sessionId && !sessionLoad) void get().selectSession(sessionId);
          }
        );
        ws.connect(null);

        set({
          authRequired: auth.required,
          authenticated: true,
          bootstrapping: false,
          adapters: adaptersRes.adapters,
          adapterCatalogs: adaptersRes.catalogs ?? {},
          adapterDefaults: adaptersRes.defaults ?? {},
          probes: probeRes.probes,
          projects: projRes.projects,
          wsClient: ws,
        });

        void get().refreshQuotas().catch((error: unknown) => reportError(error));
        const request = ++catalogRequest;
        void api.listAdapters().then(({ catalogs }) => {
          if (catalogs && request === catalogRequest) set({ adapterCatalogs: catalogs });
        }).catch(() => undefined);

        if (projRes.projects.length > 0) {
          await get().selectProject(projRes.projects[0].id);
        } else {
          await get().selectProject(null);
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

    selectProject: async (projectId: string | null) => {
      const request = ++projectRequest;
      const memoryAtStart = ++memoryRequest;
      clearSession();
      set({ activeProjectId: projectId, sessions: [], memoryJobs: [], memoryFacts: [] });
      try {
        const [{ sessions }, mem] = await Promise.all([
          projectId
            ? api.listSessions(projectId, true)
            : api.listStandaloneSessions(true),
          projectId
            ? api.getMemory(projectId)
            : Promise.resolve({ jobs: [], facts: [] }),
        ]);
        if (request !== projectRequest) return;
        set({
          sessions,
          ...(memoryAtStart === memoryRequest && {
            memoryJobs: mem.jobs,
            memoryFacts: mem.facts,
          }),
        });
        const firstActive = sessions.find((s) => s.archived_at == null);
        if (firstActive) await get().selectSession(firstActive.id);
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
      const load =
        sessionLoad?.id === sessionId
          ? sessionLoad
          : { id: sessionId, events: [] as WsEvent[] };
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
          Object.entries(detail.statuses).map(([handle, [status, turnId]]) => [
            handle,
            { status, turnId },
          ])
        );
        set({
          activeSession: detail.session,
          members: detail.members,
          memberOptions: detail.discovered_options ?? {},
          permissionRules: detail.permission_rules,
          pendingPermissions: detail.pending_permissions,
          memberStatuses: statusMap,
          sessionContext: detail.context ?? null,
          messages: msgsRes.messages.map((message) =>
            message.metadata.streaming === true
              ? existing.get(message.id) ?? message
              : message
          ),
          trajectories: trajRes.trajectories,
          turnEventSnapshots: {},
        });
        sessionLoad = null;
        for (const event of load.events) get().handleWsEvent(event);
        const runningTurns = new Set(trajRes.trajectories.filter((turn) => turn.status === "running").map((turn) => turn.turn_id));
        for (const message of get().messages) {
          if (message.turn_id && message.metadata.streaming === true) runningTurns.add(message.turn_id);
        }
        for (const turnId of runningTurns) void get().loadTurnEvents(turnId);
        await get().refreshSessionDiff();
      } catch (error) {
        if (request === sessionRequest) {
          sessionLoad = null;
          for (const event of load.events) get().handleWsEvent(event);
          reportError(error);
        }
      }
    },

    createAndSelectSession: async (
      title = "New session",
      orchestrator?: string | null,
      permissionPolicy?: PermissionPolicy
    ) => {
      const pid = get().activeProjectId;
      const request = projectRequest;
      const payload = {
        title,
        orchestrator_member: orchestrator,
        permission_policy: permissionPolicy ?? get().pendingPermissionPolicy,
      };
      const { session } = pid
        ? await api.createSession(pid, payload)
        : await api.createStandaloneSession(payload);
      const { sessions } = pid
        ? await api.listSessions(pid, true)
        : await api.listStandaloneSessions(true);
      if (request !== projectRequest) return null;
      set({ sessions });
      await get().selectSession(session.id);
      return session;
    },

    renameSession: async (sessionId: string, title: string) => {
      const cleaned = title.trim();
      if (!cleaned) return;
      const { session } = await api.patchSession(sessionId, { title: cleaned });
      set((s) => ({
        sessions: s.sessions.map((item) => (item.id === session.id ? session : item)),
        activeSession: s.activeSession?.id === session.id ? session : s.activeSession,
      }));
    },

    archiveSession: async (sessionId: string, archived: boolean) => {
      const pid = get().activeProjectId;
      const request = projectRequest;
      const { session } = await api.patchSession(sessionId, { archived });
      const { sessions } = pid
        ? await api.listSessions(pid, true)
        : await api.listStandaloneSessions(true);
      if (request !== projectRequest) return;
      set({ sessions });
      if (
        archived &&
        (get().activeSession?.id === sessionId || sessionLoad?.id === sessionId)
      ) {
        const nextActive = sessions.find((s) => s.archived_at == null);
        if (nextActive) {
          await get().selectSession(nextActive.id);
        } else {
          clearSession();
        }
      } else if (!archived) {
        await get().selectSession(session.id);
      }
    },

    deleteSessionById: async (sessionId: string) => {
      const pid = get().activeProjectId;
      const request = projectRequest;
      await api.deleteSession(sessionId);
      const { sessions } = pid
        ? await api.listSessions(pid, true)
        : await api.listStandaloneSessions(true);
      if (request !== projectRequest) return;
      set({ sessions });
      if (get().activeSession?.id === sessionId || sessionLoad?.id === sessionId) {
        const nextActive = sessions.find((s) => s.archived_at == null);
        if (nextActive) {
          await get().selectSession(nextActive.id);
        } else {
          clearSession();
        }
      }
    },

    compactActiveSession: async () => {
      const sid = get().activeSession?.id;
      if (!sid) return;
      const { message } = await api.compactSession(sid);
      get().handleWsEvent({ type: "chat.message_created", message });
      try {
        const detail = await api.getSession(sid);
        if (get().activeSession?.id === sid) {
          set((s) => ({
            activeSession: detail.session,
            sessionContext: detail.context ?? s.sessionContext,
          }));
        }
      } catch {
        // ignore transient refresh failure
      }
    },

    refreshSessionDiff: async () => {
      const sid = get().activeSession?.id;
      if (!sid) return;
      const request = ++diffRequest;
      const selected = sessionRequest;
      try {
        const { diff } = await api.getSessionDiff(sid);
        if (request === diffRequest && selected === sessionRequest) {
          set({ sessionDiff: diff });
        }
      } catch {
        if (request === diffRequest && selected === sessionRequest) {
          set({ sessionDiff: null });
        }
      }
    },

    refreshSessionContext: async () => {
      const sid = get().activeSession?.id;
      if (!sid) return;
      try {
        const { context } = await api.getSessionContext(sid);
        if (get().activeSession?.id === sid) {
          set({ sessionContext: context });
        }
      } catch {
        // ignore transient errors
      }
    },

    loadStorage: async () => {
      set({ storageLoading: true });
      try {
        const { storage } = await api.getStorage();
        set({ storageSummary: storage });
      } catch (error) {
        reportError(error);
      } finally {
        set({ storageLoading: false });
      }
    },

    cleanupStorage: async (action) => {
      set({ storageLoading: true });
      try {
        const { result, storage } = await api.cleanupStorage(action);
        const pid = get().activeProjectId;
        const { sessions } = pid
          ? await api.listSessions(pid, true)
          : await api.listStandaloneSessions(true);
        set({ storageSummary: storage, sessions });
        return result;
      } finally {
        set({ storageLoading: false });
      }
    },

    updateAdapterDefaults: async (adapterId, payload) => {
      const { pref } = await api.updateAdapterDefaults(adapterId, payload);
      set((s) => ({
        adapterDefaults: { ...s.adapterDefaults, [adapterId]: pref },
      }));
    },

    updateMemberConfig: async (handle, payload) => {
      const sid = get().activeSession?.id;
      if (!sid) return;
      const { member } = await api.patchMemberConfig(sid, handle, payload);
      set((s) => s.activeSession?.id === sid
        ? { members: s.members.map((m) => (m.handle === member.handle ? member : m)) }
        : s
      );
    },

    sendMessage: async (content, opts) => {
      if (!content.trim()) return;
      let sid = get().activeSession?.id ?? null;
      if (!sid) {
        const created = await get().createAndSelectSession("New session");
        sid = created?.id ?? get().activeSession?.id ?? null;
        if (!sid) return;
      }
      set({ hopPaused: null });
      const { message } = await api.postMessage(sid, content, opts);
      get().handleWsEvent({ type: "chat.message_created", message });
    },

    runShellCommand: async (sessionId, command) => {
      const trimmed = command.trim();
      if (!trimmed) return;
      const currentCwd =
        get().shellCwd[sessionId] ?? get().activeSession?.integration_wt_path ?? undefined;
      const res = await api.runSessionCommand(sessionId, trimmed, currentCwd);
      const entry: ShellHistoryEntry = {
        id: `${Date.now()}_${Math.random().toString(36).slice(2, 7)}`,
        command: trimmed,
        cwd: res.cwd,
        exit_code: res.exit_code,
        stdout: res.stdout,
        stderr: res.stderr,
      };
      set((s) => ({
        shellCwd: { ...s.shellCwd, [sessionId]: res.cwd },
        shellHistory: {
          ...s.shellHistory,
          [sessionId]: [...(s.shellHistory[sessionId] ?? []), entry].slice(-80),
        },
      }));
    },

    clearShellHistory: (sessionId) =>
      set((s) => ({
        shellHistory: { ...s.shellHistory, [sessionId]: [] },
      })),

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
      set({ inspectedTurnId: turnId, inspectedEvents: get().turnEvents[turnId] ?? [], inspectedTrajectory: null });
      await get().loadTurnEvents(turnId);
    },

    loadTurnEvents: async (turnId: string) => {
      const activeSid = get().activeSession?.id;
      if (!activeSid) return;
      const selection = sessionRequest;
      const key = `${selection}:${turnId}`;
      const pending = turnRequests.get(key);
      if (pending) return pending;

      const request = api.getTrajectory(turnId).then((res) => {
        if (selection !== sessionRequest || res.trajectory.session_id !== activeSid) return;
        set((state) => {
          const events = new Map(res.events.map((event) => [event.id, event]));
          for (const event of state.turnEvents[turnId] ?? []) events.set(event.id, event);
          const sorted = [...events.values()].sort((a, b) => a.id - b.id);
          return {
            ...(state.inspectedTurnId === turnId && { inspectedTrajectory: res.trajectory, inspectedEvents: sorted }),
            turnEvents: { ...state.turnEvents, [turnId]: sorted },
            turnEventSnapshots: { ...state.turnEventSnapshots, [turnId]: true },
          };
        });
      }).catch((error: unknown) => {
        if (selection === sessionRequest) reportError(error);
      }).finally(() => turnRequests.delete(key));
      turnRequests.set(key, request);
      return request;
    },

    openDrawerTab: (tab: DrawerTab) => {
      if (tab === "memory") {
        set({ memoryModalOpen: true });
        return;
      }
      if (tab === "settings") {
        set({ settingsOpen: true });
        return;
      }
      set({ drawerOpen: true, drawerTab: tab });
    },

    closeDrawer: () => {
      get().wsClient?.subscribeTrajectory(null);
      set({ drawerOpen: false });
    },

    refreshMemory: async (query?: string) => {
      const pid = get().activeProjectId;
      if (!pid) return;
      const request = ++memoryRequest;
      const mem = await api.getMemory(pid, query);
      if (request === memoryRequest) {
        set({ memoryJobs: mem.jobs, memoryFacts: mem.facts });
      }
    },

    refreshQuotas: async () => {
      const { quotas } = await api.listQuotas();
      for (const snapshot of quotas) {
        get().handleWsEvent({ type: "quota.updated", snapshot });
      }
    },

    refreshQuotasLive: async () => {
      const request = ++catalogRequest;
      set({ quotasRefreshing: true });
      try {
        const [{ quotas }, { probes }, catalog] = await Promise.all([
          api.refreshQuotasLive(),
          api.probeAdapters(),
          api.listAdapters(true),
        ]);
        const map: Record<string, QuotaSnapshot> = {};
        for (const q of quotas) map[q.adapter_id] = q;
        if (request === catalogRequest) {
          set({ quotas: map, probes, adapters: catalog.adapters, adapterCatalogs: catalog.catalogs ?? get().adapterCatalogs });
        }
      } catch (e) {
        set({ errorBanner: e instanceof Error ? e.message : String(e) });
      } finally {
        set({ quotasRefreshing: false });
      }
    },

    handleWsEvent: (ev: WsEvent) => {
      const eventSessionId =
        "message" in ev
          ? ev.message.session_id
          : "session" in ev
            ? ev.session.id
            : "session_id" in ev
              ? ev.session_id
              : "prompt" in ev
                ? ev.prompt.session_id
                : null;
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
            return {
              messages: [...s.messages, ev.message].sort((a, b) => a.seq - b.seq),
            };
          });
          break;
        }
        case "chat.message_delta": {
          if (ev.session_id !== activeSid) return;
          set((s) => ({
            messages: s.messages.map((m) =>
              m.id === ev.message_id && m.metadata.streaming === true
                ? { ...m, content: ev.replace ? ev.delta : m.content + ev.delta }
                : m
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
            void api
              .listTrajectories(ev.message.session_id)
              .then((res) => {
                if (selected === sessionRequest && request === trajectoryRequest) {
                  set({ trajectories: res.trajectories });
                }
              })
              .catch(() => undefined);
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
          if (ev.session_id !== activeSid) return;
          set((s) => {
            const prevTurn = s.turnEvents[ev.turn_id] ?? [];
            const nextTurn = prevTurn.some((e) => e.id === ev.event.id)
              ? prevTurn
              : [...prevTurn, ev.event].sort((a, b) => a.id - b.id);
            const nextInspected =
              ev.turn_id === s.inspectedTurnId &&
              !s.inspectedEvents.some((e) => e.id === ev.event.id)
                ? [...s.inspectedEvents, ev.event].sort((a, b) => a.id - b.id)
                : s.inspectedEvents;
            return {
              turnEvents: { ...s.turnEvents, [ev.turn_id]: nextTurn },
              inspectedEvents: nextInspected,
            };
          });
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
          set((s) =>
            s.pendingPermissions.some((p) => p.request_id === ev.prompt.request_id)
              ? s
              : { pendingPermissions: [...s.pendingPermissions, ev.prompt] }
          );
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
          set((s) =>
            (s.quotas[ev.snapshot.adapter_id]?.updated_at ?? 0) > ev.snapshot.updated_at
              ? s
              : { quotas: { ...s.quotas, [ev.snapshot.adapter_id]: ev.snapshot } }
          );
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
        case "session.updated": {
          set((s) => ({
            sessions: s.sessions.some((item) => item.id === ev.session.id)
              ? s.sessions.map((item) =>
                  item.id === ev.session.id ? ev.session : item
                )
              : s.sessions,
            activeSession:
              s.activeSession?.id === ev.session.id
                ? ev.session
                : s.activeSession,
          }));
          break;
        }
        case "context.updated": {
          if (ev.session_id === activeSid) {
            set({ sessionContext: ev.report });
          }
          break;
        }
      }
    },
  };
});
