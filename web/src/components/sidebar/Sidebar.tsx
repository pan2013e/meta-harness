import { useEffect, useState } from "react";
import {
  Archive,
  ArchiveRestore,
  Check,
  ChevronRight,
  ChevronsUpDown,
  FolderGit2,
  FolderOpen,
  GitBranch,
  Layers,
  MessageSquare,
  MessageSquarePlus,
  MoreHorizontal,
  PanelLeftClose,
  Pencil,
  Settings2,
  Sparkles,
  Trash2,
} from "lucide-react";
import { useAppStore } from "../../store/useAppStore";

export function Sidebar() {
  const {
    projects,
    activeProjectId,
    sessions,
    activeSession,
    sidebarCollapsed,
    toggleSidebar,
    setFolderModalOpen,
    selectProject,
    gitInitActiveProject,
    selectSession,
    createAndSelectSession,
    renameSession,
    archiveSession,
    deleteSessionById,
    setSettingsOpen,
    setError,
  } = useAppStore();

  const [projDropdownOpen, setProjDropdownOpen] = useState(false);
  const [archivedOpen, setArchivedOpen] = useState(false);
  const [editingSessionId, setEditingSessionId] = useState<string | null>(null);
  const [editingTitle, setEditingTitle] = useState("");
  const [contextMenu, setContextMenu] = useState<{
    sessionId: string;
    x: number;
    y: number;
  } | null>(null);

  useEffect(() => {
    if (!contextMenu) return;
    const onEscape = (e: KeyboardEvent) => {
      if (e.key === "Escape") setContextMenu(null);
    };
    window.addEventListener("keydown", onEscape);
    return () => window.removeEventListener("keydown", onEscape);
  }, [contextMenu]);

  if (sidebarCollapsed) {
    return null;
  }

  const activeProject = projects.find((p) => p.id === activeProjectId) ?? null;
  const activeSessions = sessions.filter((s) => s.archived_at == null);
  const archivedSessions = sessions.filter((s) => s.archived_at != null);
  const contextTarget = contextMenu
    ? sessions.find((s) => s.id === contextMenu.sessionId) ?? null
    : null;

  const startRename = (id: string, currentTitle: string) => {
    setContextMenu(null);
    setEditingSessionId(id);
    setEditingTitle(currentTitle);
  };

  const commitRename = async () => {
    const id = editingSessionId;
    const next = editingTitle.trim();
    if (!id || !next) {
      setEditingSessionId(null);
      return;
    }
    try {
      await renameSession(id, next);
      setEditingSessionId(null);
    } catch (error) {
      setError(error instanceof Error ? error.message : String(error));
    }
  };

  const runSessionAction = (operation: Promise<void>) => {
    void operation.catch((error: unknown) => setError(error instanceof Error ? error.message : String(error)));
  };

  return (
    <aside className="fixed inset-y-0 left-0 z-40 w-[248px] max-w-[calc(100vw-2.5rem)] shrink-0 border-r border-line bg-panel flex flex-col h-full select-none shadow-xl lg:static lg:shadow-none">
      {/* Brand Header */}
      <div className="px-4 h-14 border-b border-line flex items-center justify-between gap-2 shrink-0">
        <div className="flex items-center gap-2.5 min-w-0">
          <div className="w-7 h-7 rounded-lg bg-brand-soft border border-brand/35 flex items-center justify-center text-brand-ink shrink-0">
            <Layers className="w-4 h-4" />
          </div>
          <h1 className="text-[13px] font-semibold tracking-tight text-ink truncate">
            Meta-Harness
          </h1>
        </div>
        <button
          type="button"
          title="Collapse sidebar (Ctrl+B)"
          aria-label="Collapse sidebar"
          onClick={toggleSidebar}
          className="p-1.5 rounded-lg text-ink-muted hover:text-ink hover:bg-hover transition-colors"
        >
          <PanelLeftClose className="w-3.5 h-3.5" />
        </button>
      </div>

      {/* Project Switcher */}
      <div className="p-3 border-b border-line space-y-2 shrink-0">
        <div className="flex items-center justify-between">
          <span className="text-[10.5px] uppercase tracking-wider font-semibold text-ink-muted">
            Project
          </span>
          <button
            type="button"
            onClick={() => setFolderModalOpen(true)}
            className="inline-flex items-center gap-1 px-2 py-1 rounded-lg bg-brand-soft hover:bg-brand/25 border border-brand/25 text-[11px] font-medium text-brand-ink transition-colors"
          >
            <FolderOpen className="w-3.5 h-3.5" />
            <span>Open</span>
          </button>
        </div>

        <div className="relative">
          <button
            type="button"
            onClick={() => setProjDropdownOpen((o) => !o)}
            className="w-full flex items-center justify-between gap-2 rounded-xl bg-inset hover:bg-elevated border border-line px-3 py-2 text-xs text-ink transition-colors"
          >
            <span className="flex items-center gap-2 min-w-0">
              {activeProject ? (
                <FolderGit2 className="w-3.5 h-3.5 text-brand-ink shrink-0" />
              ) : (
                <Sparkles className="w-3.5 h-3.5 text-brand-ink shrink-0" />
              )}
              <span className="truncate font-medium">
                {activeProject ? activeProject.name : "Quick Sessions"}
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
                <button
                  type="button"
                  onClick={() => {
                    setProjDropdownOpen(false);
                    void selectProject(null);
                  }}
                  className={`w-full flex items-center justify-between gap-2 px-2.5 py-2 rounded-lg text-left text-xs transition-colors ${
                    activeProjectId === null
                      ? "bg-brand-soft text-ink"
                      : "hover:bg-hover text-ink-soft"
                  }`}
                >
                  <div className="min-w-0">
                    <div className="font-medium truncate text-ink">Quick Sessions</div>
                    <div className="text-[10px] font-mono text-ink-muted truncate">
                      ~/.meta-harness/workspaces
                    </div>
                  </div>
                  {activeProjectId === null && (
                    <Check className="w-3.5 h-3.5 text-brand-ink shrink-0" />
                  )}
                </button>

                {projects.length > 0 && <div className="border-t border-line my-1" />}

                {projects.map((p) => {
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
                })}
              </div>
            </>
          )}
        </div>

        {activeProject && !activeProject.is_git_repo && (
          <div className="rounded-xl bg-warn-soft border border-warn/30 p-2.5 text-[11px] text-warn space-y-1.5">
            <p>Not a Git repo — using shared-folder mode.</p>
            <button
              type="button"
              onClick={() => void gitInitActiveProject()}
              className="inline-flex items-center gap-1.5 px-2.5 py-1 rounded-lg bg-amber-500 text-zinc-950 font-semibold hover:bg-amber-400 transition-colors"
            >
              <GitBranch className="w-3 h-3" />
              <span>Initialize Git</span>
            </button>
          </div>
        )}
      </div>

      {/* Sessions List */}
      <div className="p-3 flex-1 overflow-y-auto space-y-2">
        <div className="flex items-center justify-between">
          <span className="text-[10.5px] uppercase tracking-wider font-semibold text-ink-muted">
            Sessions
          </span>
          <button
            type="button"
            onClick={() => void createAndSelectSession("New session")}
            className="inline-flex items-center gap-1 px-2 py-1 rounded-lg bg-brand-soft hover:bg-brand/25 border border-brand/25 text-[11px] font-medium text-brand-ink transition-colors"
          >
            <MessageSquarePlus className="w-3.5 h-3.5" />
            <span>New</span>
          </button>
        </div>

        <div className="space-y-1">
          {activeSessions.map((s) => {
            const active = s.id === activeSession?.id;
            const isEditing = editingSessionId === s.id;
            return (
              <div
                key={s.id}
                onClick={() => {
                  if (!isEditing) void selectSession(s.id);
                }}
                onDoubleClick={(e) => {
                  e.stopPropagation();
                  startRename(s.id, s.title);
                }}
                onContextMenu={(e) => {
                  e.preventDefault();
                  setContextMenu({ sessionId: s.id, x: e.clientX, y: e.clientY });
                }}
                className={`group flex items-center justify-between gap-1.5 rounded-xl px-2.5 py-2 text-xs cursor-pointer transition-colors ${
                  active
                    ? "bg-brand-soft text-ink border border-brand/30 font-medium"
                    : "text-ink-soft hover:bg-hover hover:text-ink border border-transparent"
                }`}
              >
                <span className="flex items-center gap-2 min-w-0 flex-1">
                  <MessageSquare
                    className={`w-3.5 h-3.5 shrink-0 ${
                      active ? "text-brand-ink" : "text-ink-muted"
                    }`}
                  />
                  {isEditing ? (
                    <input
                      type="text"
                      aria-label="Session title"
                      autoFocus
                      value={editingTitle}
                      onClick={(e) => e.stopPropagation()}
                      onChange={(e) => setEditingTitle(e.target.value)}
                      onBlur={() => void commitRename()}
                      onKeyDown={(e) => {
                        if (e.key === "Enter") {
                          e.preventDefault();
                          void commitRename();
                        } else if (e.key === "Escape") {
                          e.preventDefault();
                          setEditingSessionId(null);
                        }
                      }}
                      className="w-full rounded bg-inset border border-brand px-1.5 py-0.5 text-xs text-ink focus:outline-none"
                    />
                  ) : (
                    <span className="truncate">{s.title}</span>
                  )}
                </span>

                {!isEditing && (
                  <button
                    type="button"
                    aria-label={`Session options for ${s.title}`}
                    title="Rename, archive, or delete"
                    onClick={(e) => {
                      e.stopPropagation();
                      const rect = e.currentTarget.getBoundingClientRect();
                      setContextMenu({
                        sessionId: s.id,
                        x: rect.right,
                        y: rect.bottom + 4,
                      });
                    }}
                    className="opacity-0 group-hover:opacity-100 focus:opacity-100 text-ink-muted hover:text-ink p-0.5 transition-opacity"
                  >
                    <MoreHorizontal className="w-3.5 h-3.5" />
                  </button>
                )}
              </div>
            );
          })}
          {activeSessions.length === 0 && (
            <p className="text-[11px] text-ink-muted px-1 py-2">
              Send a message or click New to start a session.
            </p>
          )}
        </div>

        {archivedSessions.length > 0 && (
          <div className="pt-2 border-t border-line/70 space-y-1">
            <button
              type="button"
              onClick={() => setArchivedOpen((v) => !v)}
              className="w-full flex items-center gap-1.5 px-1 py-1 text-[10.5px] font-medium text-ink-muted hover:text-ink transition-colors"
            >
              <ChevronRight
                className={`w-3 h-3 transition-transform ${archivedOpen ? "rotate-90" : ""}`}
              />
              <span>Archived ({archivedSessions.length})</span>
            </button>

            {archivedOpen && (
              <div className="space-y-1">
                {archivedSessions.map((s) => (
                  <div
                    key={s.id}
                    onContextMenu={(e) => {
                      e.preventDefault();
                      setContextMenu({ sessionId: s.id, x: e.clientX, y: e.clientY });
                    }}
                    className="group flex items-center justify-between gap-1.5 rounded-xl px-2.5 py-1.5 text-xs text-ink-muted hover:bg-hover hover:text-ink transition-colors"
                  >
                    <span className="flex items-center gap-2 min-w-0 flex-1 truncate">
                      <Archive className="w-3 h-3 shrink-0 opacity-70" />
                      <span className="truncate">{s.title}</span>
                    </span>
                    <button type="button" aria-label={`Restore ${s.title}`} title="Restore session"
                      onClick={() => runSessionAction(archiveSession(s.id, false))}
                      className="rounded-md p-1 text-ink-muted hover:bg-panel hover:text-brand-ink">
                      <ArchiveRestore className="h-3.5 w-3.5" />
                    </button>
                    <button
                      type="button"
                      aria-label={`Session options for ${s.title}`}
                      onClick={(e) => {
                        e.stopPropagation();
                        const rect = e.currentTarget.getBoundingClientRect();
                        setContextMenu({
                          sessionId: s.id,
                          x: rect.right,
                          y: rect.bottom + 4,
                        });
                      }}
                      className="opacity-0 group-hover:opacity-100 focus:opacity-100 text-ink-muted hover:text-ink p-0.5 transition-opacity"
                    >
                      <MoreHorizontal className="w-3.5 h-3.5" />
                    </button>
                  </div>
                ))}
              </div>
            )}
          </div>
        )}
      </div>

      {/* Single Settings Button (when sidebar is visible) */}
      <div className="p-3 border-t border-line shrink-0">
        <button
          type="button"
          onClick={() => setSettingsOpen(true)}
          className="w-full flex items-center gap-2 rounded-xl bg-elevated/70 hover:bg-hover border border-line px-3 py-2 text-xs text-ink font-medium transition-colors"
        >
          <Settings2 className="w-3.5 h-3.5 text-brand-ink" />
          <span>Settings</span>
        </button>
      </div>

      {/* Right-Click / More Context Menu */}
      {contextMenu && (
        <>
          <div
            className="fixed inset-0 z-50"
            onClick={() => setContextMenu(null)}
            onContextMenu={(e) => {
              e.preventDefault();
              setContextMenu(null);
            }}
          />
          <div
            role="menu"
            style={{
              left: Math.min(contextMenu.x, Math.max(8, window.innerWidth - 170)),
              top: Math.min(contextMenu.y, Math.max(8, window.innerHeight - 135)),
            }}
            className="fixed z-50 w-40 rounded-xl border border-line-strong bg-panel shadow-2xl p-1 space-y-0.5 text-xs"
          >
            <button
              type="button"
              role="menuitem"
              onClick={() => {
                if (contextTarget) startRename(contextTarget.id, contextTarget.title);
              }}
              className="w-full flex items-center gap-2 px-2.5 py-1.5 rounded-lg text-left text-ink hover:bg-hover transition-colors"
            >
              <Pencil className="w-3.5 h-3.5 text-brand-ink" />
              <span>Rename</span>
            </button>
            <button
              type="button"
              role="menuitem"
              onClick={() => {
                const id = contextMenu.sessionId;
                const isArchived = contextTarget?.archived_at != null;
                setContextMenu(null);
                runSessionAction(archiveSession(id, !isArchived));
              }}
              className="w-full flex items-center gap-2 px-2.5 py-1.5 rounded-lg text-left text-ink hover:bg-hover transition-colors"
            >
              {contextTarget?.archived_at != null ? (
                <>
                  <ArchiveRestore className="w-3.5 h-3.5 text-brand-ink" />
                  <span>Restore</span>
                </>
              ) : (
                <>
                  <Archive className="w-3.5 h-3.5 text-ink-muted" />
                  <span>Archive</span>
                </>
              )}
            </button>
            <button
              type="button"
              role="menuitem"
              onClick={() => {
                const session = contextTarget;
                setContextMenu(null);
                if (session && window.confirm(`Permanently delete “${session.title}” and its worktrees?`)) {
                  runSessionAction(deleteSessionById(session.id));
                }
              }}
              className="w-full flex items-center gap-2 px-2.5 py-1.5 rounded-lg text-left text-neg hover:bg-neg-soft transition-colors"
            >
              <Trash2 className="w-3.5 h-3.5" />
              <span>Delete</span>
            </button>
          </div>
        </>
      )}
    </aside>
  );
}
