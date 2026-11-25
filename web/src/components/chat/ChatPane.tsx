import { useEffect, useMemo, useRef } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { Brain, Crown, History, Settings2 } from "lucide-react";
import { api } from "../../api/client";
import { useAppStore } from "../../store/useAppStore";
import type { ChatMessage } from "../../types/api";
import { SessionChangesBar } from "../gitbar/SessionChangesBar";
import { Composer } from "./Composer";
import { MessageRow } from "./MessageRow";

export function ChatPane() {
  const {
    activeSession,
    messages,
    hopPaused,
    pendingPermissions,
    continueHopChain,
    stopAll,
    openDrawerTab,
  } = useAppStore();

  const parentRef = useRef<HTMLDivElement | null>(null);

  // Group consult replies under their parent consult question message
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
    getScrollElement: () => parentRef.current,
    estimateSize: () => 110,
    overscan: 8,
  });

  useEffect(() => {
    if (visibleMessages.length > 0) {
      virtualizer.scrollToIndex(visibleMessages.length - 1, { align: "end" });
    }
  }, [visibleMessages.length, virtualizer]);

  // Keyboard shortcuts Alt+A / Alt+R for top pending permission card
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!activeSession || pendingPermissions.length === 0) return;
      const top = pendingPermissions[0];
      if (e.altKey && e.key.toLowerCase() === "a") {
        e.preventDefault();
        void api.resolvePermission(activeSession.id, top.request_id, "allow_once");
      } else if (e.altKey && e.key.toLowerCase() === "r") {
        e.preventDefault();
        void api.resolvePermission(activeSession.id, top.request_id, "reject_once");
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [activeSession, pendingPermissions]);

  if (!activeSession) {
    return (
      <main className="flex-1 flex flex-col items-center justify-center p-8 text-center bg-zinc-950">
        <h2 className="text-lg font-semibold text-zinc-200">
          Welcome to Meta-Harness
        </h2>
        <p className="text-sm text-zinc-500 max-w-md mt-2">
          Open a project folder in the left sidebar and start a group session to collaborate with @codex, @claude, and @antigravity over ACP.
        </p>
      </main>
    );
  }

  return (
    <main className="flex-1 flex flex-col min-w-0 bg-zinc-950 h-full">
      {/* Session Header */}
      <header className="px-4 py-2.5 border-b border-zinc-800 flex items-center justify-between gap-3">
        <div className="flex items-center gap-3 min-w-0">
          <h2 className="text-sm font-semibold text-zinc-100 truncate">
            {activeSession.title}
          </h2>
          {activeSession.orchestrator_member && (
            <span className="inline-flex items-center gap-1 px-2 py-0.5 rounded-full bg-amber-500/10 border border-amber-500/30 text-[11px] text-amber-300">
              <Crown className="w-3 h-3" />
              <span>Orchestrator: @{activeSession.orchestrator_member}</span>
            </span>
          )}
        </div>

        <div className="flex items-center gap-1.5">
          <button
            type="button"
            onClick={() => openDrawerTab("trajectory")}
            className="inline-flex items-center gap-1 px-2.5 py-1 rounded-md bg-zinc-900 hover:bg-zinc-800 border border-zinc-800 text-xs text-zinc-300"
          >
            <History className="w-3.5 h-3.5" />
            <span>Trajectories</span>
          </button>
          <button
            type="button"
            onClick={() => openDrawerTab("memory")}
            className="inline-flex items-center gap-1 px-2.5 py-1 rounded-md bg-zinc-900 hover:bg-zinc-800 border border-zinc-800 text-xs text-zinc-300"
          >
            <Brain className="w-3.5 h-3.5" />
            <span>Memory</span>
          </button>
          <button
            type="button"
            onClick={() => openDrawerTab("settings")}
            className="inline-flex items-center gap-1 px-2.5 py-1 rounded-md bg-zinc-900 hover:bg-zinc-800 border border-zinc-800 text-xs text-zinc-300"
          >
            <Settings2 className="w-3.5 h-3.5" />
            <span>Switch</span>
          </button>
        </div>
      </header>

      <SessionChangesBar />

      {/* Virtualized Message Stream */}
      <div ref={parentRef} className="flex-1 overflow-y-auto py-3">
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
                  consultReplies={repliesByParent.get(msg.id) ?? []}
                />
              </div>
            );
          })}
        </div>

        {visibleMessages.length === 0 && (
          <div className="h-full flex items-center justify-center text-xs text-zinc-500">
            Type a task below — mention @codex / @claude directly, or let your Orchestrator choose!
          </div>
        )}
      </div>

      {/* Hop-Limit Paused Banner */}
      {hopPaused && (
        <div className="mx-4 mb-2 rounded-lg border border-amber-500/40 bg-amber-500/10 px-3 py-2 flex items-center justify-between text-xs text-amber-200">
          <span>
            Switch paused at hop {hopPaused.hops_used}/{hopPaused.max_hops} (pending:{" "}
            {hopPaused.pending_targets.map((t) => `@${t}`).join(", ")})
          </span>
          <div className="flex gap-2">
            <button
              type="button"
              onClick={() => void continueHopChain(hopPaused.chain_id)}
              className="px-2.5 py-1 rounded bg-amber-500 text-zinc-950 font-medium"
            >
              Continue (+6 hops)
            </button>
            <button
              type="button"
              onClick={() => void stopAll()}
              className="px-2.5 py-1 rounded bg-zinc-800 text-zinc-200"
            >
              Stop
            </button>
          </div>
        </div>
      )}

      <Composer />
    </main>
  );
}
