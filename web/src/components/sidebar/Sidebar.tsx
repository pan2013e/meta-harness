import { useState } from "react";
import {
  Check,
  ChevronsUpDown,
  Crown,
  FolderGit2,
  FolderOpen,
  FolderTree,
  GitBranch,
  Layers,
  MessageSquare,
  MessageSquarePlus,
  PanelLeftClose,
  Plus,
  RefreshCw,
  Trash2,
  UserPlus,
  X,
} from "lucide-react";
import { api } from "../../api/client";
import { useAppStore } from "../../store/useAppStore";
import {
  HarnessAvatar,
  QuotaSnapshotSummary,
  StatusDot,
} from "../common/Badges";

export function Sidebar() {
  const {
    projects,
    activeProjectId,
    sessions,
    activeSession,
    members,
    memberStatuses,
    adapters,
    quotas,
    sidebarCollapsed,
    toggleSidebar,
    setFolderModalOpen,
    openProjectWithNativeOrModal,
    pickingFolder,
    quotasRefreshing,
    refreshQuotasLive,
    selectProject,
    gitInitActiveProject,
    selectSession,
    createAndSelectSession,
    deleteSessionById,
    openDrawerTab,
    setError,
  } = useAppStore();

  const [projDropdownOpen, setProjDropdownOpen] = useState(false);
  const [newSessionOpen, setNewSessionOpen] = useState(false);
  const [personaOpen, setPersonaOpen] = useState(false);

  if (sidebarCollapsed) {
    return null;
  }

  const activeProject = projects.find((p) => p.id === activeProjectId) ?? null;

  const toggleOrchestrator = async (handle: string) => {
    if (!activeSession) return;
    const next = activeSession.orchestrator_member === handle ? null : handle;
    try {
      await api.patchSession(activeSession.id, { orchestrator_member: next });
      await selectSession(activeSession.id);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  return (
    <aside className="w-[296px] shrink-0 border-r border-line bg-panel flex flex-col h-full select-none">
      {/* Brand Header */}
      <div className="px-4 py-3 border-b border-line flex items-center justify-between gap-2">
        <div className="flex items-center gap-2.5 min-w-0">
          <div className="w-7 h-7 rounded-lg bg-brand-soft border border-brand/35 flex items-center justify-center text-brand-ink shrink-0">
            <Layers className="w-4 h-4" />
          </div>
          <div className="min-w-0">
            <h1 className="text-[13px] font-semibold tracking-tight text-ink truncate">
              Meta-Harness
            </h1>
            <p className="text-[10px] text-ink-muted truncate">
              ACP Multi-Agent Switch
            </p>
          </div>
        </div>
        <div className="flex items-center gap-1">
          <button
            type="button"
            title="Refresh live provider quotas"
            onClick={() => void refreshQuotasLive()}
            className="p-1.5 rounded-lg text-ink-muted hover:text-ink hover:bg-hover transition-colors"
          >
            <RefreshCw
              className={`w-3.5 h-3.5 ${quotasRefreshing ? "animate-spin text-brand-ink" : ""}`}
            />
          </button>
          <button
            type="button"
            title="Collapse sidebar (Ctrl+B)"
            onClick={toggleSidebar}
            className="p-1.5 rounded-lg text-ink-muted hover:text-ink hover:bg-hover transition-colors"
          >
            <PanelLeftClose className="w-3.5 h-3.5" />
          </button>
        </div>
      </div>

      {/* Project Switcher */}
      <div className="p-3 border-b border-line space-y-2">
        <div className="flex items-center justify-between">
          <span className="text-[10.5px] uppercase tracking-wider font-semibold text-ink-muted">
            Project
          </span>
          <div className="flex items-center gap-1">
            <button
              type="button"
              title="Browse folders in modal"
              onClick={() => setFolderModalOpen(true)}
              className="p-1 rounded-md text-ink-muted hover:text-ink hover:bg-hover transition-colors"
            >
              <FolderTree className="w-3.5 h-3.5" />
            </button>
            <button
              type="button"
              disabled={pickingFolder}
              onClick={() => void openProjectWithNativeOrModal()}
              className="inline-flex items-center gap-1 px-2 py-0.5 rounded-md bg-brand-soft hover:bg-brand/25 text-[11px] font-medium text-brand-ink transition-colors disabled:opacity-50"
            >
              <FolderOpen className="w-3 h-3" />
              <span>{pickingFolder ? "Opening..." : "Open"}</span>
            </button>
          </div>
        </div>

        {/* Custom Project Dropdown */}
        <div className="relative">
          <button
            type="button"
            onClick={() => setProjDropdownOpen((o) => !o)}
            className="w-full flex items-center justify-between gap-2 rounded-xl bg-inset hover:bg-elevated border border-line px-3 py-2 text-xs text-ink transition-colors"
          >
            <span className="flex items-center gap-2 min-w-0">
              <FolderGit2 className="w-3.5 h-3.5 text-brand-ink shrink-0" />
              <span className="truncate font-medium">
                {activeProject ? activeProject.name : "Select or open a project..."}
              </span>
            </span>
            <ChevronsUpDown className="w-3.5 h-3.5 text-ink-muted shrink-0" />
          </button>

          {projDropdownOpen && (
            <>
              <div
                className="fixed inset-0 z-30"
                onClick={() => setProjDropdownOpen(false)}
              />
              <div className="absolute left-0 right-0 mt-1.5 rounded-xl border border-line-strong bg-panel shadow-2xl p-1 z-40 max-h-60 overflow-y-auto space-y-0.5">
                {projects.length === 0 ? (
                  <div className="px-3 py-2 text-[11px] text-ink-muted">
                    No projects opened yet
                  </div>
                ) : (
                  projects.map((p) => {
                    const selected = p.id === activeProjectId;
                    return (
                      <button
                        key={p.id}
                        type="button"
                        onClick={() => {
                          setProjDropdownOpen(false);
                          void selectProject(p.id);
                        }}
                        className={`w-full flex items-center justify-between gap-2 px-2.5 py-2 rounded-lg text-left text-xs transition-colors ${
                          selected ? "bg-brand-soft text-ink" : "hover:bg-hover text-ink-soft"
                        }`}
                      >
                        <div className="min-w-0">
                          <div className="font-medium truncate text-ink">{p.name}</div>
                          <div className="text-[10px] font-mono text-ink-muted truncate">
                            {p.root_path}
                          </div>
                        </div>
                        {selected && <Check className="w-3.5 h-3.5 text-brand-ink shrink-0" />}
                      </button>
                    );
                  })
                )}
              </div>
            </>
          )}
        </div>

        {activeProject && (
          <div className="space-y-1.5 px-0.5">
            <div
              className="text-[10.5px] text-ink-muted font-mono truncate"
              title={activeProject.root_path}
            >
              {activeProject.root_path}
            </div>
            {!activeProject.is_git_repo && (
              <div className="rounded-xl bg-warn-soft border border-warn/30 p-2.5 text-[11px] text-warn space-y-1.5">
                <p>Not a Git repo — running in shared-folder mode.</p>
                <button
                  type="button"
                  onClick={() => void gitInitActiveProject()}
                  className="inline-flex items-center gap-1.5 px-2.5 py-1 rounded-lg bg-amber-500 text-zinc-950 font-semibold hover:bg-amber-400 transition-colors"
                >
                  <GitBranch className="w-3 h-3" />
                  <span>Initialize Git for Worktrees</span>
                </button>
              </div>
            )}
          </div>
        )}
      </div>

      {/* Sessions */}
      <div className="p-3 border-b border-line space-y-2 max-h-48 overflow-y-auto">
        <div className="flex items-center justify-between">
          <span className="text-[10.5px] uppercase tracking-wider font-semibold text-ink-muted">
            Sessions
          </span>
          {activeProject && (
            <button
              type="button"
              onClick={() => setNewSessionOpen(true)}
              className="inline-flex items-center gap-1 px-2 py-0.5 rounded-md hover:bg-hover text-[11px] font-medium text-brand-ink transition-colors"
            >
              <MessageSquarePlus className="w-3.5 h-3.5" />
              <span>New</span>
            </button>
          )}
        </div>

        <div className="space-y-1">
          {sessions.map((s) => {
            const active = s.id === activeSession?.id;
            return (
              <div
                key={s.id}
                onClick={() => void selectSession(s.id)}
                className={`group flex items-center justify-between gap-2 rounded-xl px-2.5 py-2 text-xs cursor-pointer transition-colors ${
                  active
                    ? "bg-brand-soft text-ink border border-brand/30 font-medium"
                    : "text-ink-soft hover:bg-hover hover:text-ink border border-transparent"
                }`}
              >
                <span className="flex items-center gap-2 min-w-0">
                  <MessageSquare
                    className={`w-3.5 h-3.5 shrink-0 ${
                      active ? "text-brand-ink" : "text-ink-muted"
                    }`}
                  />
                  <span className="truncate">{s.title}</span>
                </span>
                <button
                  type="button"
                  title="Delete session & prune worktrees"
                  onClick={(e) => {
                    e.stopPropagation();
                    void deleteSessionById(s.id);
                  }}
                  className="opacity-0 group-hover:opacity-100 text-ink-muted hover:text-neg p-0.5 transition-opacity"
                >
                  <Trash2 className="w-3.5 h-3.5" />
                </button>
              </div>
            );
          })}
          {activeProject && sessions.length === 0 && (
            <p className="text-[11px] text-ink-muted px-1">
              Click New to launch a group session.
            </p>
          )}
        </div>
      </div>

      {/* Group Roster + Live Quotas */}
      <div className="p-3 flex-1 overflow-y-auto space-y-2.5">
        <div className="flex items-center justify-between">
          <button
            type="button"
            onClick={() => openDrawerTab("settings")}
            className="text-[10.5px] uppercase tracking-wider font-semibold text-ink-muted hover:text-ink transition-colors"
          >
            Group Roster & Quotas
          </button>
          {activeSession && (
            <button
              type="button"
              onClick={() => setPersonaOpen(true)}
              className="inline-flex items-center gap-1 px-2 py-0.5 rounded-md hover:bg-hover text-[11px] font-medium text-brand-ink transition-colors"
            >
              <UserPlus className="w-3.5 h-3.5" />
              <span>Persona</span>
            </button>
          )}
        </div>

        <div className="space-y-2">
          {members.map((m) => {
            const st = memberStatuses[m.handle];
            const adapter = adapters[m.adapter_id];
            const quota = quotas[m.adapter_id];
            const isOrch = activeSession?.orchestrator_member === m.handle;
            const accent = adapter?.accent_color ?? "#6366f1";

            return (
              <div
                key={m.handle}
                className="rounded-xl border border-line bg-elevated/70 p-2.5 space-y-2 transition-colors hover:border-line-strong"
              >
                <div className="flex items-center justify-between gap-2">
                  <div className="flex items-center gap-2.5 min-w-0">
                    <HarnessAvatar
                      handle={m.handle}
                      accentColor={accent}
                      status={st?.status}
                      size="sm"
                    />
                    <div className="min-w-0">
                      <div className="flex items-center gap-1.5">
                        <span className="text-xs font-semibold text-ink truncate">
                          @{m.handle}
                        </span>
                        {quota?.plan_label && (
                          <span className="px-1.5 py-0.2 rounded bg-inset border border-line text-[9.5px] font-mono text-ink-muted">
                            {quota.plan_label}
                          </span>
                        )}
                      </div>
                      <div className="text-[10px] text-ink-muted truncate font-mono">
                        {m.active_model ?? m.display_name}
                      </div>
                    </div>
                  </div>
                  <button
                    type="button"
                    title={
                      isOrch
                        ? "Session Orchestrator (click to clear)"
                        : "Set as Session Orchestrator"
                    }
                    onClick={() => void toggleOrchestrator(m.handle)}
                    className={`p-1.5 rounded-lg transition-colors ${
                      isOrch
                        ? "bg-warn-soft text-warn border border-warn/30"
                        : "text-ink-muted hover:text-ink hover:bg-hover"
                    }`}
                  >
                    <Crown className="w-3.5 h-3.5" />
                  </button>
                </div>

                <div className="flex items-center justify-between gap-2">
                  <StatusDot status={st?.status} />
                  {m.worktree_branch && (
                    <span
                      className="text-[10px] font-mono text-ink-muted truncate max-w-[120px] px-1.5 py-0.5 rounded bg-inset"
                      title={m.worktree_branch}
                    >
                      {m.worktree_branch.split("/").pop()}
                    </span>
                  )}
                </div>

                {quota && (
                  <div className="pt-1.5 border-t border-line">
                    <QuotaSnapshotSummary quota={quota} compact />
                  </div>
                )}
              </div>
            );
          })}
        </div>
      </div>

      {/* Modals */}
      {newSessionOpen && (
        <NewSessionModal
          members={Object.keys(adapters)}
          defaultOrch={members[0]?.handle ?? "antigravity"}
          onClose={() => setNewSessionOpen(false)}
          onCreate={async (title, orch) => {
            await createAndSelectSession(title, orch);
            setNewSessionOpen(false);
          }}
        />
      )}

      {personaOpen && activeSession && (
        <AddPersonaModal
          adapters={Object.keys(adapters)}
          onClose={() => setPersonaOpen(false)}
          onAdd={async (payload) => {
            await api.upsertMember(activeSession.id, payload);
            await selectSession(activeSession.id);
            setPersonaOpen(false);
          }}
        />
      )}
    </aside>
  );
}

function NewSessionModal({
  members,
  defaultOrch,
  onClose,
  onCreate,
}: {
  members: string[];
  defaultOrch: string;
  onClose: () => void;
  onCreate: (title: string, orchestrator: string | null) => Promise<void>;
}) {
  const [title, setTitle] = useState("Feature Discussion");
  const [orch, setOrch] = useState(defaultOrch);
  const [err, setErr] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);

  const submit = async () => {
    setSaving(true);
    setErr(null);
    try {
      await onCreate(title.trim() || "Group Session", orch.trim() || null);
    } catch (e) {
      setErr(e instanceof Error ? e.message : String(e));
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="fixed inset-0 z-50 bg-black/70 backdrop-blur-xs flex items-center justify-center p-4">
      <div className="w-full max-w-sm rounded-2xl border border-line-strong bg-panel shadow-2xl p-5 space-y-4">
        <div className="flex items-center justify-between">
          <h2 className="text-sm font-semibold text-ink">New Group Session</h2>
          <button
            type="button"
            onClick={onClose}
            className="text-ink-muted hover:text-ink"
          >
            <X className="w-4 h-4" />
          </button>
        </div>

        <div className="space-y-3 text-xs">
          <div>
            <label className="block text-ink-soft font-medium mb-1">Session Title</label>
            <input
              type="text"
              value={title}
              onChange={(e) => setTitle(e.target.value)}
              className="w-full rounded-xl bg-inset border border-line focus:border-brand px-3 py-2 text-ink focus:outline-none"
            />
          </div>
          <div>
            <label className="block text-ink-soft font-medium mb-1.5">
              Session Orchestrator
            </label>
            <div className="flex flex-wrap gap-1.5">
              {members.map((m) => (
                <button
                  key={m}
                  type="button"
                  onClick={() => setOrch(m)}
                  className={`px-2.5 py-1 rounded-lg border font-mono text-[11px] transition-colors ${
                    orch === m
                      ? "bg-brand-soft border-brand text-brand-ink font-semibold"
                      : "bg-inset border-line text-ink-soft hover:bg-hover"
                  }`}
                >
                  @{m}
                </button>
              ))}
              <button
                type="button"
                onClick={() => setOrch("")}
                className={`px-2.5 py-1 rounded-lg border text-[11px] transition-colors ${
                  orch === ""
                    ? "bg-brand-soft border-brand text-brand-ink font-semibold"
                    : "bg-inset border-line text-ink-soft hover:bg-hover"
                }`}
              >
                None
              </button>
            </div>
          </div>
        </div>

        {err && <p className="text-xs text-neg">{err}</p>}

        <div className="flex justify-end gap-2 pt-1">
          <button
            type="button"
            onClick={onClose}
            className="text-xs text-ink-soft hover:text-ink px-3 py-1.5"
          >
            Cancel
          </button>
          <button
            type="button"
            disabled={saving}
            onClick={() => void submit()}
            className="px-4 py-1.5 rounded-lg bg-brand hover:bg-brand-hover disabled:opacity-50 text-xs font-medium text-white"
          >
            {saving ? "Creating..." : "Create"}
          </button>
        </div>
      </div>
    </div>
  );
}

function AddPersonaModal({
  adapters,
  onClose,
  onAdd,
}: {
  adapters: string[];
  onClose: () => void;
  onAdd: (payload: {
    handle: string;
    adapter_id: string;
    display_name: string;
    persona_prompt: string;
  }) => Promise<void>;
}) {
  const [handle, setHandle] = useState("codex-reviewer");
  const [adapterId, setAdapterId] = useState(adapters[0] ?? "codex");
  const [displayName, setDisplayName] = useState("Codex Reviewer");
  const [prompt, setPrompt] = useState(
    "Focus strictly on security, race conditions, and test coverage."
  );
  const [err, setErr] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);

  const submit = async () => {
    setSaving(true);
    setErr(null);
    try {
      await onAdd({
        handle,
        adapter_id: adapterId,
        display_name: displayName,
        persona_prompt: prompt,
      });
    } catch (e) {
      setErr(e instanceof Error ? e.message : String(e));
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="fixed inset-0 z-50 bg-black/70 backdrop-blur-xs flex items-center justify-center p-4">
      <div className="w-full max-w-md rounded-2xl border border-line-strong bg-panel shadow-2xl p-5 space-y-4">
        <div className="flex items-center justify-between">
          <h2 className="text-sm font-semibold text-ink">Add Member Persona</h2>
          <button
            type="button"
            onClick={onClose}
            className="text-ink-muted hover:text-ink"
          >
            <X className="w-4 h-4" />
          </button>
        </div>

        <div className="space-y-3 text-xs">
          <div>
            <label className="block text-ink-soft font-medium mb-1">@handle</label>
            <input
              type="text"
              value={handle}
              onChange={(e) => setHandle(e.target.value)}
              className="w-full rounded-xl bg-inset border border-line focus:border-brand px-3 py-2 font-mono text-ink focus:outline-none"
            />
          </div>
          <div>
            <label className="block text-ink-soft font-medium mb-1.5">
              Harness Adapter (shared quota pool)
            </label>
            <div className="flex flex-wrap gap-1.5">
              {adapters.map((a) => (
                <button
                  key={a}
                  type="button"
                  onClick={() => setAdapterId(a)}
                  className={`px-2.5 py-1 rounded-lg border font-mono text-[11px] transition-colors ${
                    adapterId === a
                      ? "bg-brand-soft border-brand text-brand-ink font-semibold"
                      : "bg-inset border-line text-ink-soft hover:bg-hover"
                  }`}
                >
                  {a}
                </button>
              ))}
            </div>
          </div>
          <div>
            <label className="block text-ink-soft font-medium mb-1">Display Name</label>
            <input
              type="text"
              value={displayName}
              onChange={(e) => setDisplayName(e.target.value)}
              className="w-full rounded-xl bg-inset border border-line focus:border-brand px-3 py-2 text-ink focus:outline-none"
            />
          </div>
          <div>
            <label className="block text-ink-soft font-medium mb-1">Persona Prompt</label>
            <textarea
              rows={3}
              value={prompt}
              onChange={(e) => setPrompt(e.target.value)}
              className="w-full rounded-xl bg-inset border border-line focus:border-brand px-3 py-2 text-ink focus:outline-none"
            />
          </div>
        </div>

        {err && <p className="text-xs text-neg">{err}</p>}

        <div className="flex justify-end gap-2 pt-1">
          <button
            type="button"
            onClick={onClose}
            className="text-xs text-ink-soft hover:text-ink px-3 py-1.5"
          >
            Cancel
          </button>
          <button
            type="button"
            disabled={saving}
            onClick={() => void submit()}
            className="inline-flex items-center gap-1.5 px-4 py-1.5 rounded-lg bg-brand hover:bg-brand-hover disabled:opacity-50 text-xs font-medium text-white"
          >
            <Plus className="w-3.5 h-3.5" />
            <span>{saving ? "Adding..." : "Add Member"}</span>
          </button>
        </div>
      </div>
    </div>
  );
}
