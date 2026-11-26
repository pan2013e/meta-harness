import { useEffect, useMemo, useRef } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import {
  Brain,
  Crown,
  FolderGit2,
  FolderOpen,
  FolderTree,
  History,
  Layers,
  Loader2,
  MessageSquare,
  MessageSquarePlus,
  PanelLeftOpen,
  RefreshCw,
  Settings2,
} from "lucide-react";
import { api } from "../../api/client";
import { useAppStore } from "../../store/useAppStore";
import type { ChatMessage } from "../../types/api";
import {
  HarnessAvatar,
  QuotaSnapshotSummary,
} from "../common/Badges";
import { SessionChangesBar } from "../gitbar/SessionChangesBar";
import { Composer } from "./Composer";
import { MessageRow } from "./MessageRow";

const NO_CONSULT_REPLIES: ChatMessage[] = [];

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
    drawerOpen,
    drawerTab,
    continueHopChain,
    stopAll,
    openDrawerTab,
    openProjectWithNativeOrModal,
    setFolderModalOpen,
    pickingFolder,
    selectProject,
    createAndSelectSession,
    probes,
    quotas,
    quotasRefreshing,
    refreshQuotasLive,
    setSettingsOpen,
    memberStatuses,
  } = useAppStore();

  const parentRef = useRef<HTMLDivElement | null>(null);
  const followTail = useRef(true);
  const activeProject = projects.find((p) => p.id === activeProjectId) ?? null;
  const activeOrchestrator = useAppStore((state) => state.members.find((member) =>
    member.handle === state.activeSession?.orchestrator_member));
  const orchestratorAvailable = probes.some((probe) => probe.installed && probe.adapter_id === activeOrchestrator?.adapter_id);
  const startingMembers = Object.entries(memberStatuses).filter(([, member]) => member.status === "spawning");

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
      if (!activeSession || pendingPermissions.length === 0) return;
      const top = pendingPermissions[0];
      const key = e.key.toLowerCase();
      if (!e.altKey || (key !== "a" && key !== "r")) return;
      const kind = key === "a" ? "allow_once" : "reject_once";
      const option = top.options.find((item) => item.kind === kind);
      if (!option) return;
      e.preventDefault();
      void api.resolvePermission(activeSession.id, top.request_id, option.option_id)
        .catch((error: unknown) => useAppStore.getState().setError(
          error instanceof Error ? error.message : String(error),
        ));
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [activeSession, pendingPermissions]);

  return (
    <main className="flex-1 flex flex-col min-w-0 bg-app h-full">
      {/* Top Workspace Header */}
      <header className="flex min-h-14 items-center justify-between gap-2 border-b border-line bg-panel/70 px-3 sm:px-5">
        <div className="flex items-center gap-2.5 min-w-0">
          <div className={sidebarCollapsed ? "" : "lg:hidden"}>
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
          {activeSession ? (
            <div className="flex items-center gap-2.5 min-w-0">
              <h2 className="truncate text-[15px] font-semibold tracking-tight text-ink">
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
              Workspace Overview
            </div>
          )}
        </div>

        <div className="flex shrink-0 items-center gap-1">
          {(["trajectory", "memory"] as const).map((tab) => {
            const isActive = drawerOpen && drawerTab === tab;
            const label = tab === "trajectory" ? "Trajectory" : "Memory";
            const Icon = tab === "trajectory" ? History : Brain;
            return (
              <button
                key={tab}
                type="button"
                aria-label={label}
                onClick={() => openDrawerTab(tab)}
                className={`inline-flex items-center gap-1.5 rounded-lg px-2.5 py-2 text-xs transition-colors ${
                  isActive
                    ? "bg-brand-soft text-brand-ink font-medium"
                    : "text-ink-soft hover:bg-hover hover:text-ink"
                }`}
              >
                <Icon className="w-3.5 h-3.5" />
                <span className="hidden md:inline">{label}</span>
              </button>
            );
          })}
          <button type="button" aria-label="Settings" title="Settings" onClick={() => setSettingsOpen(true)}
            className="ml-1 rounded-lg p-2 text-ink-soft hover:bg-hover hover:text-ink">
            <Settings2 className="h-4 w-4" />
          </button>
        </div>
      </header>

      {/* Welcome Dashboard when no active session */}
      {!activeSession ? (
        <div className="flex-1 overflow-y-auto p-6 md:p-10">
          <div className="max-w-3xl mx-auto space-y-8">
            {/* Hero CTA */}
            <div className="rounded-2xl border border-line bg-panel p-6 md:p-8 space-y-4 shadow-sm">
              <div className="inline-flex items-center gap-2 px-2.5 py-1 rounded-full bg-brand-soft border border-brand/30 text-[11px] font-medium text-brand-ink">
                <Layers className="w-3.5 h-3.5" />
                <span>ACP Group Chat & Git Worktree Switch</span>
              </div>
              <h2 className="text-2xl font-bold tracking-tight text-ink">
                Collaborate with @codex, @claude, and @antigravity side by side
              </h2>
              <p className="text-sm text-ink-soft max-w-2xl leading-relaxed">
                Open a repository folder to launch isolated per-agent Git worktrees,
                parallel orchestration, synchronous peer consults, and shared episodic memory.
              </p>

              <div className="flex flex-wrap items-center gap-3 pt-1">
                <button
                  type="button"
                  disabled={pickingFolder}
                  onClick={() => void openProjectWithNativeOrModal()}
                  className="inline-flex items-center gap-2 px-4 py-2.5 rounded-xl bg-brand hover:bg-brand-hover disabled:opacity-50 text-xs font-semibold text-white shadow-sm transition-colors"
                >
                  {pickingFolder ? (
                    <Loader2 className="w-4 h-4 animate-spin" />
                  ) : (
                    <FolderOpen className="w-4 h-4" />
                  )}
                  <span>
                    {pickingFolder ? "Opening System Picker..." : "Open Project Folder"}
                  </span>
                </button>

                <button
                  type="button"
                  onClick={() => setFolderModalOpen(true)}
                  className="inline-flex items-center gap-2 px-4 py-2.5 rounded-xl bg-elevated hover:bg-hover border border-line text-xs font-medium text-ink transition-colors"
                >
                  <FolderTree className="w-4 h-4 text-brand-ink" />
                  <span>Browse Directory…</span>
                </button>

                {activeProject && (
                  <button
                    type="button"
                    onClick={() => void createAndSelectSession("Main Group Chat")}
                    className="inline-flex items-center gap-2 px-4 py-2.5 rounded-xl bg-pos-soft hover:bg-pos/20 border border-pos/30 text-xs font-semibold text-pos transition-colors"
                  >
                    <MessageSquarePlus className="w-4 h-4" />
                    <span>New Session in {activeProject.name}</span>
                  </button>
                )}
              </div>
            </div>

            {/* Recent Projects */}
            {projects.length > 0 && (
              <div className="space-y-3">
                <h3 className="text-xs uppercase tracking-wider font-semibold text-ink-muted">
                  Recent Projects
                </h3>
                <div className="grid grid-cols-1 sm:grid-cols-2 gap-3">
                  {projects.map((p) => (
                    <button
                      key={p.id}
                      type="button"
                      onClick={() => void selectProject(p.id)}
                      className={`text-left rounded-xl border p-3.5 transition-colors space-y-1 ${
                        p.id === activeProjectId
                          ? "bg-brand-soft/60 border-brand/40"
                          : "bg-panel hover:bg-hover border-line"
                      }`}
                    >
                      <div className="flex items-center justify-between gap-2">
                        <span className="flex items-center gap-2 text-xs font-semibold text-ink truncate">
                          <FolderGit2 className="w-4 h-4 text-brand-ink shrink-0" />
                          <span className="truncate">{p.name}</span>
                        </span>
                        <span
                          className={`text-[10px] font-mono px-1.5 py-0.5 rounded ${
                            p.is_git_repo
                              ? "bg-pos-soft text-pos"
                              : "bg-warn-soft text-warn"
                          }`}
                        >
                          {p.is_git_repo ? "git" : "shared"}
                        </span>
                      </div>
                      <div className="text-[11px] font-mono text-ink-muted truncate">
                        {p.root_path}
                      </div>
                    </button>
                  ))}
                </div>
              </div>
            )}

            {/* Live Harness Status & Quotas */}
            <div className="space-y-3">
              <div className="flex items-center justify-between">
                <h3 className="text-xs uppercase tracking-wider font-semibold text-ink-muted">
                  Connected Harnesses & Live Subscription Quotas
                </h3>
                <button
                  type="button"
                  disabled={quotasRefreshing}
                  onClick={() => void refreshQuotasLive()}
                  className="inline-flex items-center gap-1.5 text-xs text-brand-ink hover:underline"
                >
                  <RefreshCw
                    className={`w-3.5 h-3.5 ${quotasRefreshing ? "animate-spin" : ""}`}
                  />
                  <span>Refresh Live APIs</span>
                </button>
              </div>

              <div className="grid grid-cols-1 sm:grid-cols-3 gap-3">
                {probes.map((p) => {
                  const q = quotas[p.adapter_id];
                  return (
                    <div
                      key={p.adapter_id}
                      className="rounded-xl border border-line bg-panel p-3.5 space-y-2.5"
                    >
                      <div className="flex items-center justify-between gap-2">
                        <div className="flex items-center gap-2 min-w-0">
                          <HarnessAvatar
                            handle={p.adapter_id}
                            accentColor={p.accent_color}
                            size="sm"
                          />
                          <div className="min-w-0">
                            <div className="text-xs font-semibold text-ink truncate">
                              {p.display_name}
                            </div>
                            <div className="text-[10px] text-ink-muted">
                              {q?.plan_label ?? (p.installed ? "Detected" : "Not installed")}
                            </div>
                          </div>
                        </div>
                        <span
                          className={`text-[10px] px-1.5 py-0.5 rounded font-mono shrink-0 ${
                            p.installed
                              ? "bg-pos-soft text-pos"
                              : "bg-elevated text-ink-muted"
                          }`}
                        >
                          {p.installed ? "Ready" : "Missing"}
                        </span>
                      </div>

                      {q && p.installed && <QuotaSnapshotSummary quota={q} />}
                    </div>
                  );
                })}
              </div>
            </div>
          </div>
        </div>
      ) : (
        <>
          <SessionChangesBar key={activeSession.id} />

          {/* Virtualized Developer Thread Stream */}
          <div ref={parentRef} className="min-h-0 flex-1 overflow-y-auto py-3"
            onScroll={(event) => {
              const { scrollHeight, scrollTop, clientHeight } = event.currentTarget;
              followTail.current = scrollHeight - scrollTop - clientHeight < 80;
            }}>
            {visibleMessages.length === 0 && (
              <div className="flex h-full flex-col items-center justify-center gap-2 px-6 text-center">
                <MessageSquare className="mb-1 h-6 w-6 text-brand-ink" />
                <h3 className="text-base font-semibold">Start a conversation</h3>
                <p className="text-sm text-ink-muted">Send a message to the group or use @ to choose a harness.</p>
              </div>
            )}
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
                      consultReplies={repliesByParent.get(msg.id) ?? NO_CONSULT_REPLIES}
                    />
                  </div>
                );
              })}
            </div>

            {visibleMessages.length === 0 && (
              <div className="h-full flex flex-col items-center justify-center p-8 text-center space-y-2">
                <p className="text-sm font-medium text-ink">Session ready</p>
                <p className="text-xs text-ink-muted max-w-md">
                  Mention <span className="font-mono text-brand-ink">@codex</span> or{" "}
                  <span className="font-mono text-brand-ink">@claude</span> directly, or
                  describe a task for your Orchestrator to delegate cleanly.
                </p>
              </div>
            )}
          </div>

          {hopPaused && (
            <div className="mx-5 mb-2 rounded-xl border border-warn/40 bg-warn-soft px-3.5 py-2.5 flex items-center justify-between text-xs text-warn">
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
          )}

          {startingMembers.length > 0 && (
            <div role="status" className="mx-auto flex w-full max-w-[50rem] items-center gap-2 px-5 py-1.5 text-xs text-ink-muted">
              <Loader2 className="h-3.5 w-3.5 animate-spin text-brand-ink" />
              Starting {startingMembers.map(([handle]) => `@${handle}`).join(", ")}…
            </div>
          )}
          <Composer key={activeSession.id} />
        </>
      )}
    </main>
  );
}
