import { useState } from "react";
import {
  Crown,
  FolderGit2,
  FolderOpen,
  GitBranch,
  MessageSquarePlus,
  Plus,
  Trash2,
  UserPlus,
  X,
} from "lucide-react";
import { api } from "../../api/client";
import { useAppStore } from "../../store/useAppStore";
import type { FsEntry } from "../../types/api";
import { QuotaMeter, StatusDot } from "../common/Badges";

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
    selectProject,
    createAndSelectProject,
    gitInitActiveProject,
    selectSession,
    createAndSelectSession,
    deleteSessionById,
    openDrawerTab,
    setError,
  } = useAppStore();

  const [folderModalOpen, setFolderModalOpen] = useState(false);
  const [newSessionOpen, setNewSessionOpen] = useState(false);
  const [personaOpen, setPersonaOpen] = useState(false);

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
    <aside className="w-72 shrink-0 border-r border-zinc-800 bg-zinc-900/60 flex flex-col h-full select-none">
      {/* App Header */}
      <div className="px-4 py-3 border-b border-zinc-800 flex items-center justify-between">
        <div>
          <h1 className="text-sm font-semibold tracking-tight text-zinc-100">
            Meta-Harness
          </h1>
          <p className="text-[11px] text-zinc-500">ACP Multi-Agent Switch</p>
        </div>
        <button
          type="button"
          onClick={() => openDrawerTab("settings")}
          className="text-[11px] px-2 py-1 rounded bg-zinc-800 hover:bg-zinc-700 text-zinc-300 transition-colors"
        >
          Quotas
        </button>
      </div>

      {/* Project Picker */}
      <div className="p-3 border-b border-zinc-800 space-y-2">
        <div className="flex items-center justify-between">
          <span className="text-[11px] uppercase tracking-wider font-medium text-zinc-400">
            Project
          </span>
          <button
            type="button"
            onClick={() => setFolderModalOpen(true)}
            className="inline-flex items-center gap-1 text-xs text-indigo-400 hover:text-indigo-300"
          >
            <FolderOpen className="w-3.5 h-3.5" />
            <span>Open</span>
          </button>
        </div>

        <select
          aria-label="Active project"
          value={activeProjectId ?? ""}
          onChange={(e) => {
            if (e.target.value) void selectProject(e.target.value);
          }}
          className="w-full rounded-md bg-zinc-950 border border-zinc-800 px-2.5 py-1.5 text-xs text-zinc-100 focus:outline-none focus:border-indigo-500"
        >
          {projects.length === 0 && <option value="">No projects open</option>}
          {projects.map((p) => (
            <option key={p.id} value={p.id}>
              {p.name}
            </option>
          ))}
        </select>

        {activeProject && (
          <div className="space-y-1.5">
            <div
              className="text-[11px] text-zinc-500 font-mono truncate"
              title={activeProject.root_path}
            >
              {activeProject.root_path}
            </div>
            {!activeProject.is_git_repo && (
              <div className="rounded-md bg-amber-500/10 border border-amber-500/30 p-2 text-[11px] text-amber-200 space-y-1.5">
                <p>Not a Git repo — using serialized shared-folder mode.</p>
                <button
                  type="button"
                  onClick={() => void gitInitActiveProject()}
                  className="inline-flex items-center gap-1 px-2 py-1 rounded bg-amber-500 text-zinc-950 font-medium hover:bg-amber-400"
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
      <div className="p-3 border-b border-zinc-800 space-y-2 max-h-48 overflow-y-auto">
        <div className="flex items-center justify-between">
          <span className="text-[11px] uppercase tracking-wider font-medium text-zinc-400">
            Sessions
          </span>
          {activeProject && (
            <button
              type="button"
              onClick={() => setNewSessionOpen(true)}
              className="inline-flex items-center gap-1 text-xs text-indigo-400 hover:text-indigo-300"
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
                className={`group flex items-center justify-between rounded-md px-2.5 py-1.5 text-xs cursor-pointer transition-colors ${
                  active
                    ? "bg-indigo-500/20 text-indigo-200 border border-indigo-500/30"
                    : "text-zinc-300 hover:bg-zinc-800/70"
                }`}
                onClick={() => void selectSession(s.id)}
              >
                <span className="truncate">{s.title}</span>
                <button
                  type="button"
                  title="Delete session & prune worktrees"
                  onClick={(e) => {
                    e.stopPropagation();
                    void deleteSessionById(s.id);
                  }}
                  className="opacity-0 group-hover:opacity-100 text-zinc-500 hover:text-rose-400 p-0.5"
                >
                  <Trash2 className="w-3 h-3" />
                </button>
              </div>
            );
          })}
          {activeProject && sessions.length === 0 && (
            <p className="text-[11px] text-zinc-500">Create a session to start.</p>
          )}
        </div>
      </div>

      {/* Member Roster + Quotas */}
      <div className="p-3 flex-1 overflow-y-auto space-y-2.5">
        <div className="flex items-center justify-between">
          <span className="text-[11px] uppercase tracking-wider font-medium text-zinc-400">
            Group Roster
          </span>
          {activeSession && (
            <button
              type="button"
              onClick={() => setPersonaOpen(true)}
              className="inline-flex items-center gap-1 text-xs text-indigo-400 hover:text-indigo-300"
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
                className="rounded-lg border border-zinc-800 bg-zinc-950/70 p-2.5 space-y-2"
              >
                <div className="flex items-center justify-between gap-2">
                  <div className="flex items-center gap-2 min-w-0">
                    <span
                      className="h-2.5 w-2.5 rounded-full shrink-0"
                      style={{ backgroundColor: accent }}
                    />
                    <div className="min-w-0">
                      <div className="text-xs font-medium text-zinc-100 truncate">
                        @{m.handle}
                      </div>
                      <div className="text-[10px] text-zinc-500 truncate">
                        {m.display_name} · {m.active_model ?? "default"}
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
                    className={`p-1 rounded transition-colors ${
                      isOrch
                        ? "bg-amber-500/20 text-amber-400"
                        : "text-zinc-600 hover:text-zinc-300"
                    }`}
                  >
                    <Crown className="w-3.5 h-3.5" />
                  </button>
                </div>

                <div className="flex items-center justify-between">
                  <StatusDot status={st?.status} />
                  {m.worktree_branch && (
                    <span
                      className="text-[10px] font-mono text-zinc-500 truncate max-w-[110px]"
                      title={m.worktree_branch}
                    >
                      {m.worktree_branch.split("/").pop()}
                    </span>
                  )}
                </div>

                {quota && (
                  <div className="space-y-1 pt-1 border-t border-zinc-800/70">
                    <QuotaMeter
                      label="5h"
                      pct={quota.window_5h_remaining_pct}
                      rateLimited={quota.rate_limited_until !== null}
                    />
                    <QuotaMeter label="7d" pct={quota.weekly_remaining_pct} />
                  </div>
                )}
              </div>
            );
          })}
        </div>
      </div>

      {/* Modals */}
      {folderModalOpen && (
        <FolderPickerModal
          onClose={() => setFolderModalOpen(false)}
          onSelect={async (path) => {
            await createAndSelectProject(path);
            setFolderModalOpen(false);
          }}
        />
      )}

      {newSessionOpen && (
        <NewSessionModal
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

function FolderPickerModal({
  onClose,
  onSelect,
}: {
  onClose: () => void;
  onSelect: (path: string) => Promise<void>;
}) {
  const [current, setCurrent] = useState("");
  const [parent, setParent] = useState<string | null>(null);
  const [entries, setEntries] = useState<FsEntry[] | null>(null);
  const [manualPath, setManualPath] = useState("");
  const [err, setErr] = useState<string | null>(null);

  const load = async (path?: string) => {
    try {
      setErr(null);
      const res = await api.listFs(path);
      setCurrent(res.current);
      setParent(res.parent);
      setEntries(res.entries);
      if (res.current) setManualPath(res.current);
    } catch (e) {
      setErr(e instanceof Error ? e.message : String(e));
    }
  };

  if (entries === null) {
    void load();
  }

  return (
    <div className="fixed inset-0 z-50 bg-black/70 flex items-center justify-center p-4">
      <div className="w-full max-w-lg rounded-xl border border-zinc-800 bg-zinc-900 shadow-2xl overflow-hidden">
        <div className="px-4 py-3 border-b border-zinc-800 flex items-center justify-between">
          <h2 className="text-sm font-semibold">Open Project Folder</h2>
          <button type="button" onClick={onClose} className="text-zinc-400 hover:text-zinc-200">
            <X className="w-4 h-4" />
          </button>
        </div>

        <div className="p-4 space-y-3">
          <div className="flex gap-2">
            <input
              type="text"
              value={manualPath}
              onChange={(e) => setManualPath(e.target.value)}
              placeholder="Enter local folder path (e.g. C:\repos\app or ~/repos/app)"
              className="flex-1 rounded-md bg-zinc-950 border border-zinc-800 px-3 py-1.5 text-xs font-mono text-zinc-100 focus:outline-none focus:border-indigo-500"
            />
            <button
              type="button"
              onClick={() => void load(manualPath)}
              className="px-3 py-1.5 rounded-md bg-zinc-800 hover:bg-zinc-700 text-xs"
            >
              Browse
            </button>
          </div>

          {err && <p className="text-xs text-rose-400">{err}</p>}

          <div className="h-64 overflow-y-auto rounded-md border border-zinc-800 bg-zinc-950 divide-y divide-zinc-900">
            {current && (
              <button
                type="button"
                onClick={() => void load(parent ?? "")}
                className="w-full px-3 py-2 text-left text-xs text-zinc-400 hover:bg-zinc-900 flex items-center gap-2"
              >
                <span>.. (Up)</span>
              </button>
            )}
            {(entries ?? []).map((entry) => (
              <button
                key={entry.path}
                type="button"
                onClick={() => void load(entry.path)}
                className="w-full px-3 py-2 text-left text-xs text-zinc-200 hover:bg-zinc-900 flex items-center justify-between"
              >
                <span className="flex items-center gap-2 truncate">
                  {entry.is_git_repo ? (
                    <FolderGit2 className="w-3.5 h-3.5 text-emerald-400 shrink-0" />
                  ) : (
                    <FolderOpen className="w-3.5 h-3.5 text-zinc-500 shrink-0" />
                  )}
                  <span className="truncate">{entry.name}</span>
                </span>
                {entry.is_git_repo && (
                  <span className="text-[10px] px-1.5 py-0.5 rounded bg-emerald-500/10 text-emerald-300">
                    git
                  </span>
                )}
              </button>
            ))}
          </div>
        </div>

        <div className="px-4 py-3 border-t border-zinc-800 flex justify-end gap-2">
          <button
            type="button"
            onClick={onClose}
            className="px-3 py-1.5 rounded-md text-xs text-zinc-400 hover:text-zinc-200"
          >
            Cancel
          </button>
          <button
            type="button"
            disabled={!manualPath.trim()}
            onClick={() => void onSelect(manualPath.trim())}
            className="px-3 py-1.5 rounded-md bg-indigo-600 hover:bg-indigo-500 disabled:opacity-40 text-xs font-medium text-white"
          >
            Open Project
          </button>
        </div>
      </div>
    </div>
  );
}

function NewSessionModal({
  defaultOrch,
  onClose,
  onCreate,
}: {
  defaultOrch: string;
  onClose: () => void;
  onCreate: (title: string, orchestrator: string | null) => Promise<void>;
}) {
  const [title, setTitle] = useState("Feature Discussion");
  const [orch, setOrch] = useState(defaultOrch);

  return (
    <div className="fixed inset-0 z-50 bg-black/70 flex items-center justify-center p-4">
      <div className="w-full max-w-sm rounded-xl border border-zinc-800 bg-zinc-900 p-4 space-y-3">
        <div className="flex items-center justify-between">
          <h2 className="text-sm font-semibold">New Group Session</h2>
          <button type="button" onClick={onClose} className="text-zinc-400">
            <X className="w-4 h-4" />
          </button>
        </div>
        <div className="space-y-2">
          <label className="block text-xs text-zinc-400">Session Title</label>
          <input
            type="text"
            value={title}
            onChange={(e) => setTitle(e.target.value)}
            className="w-full rounded-md bg-zinc-950 border border-zinc-800 px-3 py-1.5 text-xs text-zinc-100"
          />
          <label className="block text-xs text-zinc-400">Orchestrator Handle</label>
          <input
            type="text"
            value={orch}
            onChange={(e) => setOrch(e.target.value)}
            placeholder="antigravity / claude / codex"
            className="w-full rounded-md bg-zinc-950 border border-zinc-800 px-3 py-1.5 text-xs text-zinc-100"
          />
        </div>
        <div className="flex justify-end gap-2 pt-2">
          <button type="button" onClick={onClose} className="text-xs text-zinc-400 px-3 py-1.5">
            Cancel
          </button>
          <button
            type="button"
            onClick={() => void onCreate(title.trim() || "Session", orch.trim() || null)}
            className="px-3 py-1.5 rounded-md bg-indigo-600 hover:bg-indigo-500 text-xs font-medium text-white"
          >
            Create
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

  return (
    <div className="fixed inset-0 z-50 bg-black/70 flex items-center justify-center p-4">
      <div className="w-full max-w-md rounded-xl border border-zinc-800 bg-zinc-900 p-4 space-y-3">
        <div className="flex items-center justify-between">
          <h2 className="text-sm font-semibold">Add Member Persona</h2>
          <button type="button" onClick={onClose} className="text-zinc-400">
            <X className="w-4 h-4" />
          </button>
        </div>
        <div className="space-y-2 text-xs">
          <div>
            <label className="block text-zinc-400 mb-1">@handle</label>
            <input
              type="text"
              value={handle}
              onChange={(e) => setHandle(e.target.value)}
              className="w-full rounded bg-zinc-950 border border-zinc-800 px-2.5 py-1.5 font-mono"
            />
          </div>
          <div>
            <label className="block text-zinc-400 mb-1">Underlying Harness Adapter (shared quota)</label>
            <select
              value={adapterId}
              onChange={(e) => setAdapterId(e.target.value)}
              className="w-full rounded bg-zinc-950 border border-zinc-800 px-2.5 py-1.5"
            >
              {adapters.map((a) => (
                <option key={a} value={a}>
                  {a}
                </option>
              ))}
            </select>
          </div>
          <div>
            <label className="block text-zinc-400 mb-1">Display Name</label>
            <input
              type="text"
              value={displayName}
              onChange={(e) => setDisplayName(e.target.value)}
              className="w-full rounded bg-zinc-950 border border-zinc-800 px-2.5 py-1.5"
            />
          </div>
          <div>
            <label className="block text-zinc-400 mb-1">Persona Prompt</label>
            <textarea
              rows={3}
              value={prompt}
              onChange={(e) => setPrompt(e.target.value)}
              className="w-full rounded bg-zinc-950 border border-zinc-800 px-2.5 py-1.5"
            />
          </div>
        </div>
        <div className="flex justify-end gap-2 pt-2">
          <button type="button" onClick={onClose} className="text-xs text-zinc-400 px-3 py-1.5">
            Cancel
          </button>
          <button
            type="button"
            onClick={() =>
              void onAdd({
                handle,
                adapter_id: adapterId,
                display_name: displayName,
                persona_prompt: prompt,
              })
            }
            className="inline-flex items-center gap-1 px-3 py-1.5 rounded bg-indigo-600 hover:bg-indigo-500 text-xs font-medium text-white"
          >
            <Plus className="w-3.5 h-3.5" />
            <span>Add Member</span>
          </button>
        </div>
      </div>
    </div>
  );
}
