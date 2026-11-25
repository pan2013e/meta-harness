import { useEffect, useMemo, useRef, useState } from "react";
import { Check, CornerDownLeft, Send, Square } from "lucide-react";
import { api } from "../../api/client";
import { useAppStore } from "../../store/useAppStore";
import { HarnessAvatar } from "../common/Badges";

export function Composer() {
  const {
    activeSession,
    members,
    adapters,
    quotas,
    memberStatuses,
    sendMessage,
    stopAll,
    selectSession,
  } = useAppStore();

  const [text, setText] = useState("");
  const [sending, setSending] = useState(false);
  const [selectedIdx, setSelectedIdx] = useState(0);
  const [menuDismissed, setMenuDismissed] = useState(false);
  const textareaRef = useRef<HTMLTextAreaElement | null>(null);

  const anyActive = Object.values(memberStatuses).some(
    (s) =>
      s.status === "thinking" ||
      s.status === "running_tool" ||
      s.status === "spawning" ||
      s.status === "consulting"
  );

  const mentionQuery = useMemo(() => {
    const m = text.match(/(?:^|\s)@([a-zA-Z0-9_-]*)$/);
    return m ? m[1].toLowerCase() : null;
  }, [text]);

  useEffect(() => {
    setMenuDismissed(false);
    setSelectedIdx(0);
  }, [mentionQuery]);

  // Auto-resize textarea height cleanly
  useEffect(() => {
    const el = textareaRef.current;
    if (!el) return;
    el.style.height = "auto";
    el.style.height = `${Math.min(Math.max(el.scrollHeight, 48), 180)}px`;
  }, [text]);

  const candidates = useMemo(() => {
    if (mentionQuery === null || menuDismissed) return [];
    const base = [
      ...members.map((m) => {
        const q = quotas[m.adapter_id];
        const minPct =
          q?.window_5h_remaining_pct !== null && q?.weekly_remaining_pct !== null
            ? Math.min(q.window_5h_remaining_pct, q.weekly_remaining_pct)
            : (q?.window_5h_remaining_pct ?? q?.weekly_remaining_pct ?? null);
        return {
          handle: m.handle,
          desc: m.display_name,
          accent: adapters[m.adapter_id]?.accent_color ?? "#6366f1",
          pctLabel:
            q?.rate_limited_until !== null && q?.rate_limited_until !== undefined
              ? "429"
              : minPct !== null
                ? `${Math.round(minPct)}%`
                : null,
        };
      }),
      {
        handle: "orchestrator",
        desc: "Session Orchestrator",
        accent: "#f59e0b",
        pctLabel: null,
      },
      {
        handle: "all",
        desc: "Broadcast to all executors",
        accent: "#6366f1",
        pctLabel: null,
      },
    ];
    return base.filter((c) => c.handle.toLowerCase().startsWith(mentionQuery));
  }, [mentionQuery, menuDismissed, members, adapters, quotas]);

  const routingPreview = useMemo(() => {
    if (!activeSession) return "";
    const explicit = Array.from(
      text.replace(/```[\s\S]*?```|`[^`]+`/g, " ").matchAll(/(?:^|\s)@([a-zA-Z0-9_-]+)/g)
    ).map((m) => m[1]);
    if (explicit.length > 0) {
      return `Direct -> ${explicit.map((h) => `@${h}`).join(", ")}`;
    }
    if (activeSession.orchestrator_member) {
      return `Auto -> Orchestrator (@${activeSession.orchestrator_member})`;
    }
    return `Default -> @${members[0]?.handle ?? "member"}`;
  }, [text, activeSession, members]);

  const insertMention = (handle: string) => {
    setText((prev) =>
      prev.replace(/(^|\s)@[a-zA-Z0-9_-]*$/, `$1@${handle} `)
    );
    setMenuDismissed(true);
    textareaRef.current?.focus();
  };

  const handleSend = async () => {
    if (!text.trim() || sending || !activeSession) return;
    const body = text.trim();
    setText("");
    setSending(true);
    try {
      await sendMessage(body);
    } finally {
      setSending(false);
    }
  };

  const toggleConfirmFirst = async () => {
    if (!activeSession) return;
    await api.patchSession(activeSession.id, {
      confirm_before_dispatch: !activeSession.confirm_before_dispatch,
    });
    await selectSession(activeSession.id);
  };

  if (!activeSession) return null;

  return (
    <div className="border-t border-line bg-panel/80 px-4 py-3 relative">
      <div className="max-w-4xl mx-auto relative">
        {candidates.length > 0 && (
          <div className="absolute bottom-full left-0 mb-2 w-80 rounded-xl border border-line-strong bg-panel shadow-2xl overflow-hidden z-30 p-1">
            <div className="px-2.5 py-1 text-[10px] uppercase tracking-wider font-semibold text-ink-muted">
              Route target (Tab / Enter)
            </div>
            {candidates.map((c, idx) => (
              <button
                key={c.handle}
                type="button"
                onClick={() => insertMention(c.handle)}
                className={`w-full px-2.5 py-1.5 rounded-lg text-left text-xs flex items-center justify-between gap-2 transition-colors ${
                  idx === selectedIdx
                    ? "bg-brand-soft text-ink"
                    : "hover:bg-hover text-ink-soft"
                }`}
              >
                <span className="flex items-center gap-2 min-w-0">
                  <HarnessAvatar handle={c.handle} accentColor={c.accent} size="sm" />
                  <span className="font-mono font-semibold text-brand-ink">
                    @{c.handle}
                  </span>
                </span>
                <span className="flex items-center gap-1.5 text-[11px] text-ink-muted shrink-0">
                  <span className="truncate max-w-[100px]">{c.desc}</span>
                  {c.pctLabel && (
                    <span className="px-1.5 py-0.2 rounded bg-inset border border-line font-mono text-[10px]">
                      {c.pctLabel}
                    </span>
                  )}
                </span>
              </button>
            ))}
          </div>
        )}

        <div className="rounded-2xl border border-line-strong bg-inset focus-within:border-brand transition-colors p-3 space-y-2 shadow-sm">
          <textarea
            ref={textareaRef}
            rows={2}
            value={text}
            onChange={(e) => setText(e.target.value)}
            onKeyDown={(e) => {
              if (candidates.length > 0) {
                if (e.key === "ArrowDown") {
                  e.preventDefault();
                  setSelectedIdx((i) => (i + 1) % candidates.length);
                  return;
                }
                if (e.key === "ArrowUp") {
                  e.preventDefault();
                  setSelectedIdx((i) => (i - 1 + candidates.length) % candidates.length);
                  return;
                }
                if (e.key === "Tab" || e.key === "Enter") {
                  e.preventDefault();
                  const chosen = candidates[selectedIdx] ?? candidates[0];
                  if (chosen) insertMention(chosen.handle);
                  return;
                }
                if (e.key === "Escape") {
                  e.preventDefault();
                  setMenuDismissed(true);
                  return;
                }
              }

              if ((e.metaKey || e.ctrlKey) && e.key === "Enter") {
                e.preventDefault();
                void handleSend();
              }
            }}
            placeholder="Message group chat, type @codex / @claude, or send directly to Orchestrator..."
            className="w-full bg-transparent text-[13.5px] text-ink placeholder-ink-muted resize-none focus:outline-none leading-relaxed"
          />

          <div className="flex flex-wrap items-center justify-between gap-2 pt-2 border-t border-line">
            <div className="flex flex-wrap items-center gap-2 text-[11px]">
              <span className="px-2 py-0.5 rounded-md bg-elevated border border-line font-mono text-ink-soft">
                {routingPreview}
              </span>
              <button
                type="button"
                onClick={() => void toggleConfirmFirst()}
                className={`inline-flex items-center gap-1.5 px-2.5 py-0.5 rounded-md border transition-colors ${
                  activeSession.confirm_before_dispatch
                    ? "bg-warn-soft border-warn/40 text-warn font-medium"
                    : "bg-elevated border-line text-ink-muted hover:text-ink"
                }`}
              >
                {activeSession.confirm_before_dispatch && (
                  <Check className="w-3 h-3" />
                )}
                <span>Confirm Plan First</span>
              </button>
            </div>

            <div className="flex items-center gap-2">
              {anyActive && (
                <button
                  type="button"
                  onClick={() => void stopAll()}
                  className="inline-flex items-center gap-1.5 px-3 py-1.5 rounded-xl bg-rose-600 hover:bg-rose-500 text-xs font-medium text-white transition-colors"
                >
                  <Square className="w-3 h-3 fill-current" />
                  <span>Stop All</span>
                </button>
              )}
              <button
                type="button"
                disabled={!text.trim() || sending}
                onClick={() => void handleSend()}
                className="inline-flex items-center gap-1.5 px-3.5 py-1.5 rounded-xl bg-brand hover:bg-brand-hover disabled:opacity-40 text-xs font-medium text-white transition-colors"
              >
                <Send className="w-3.5 h-3.5" />
                <span>Send</span>
                <kbd className="hidden sm:inline-flex items-center gap-0.5 text-[10px] opacity-80 font-mono">
                  <CornerDownLeft className="w-2.5 h-2.5" />
                </kbd>
              </button>
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}
