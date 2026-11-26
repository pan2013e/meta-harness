import { lazy, Suspense, useEffect, useMemo, useRef, useState } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import {
  Brain,
  CheckCircle2,
  ChevronDown,
  Crown,
  FolderGit2,
  FolderOpen,
  GitBranch,
  GitMerge,
  Layers,
  Loader2,
  MessageSquare,
  PanelLeftOpen,
  Settings2,
  Terminal,
} from "lucide-react";
import { api } from "../../api/client";
import { useAppStore } from "../../store/useAppStore";
import type { ChatMessage } from "../../types/api";
import { Composer } from "./Composer";
import { MessageRow } from "./MessageRow";

const WebShellDrawer = lazy(async () => ({ default: (await import("./WebShellDrawer")).WebShellDrawer }));

const NO_CONSULT_REPLIES: ChatMessage[] = [];

const PROJECT_PROMPTS = [
  "Inspect this repository and summarize its architecture",
  "Plan a new feature and coordinate implementation",
  "Find edge cases or bugs in recent changes",
  "Add comprehensive tests for the core module",
];

const STANDALONE_PROMPTS = [
  "Plan a new project from scratch",
  "Create a starter web app",
  "Prototype a command-line tool",
];

export function ChatPane() {
  const {
    projects,
    activeProjectId,
    activeSession,
    messages,
    hopPaused,
    pendingPermissions,
    sidebarCollapsed,
    toggleSidebar,
    continueHopChain,
    stopAll,
    setFolderModalOpen,
    selectProject,
    gitInitActiveProject,
    probes,
    setSettingsOpen,
    setMemoryModalOpen,
    setDiffModalOpen,
    shellOpen,
    setShellOpen,
    sessionDiff,
    refreshSessionDiff,
    promoteBanner,
    memoryJobs,
    memoryFacts,
    memberStatuses,
  } = useAppStore();

  const parentRef = useRef<HTMLDivElement | null>(null);
  const followTail = useRef(true);
  const [projPickerOpen, setProjPickerOpen] = useState(false);
  const [draftPreset, setDraftPreset] = useState<{
    seq: number;
    text: string;
  } | null>(null);

  const activeProject = projects.find((p) => p.id === activeProjectId) ?? null;
  const activeOrchestrator = useAppStore((state) =>
    state.members.find((member) => member.handle === state.activeSession?.orchestrator_member)
  );
  const orchestratorAvailable = probes.some(
    (probe) => probe.installed && probe.adapter_id === activeOrchestrator?.adapter_id
  );
  const startingMembers = Object.entries(memberStatuses).filter(
    ([, member]) => member.status === "spawning"
  );

  const stat = sessionDiff?.diff_stat;
  const hasUnpromotedChanges =
    activeSession?.isolation_mode === "worktree_per_member" &&
    ((stat?.files_changed ?? 0) > 0 || (sessionDiff?.ahead_commits ?? 0) > 0);
  const memoryCount = memoryJobs.length + memoryFacts.length;
  const readyCount = probes.filter((p) => p.installed).length;

  const { visibleMessages, repliesByParent } = useMemo(() => {
    const replies = new Map<string, ChatMessage[]>();
    const visible: ChatMessage[] = [];
    for (const m of messages) {
      if (m.kind === "consult_thread" && m.parent_msg_id) {
        const list = replies.get(m.parent_msg_id) ?? [];
        list.push(m);
        replies.set(m.parent_msg_id, list);
      } else {
        visible.push(m);
      }
    }
    return { visibleMessages: visible, repliesByParent: replies };
  }, [messages]);

  const virtualizer = useVirtualizer({
    count: visibleMessages.length,
    getItemKey: (index) => visibleMessages[index].id,
    getScrollElement: () => parentRef.current,
    estimateSize: () => 118,
    overscan: 8,
  });

  useEffect(() => {
    followTail.current = true;
  }, [activeSession?.id]);

  useEffect(() => {
    if (followTail.current && visibleMessages.length > 0) {
      virtualizer.scrollToIndex(visibleMessages.length - 1, { align: "end" });
    }
  }, [visibleMessages.length, visibleMessages.at(-1)?.content, virtualizer]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.ctrlKey || e.metaKey) && e.key === "`" && activeSession) {
        e.preventDefault();
        setShellOpen(!shellOpen);
        return;
      }
      if (!activeSession || pendingPermissions.length === 0) return;
      const top = pendingPermissions[0];
      const key = e.key.toLowerCase();
      if (!e.altKey || (key !== "a" && key !== "r")) return;
      const kind = key === "a" ? "allow_once" : "reject_once";
      const option = top.options.find((item) => item.kind === kind);
      if (!option) return;
      e.preventDefault();
      void api
        .resolvePermission(activeSession.id, top.request_id, option.option_id)
        .catch((error: unknown) =>
          useAppStore
            .getState()
            .setError(error instanceof Error ? error.message : String(error))
        );
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [activeSession, pendingPermissions, shellOpen, setShellOpen]);

  return (
    <main className="flex-1 flex flex-col min-w-0 bg-app h-full">
      {/* Single Clean Top Header */}
      <header className="flex h-14 shrink-0 items-center justify-between gap-2 border-b border-line bg-panel/70 px-3 sm:px-5">
        <div className="flex items-center gap-2.5 min-w-0">
          {sidebarCollapsed && (
            <button
              type="button"
              title="Expand sidebar (Ctrl+B)"
              aria-label="Toggle sidebar"
              onClick={toggleSidebar}
              className="p-1.5 rounded-lg bg-elevated hover:bg-hover border border-line text-ink-soft hover:text-ink transition-colors"
            >
              <PanelLeftOpen className="w-3.5 h-3.5" />
            </button>
          )}
          {!sidebarCollapsed && (
            <div className="lg:hidden">
              <button
                type="button"
                title="Toggle sidebar (Ctrl+B)"
                aria-label="Toggle sidebar"
                onClick={toggleSidebar}
                className="p-1.5 rounded-lg bg-elevated hover:bg-hover border border-line text-ink-soft hover:text-ink transition-colors"
              >
                <PanelLeftOpen className="w-3.5 h-3.5" />
              </button>
            </div>
          )}

          {activeSession ? (
            <div className="flex items-center gap-2.5 min-w-0">
              <h2 className="truncate text-[14.5px] font-semibold tracking-tight text-ink">
                {activeSession.title}
              </h2>
              {activeOrchestrator && orchestratorAvailable && (
                <span className="hidden sm:inline-flex items-center gap-1 px-2 py-0.5 rounded-full bg-warn-soft border border-warn/30 text-[11px] text-warn font-medium shrink-0">
                  <Crown className="w-3 h-3" />
                  <span>@{activeOrchestrator.handle}</span>
                </span>
              )}
            </div>
          ) : (
            <div className="text-xs text-ink-muted font-medium">
              {activeProject ? activeProject.name : "Quick Sessions"}
            </div>
          )}
        </div>

        <div className="flex shrink-0 items-center gap-1.5">
          {promoteBanner && (
            <span className="inline-flex items-center gap-1.5 px-2.5 py-1 rounded-lg bg-pos-soft border border-pos/30 text-[11px] font-medium text-pos">
              <CheckCircle2 className="w-3.5 h-3.5" />
              <span className="hidden sm:inline">{promoteBanner}</span>
              <span className="sm:hidden">Merged</span>
            </span>
          )}

          {hasUnpromotedChanges && (
            <button
              type="button"
              onClick={() => {
                void refreshSessionDiff();
                setDiffModalOpen(true);
              }}
              className="inline-flex items-center gap-2 px-3 py-1.5 rounded-xl bg-emerald-600/15 hover:bg-emerald-600/25 border border-emerald-500/35 text-xs font-medium text-pos transition-colors"
            >
              <GitMerge className="w-3.5 h-3.5" />
              <span className="font-mono text-[11px]">
                +{stat?.insertions ?? 0} -{stat?.deletions ?? 0}
              </span>
              <span>
                {activeSession?.project_id !== null ? "Review & Promote" : "Review Diff"}
              </span>
            </button>
          )}

          {activeSession && (
            <button
              type="button"
              aria-pressed={shellOpen}
              title="Toggle in-web shell (Ctrl+`)"
              onClick={() => setShellOpen(!shellOpen)}
              className={`inline-flex items-center gap-1.5 rounded-lg px-2.5 py-1.5 text-xs transition-colors ${
                shellOpen
                  ? "bg-brand-soft text-brand-ink font-medium"
                  : "text-ink-soft hover:bg-hover hover:text-ink"
              }`}
            >
              <Terminal className="w-3.5 h-3.5" />
              <span className="hidden md:inline">Shell</span>
            </button>
          )}

          {activeProject && (
            <button
              type="button"
              aria-label="Memory"
              onClick={() => setMemoryModalOpen(true)}
              className="inline-flex items-center gap-1.5 rounded-lg px-2.5 py-1.5 text-xs text-ink-soft hover:bg-hover hover:text-ink transition-colors"
            >
              <Brain className="w-3.5 h-3.5" />
              <span className="hidden md:inline">Memory</span>
              {memoryCount > 0 && (
                <span className="px-1.5 py-0.2 rounded-full bg-elevated border border-line font-mono text-[10px] text-ink-muted">
                  {memoryCount}
                </span>
              )}
            </button>
          )}

          {/* Only render top-right Settings button when sidebar is collapsed (no duplicate settings buttons) */}
          {sidebarCollapsed && (
            <button
              type="button"
              aria-label="Settings"
              title="Settings"
              onClick={() => setSettingsOpen(true)}
              className="rounded-lg p-2 text-ink-soft hover:bg-hover hover:text-ink transition-colors"
            >
              <Settings2 className="h-4 w-4" />
            </button>
          )}
        </div>
      </header>

      {/* Chat-First Welcome Page when no active session */}
      {!activeSession ? (
        <div className="flex-1 flex flex-col justify-between overflow-y-auto">
          <div className="flex-1 flex flex-col items-center justify-center px-4 sm:px-6 py-8">
            <div className="w-full max-w-[44rem] space-y-6 text-center">
              <div className="mx-auto w-11 h-11 rounded-2xl bg-brand-soft border border-brand/30 flex items-center justify-center text-brand-ink shadow-sm">
                <Layers className="w-5 h-5" />
              </div>

              <div className="space-y-2">
                <h2 className="text-2xl font-bold tracking-tight text-ink">
                  What would you like to build?
                </h2>
                {readyCount === 0 && (
                  <p className="text-xs text-ink-muted max-w-md mx-auto">
                    Configure an ACP CLI harness in Settings to start.
                  </p>
                )}
              </div>

              {/* Interactive Project Pill Switcher */}
              <div className="flex flex-wrap items-center justify-center gap-2 relative">
                <div className="relative">
                  <button
                    type="button"
                    onClick={() => setProjPickerOpen((v) => !v)}
                    className="inline-flex items-center gap-2 px-3.5 py-1.5 rounded-full bg-panel hover:bg-hover border border-line-strong text-xs text-ink transition-colors"
                  >
                    <FolderGit2 className="w-3.5 h-3.5 text-brand-ink" />
                    <span className="font-medium">
                      {activeProject ? activeProject.name : "Quick Sessions"}
                    </span>
                    {activeProject && (
                      <span
                        className={`text-[10px] font-mono px-1.5 py-0.2 rounded ${
                          activeProject.is_git_repo
                            ? "bg-pos-soft text-pos"
                            : "bg-warn-soft text-warn"
                        }`}
                      >
                        {activeProject.is_git_repo ? "git" : "shared"}
                      </span>
                    )}
                    <ChevronDown className="w-3.5 h-3.5 text-ink-muted" />
                  </button>

                  {projPickerOpen && (
                    <>
                      <div
                        className="fixed inset-0 z-20"
                        onClick={() => setProjPickerOpen(false)}
                      />
                      <div className="absolute left-1/2 -translate-x-1/2 mt-1.5 w-64 rounded-xl border border-line-strong bg-panel shadow-2xl p-1 z-30 text-left space-y-0.5">
                        <button
                          type="button"
                          onClick={() => {
                            setProjPickerOpen(false);
                            void selectProject(null);
                          }}
                          className={`w-full px-2.5 py-1.5 rounded-lg text-xs flex items-center justify-between gap-2 ${
                            activeProjectId === null
                              ? "bg-brand-soft text-ink font-medium"
                              : "hover:bg-hover text-ink-soft"
                          }`}
                        >
                          <span className="truncate">Quick Sessions</span>
                          <span className="font-mono text-[10px] text-ink-muted">
                            standalone
                          </span>
                        </button>
                        {projects.map((p) => (
                          <button
                            key={p.id}
                            type="button"
                            onClick={() => {
                              setProjPickerOpen(false);
                              void selectProject(p.id);
                            }}
                            className={`w-full px-2.5 py-1.5 rounded-lg text-xs flex items-center justify-between gap-2 ${
                              p.id === activeProjectId
                                ? "bg-brand-soft text-ink font-medium"
                                : "hover:bg-hover text-ink-soft"
                            }`}
                          >
                            <span className="truncate">{p.name}</span>
                            <span className="font-mono text-[10px] text-ink-muted">
                              {p.is_git_repo ? "git" : "shared"}
                            </span>
                          </button>
                        ))}
                        <button
                          type="button"
                          onClick={() => {
                            setProjPickerOpen(false);
                            setFolderModalOpen(true);
                          }}
                          className="w-full px-2.5 py-1.5 rounded-lg text-xs text-brand-ink hover:bg-hover flex items-center gap-2 font-medium border-t border-line mt-1 pt-2"
                        >
                          <FolderOpen className="w-3.5 h-3.5" />
                          <span>Open folder…</span>
                        </button>
                      </div>
                    </>
                  )}
                </div>

                {activeProject && !activeProject.is_git_repo && (
                  <button
                    type="button"
                    onClick={() => void gitInitActiveProject()}
                    className="inline-flex items-center gap-1.5 px-3 py-1.5 rounded-full bg-warn-soft hover:bg-warn/20 border border-warn/30 text-xs font-medium text-warn transition-colors"
                  >
                    <GitBranch className="w-3.5 h-3.5" />
                    <span>Initialize Git</span>
                  </button>
                )}
              </div>

              {/* Clickable Starter Prompt Chips */}
              {readyCount > 0 && (
                <div className="grid grid-cols-1 sm:grid-cols-2 gap-2 text-left pt-1">
                  {(activeProject ? PROJECT_PROMPTS : STANDALONE_PROMPTS).map((promptText) => (
                    <button
                      key={promptText}
                      type="button"
                      onClick={() =>
                        setDraftPreset({ seq: Date.now(), text: promptText })
                      }
                      className="rounded-xl border border-line bg-panel/70 hover:bg-hover hover:border-line-strong px-3.5 py-2.5 text-xs text-ink-soft hover:text-ink transition-colors"
                    >
                      {promptText}
                    </button>
                  ))}
                </div>
              )}
            </div>
          </div>

          <Composer
            key="welcome-composer"
            draftPreset={draftPreset}
            onConsumePreset={() => setDraftPreset(null)}
          />
        </div>
      ) : (
        <>
          {/* Virtualized Message Stream */}
          <div
            ref={parentRef}
            className="min-h-0 flex-1 overflow-y-auto py-3"
            onScroll={(event) => {
              const { scrollHeight, scrollTop, clientHeight } = event.currentTarget;
              followTail.current = scrollHeight - scrollTop - clientHeight < 80;
            }}
          >
            {visibleMessages.length === 0 ? (
              <div className="flex h-full flex-col items-center justify-center gap-2 px-6 text-center">
                <MessageSquare className="mb-1 h-6 w-6 text-brand-ink" />
                <h3 className="text-base font-semibold">Start a conversation</h3>
                <p className="text-xs text-ink-muted max-w-md">
                  Describe your task or mention a harness below. The session title will update automatically after your first message.
                </p>
              </div>
            ) : (
              <div
                style={{
                  height: `${virtualizer.getTotalSize()}px`,
                  width: "100%",
                  position: "relative",
                }}
              >
                {virtualizer.getVirtualItems().map((vItem) => {
                  const msg = visibleMessages[vItem.index];
                  return (
                    <div
                      key={msg.id}
                      data-index={vItem.index}
                      ref={virtualizer.measureElement}
                      style={{
                        position: "absolute",
                        top: 0,
                        left: 0,
                        width: "100%",
                        transform: `translateY(${vItem.start}px)`,
                      }}
                    >
                      <MessageRow
                        message={msg}
                        consultReplies={
                          repliesByParent.get(msg.id) ?? NO_CONSULT_REPLIES
                        }
                      />
                    </div>
                  );
                })}
              </div>
            )}
          </div>

          {hopPaused && (
            <div className="mx-auto mb-2 w-full max-w-[52rem] px-4 sm:px-6">
              <div className="rounded-xl border border-warn/40 bg-warn-soft px-3.5 py-2.5 flex items-center justify-between text-xs text-warn">
                <span>
                  Switch paused at hop {hopPaused.hops_used}/{hopPaused.max_hops} (pending:{" "}
                  {hopPaused.pending_targets.map((t) => `@${t}`).join(", ")})
                </span>
                <div className="flex gap-2">
                  <button
                    type="button"
                    onClick={() => void continueHopChain(hopPaused.chain_id)}
                    className="px-3 py-1 rounded-lg bg-amber-500 text-zinc-950 font-semibold"
                  >
                    Continue (+6 hops)
                  </button>
                  <button
                    type="button"
                    onClick={() => void stopAll()}
                    className="px-3 py-1 rounded-lg bg-elevated border border-line text-ink"
                  >
                    Stop
                  </button>
                </div>
              </div>
            </div>
          )}

          {startingMembers.length > 0 && (
            <div
              role="status"
              className="mx-auto flex w-full max-w-[52rem] items-center gap-2 px-5 py-1.5 text-xs text-ink-muted"
            >
              <Loader2 className="h-3.5 w-3.5 animate-spin text-brand-ink" />
              Starting {startingMembers.map(([handle]) => `@${handle}`).join(", ")}…
            </div>
          )}

          <Composer key={activeSession.id} />

          {shellOpen && (
            <Suspense fallback={<div role="status" className="border-t border-line p-3 text-xs text-ink-muted">Loading terminal…</div>}>
              <WebShellDrawer
                key={activeSession.id}
                sessionId={activeSession.id}
                shared={activeSession.isolation_mode === "shared_cwd"}
                defaultCwd={
                  activeSession.integration_wt_path ??
                  activeProject?.root_path ??
                  "."
                }
              />
            </Suspense>
          )}
        </>
      )}
    </main>
  );
}
