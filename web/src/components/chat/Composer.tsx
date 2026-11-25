import { useMemo, useState } from "react";
import { Send, Square } from "lucide-react";
import { api } from "../../api/client";
import { useAppStore } from "../../store/useAppStore";

export function Composer() {
  const { activeSession, members, memberStatuses, sendMessage, stopAll, selectSession } =
    useAppStore();
  const [text, setText] = useState("");
  const [sending, setSending] = useState(false);

  const anyActive = Object.values(memberStatuses).some(
    (s) =>
      s.status === "thinking" ||
      s.status === "running_tool" ||
      s.status === "spawning" ||
      s.status === "consulting"
  );

  // Detect active `@` prefix at end of draft for autocomplete
  const mentionQuery = useMemo(() => {
    const m = text.match(/(?:^|\s)@([a-zA-Z0-9_-]*)$/);
    return m ? m[1].toLowerCase() : null;
  }, [text]);

  const candidates = useMemo(() => {
    if (mentionQuery === null) return [];
    const base = [
      ...members.map((m) => ({
        handle: m.handle,
        desc: `${m.display_name} (${m.adapter_id})`,
      })),
      { handle: "orchestrator", desc: "Session Orchestrator" },
      { handle: "all", desc: "Broadcast to all executors" },
    ];
    return base.filter((c) => c.handle.toLowerCase().startsWith(mentionQuery));
  }, [mentionQuery, members]);

  const routingPreview = useMemo(() => {
    if (!activeSession) return "";
    const explicit = Array.from(
      text.replace(/```[\s\S]*?```|`[^`]+`/g, " ").matchAll(/(?:^|\s)@([a-zA-Z0-9_-]+)/g)
    ).map((m) => m[1]);
    if (explicit.length > 0) {
      return `Direct -> ${explicit.map((h) => `@${h}`).join(", ")}`;
    }
    if (activeSession.orchestrator_member) {
      return `Via Orchestrator (@${activeSession.orchestrator_member}) -> auto-selects executor, model & effort`;
    }
    return `Default -> @${members[0]?.handle ?? "member"}`;
  }, [text, activeSession, members]);

  const insertMention = (handle: string) => {
    setText((prev) =>
      prev.replace(/(^|\s)@[a-zA-Z0-9_-]*$/, `$1@${handle} `)
    );
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
    <div className="border-t border-zinc-800 bg-zinc-900/50 p-3 relative">
      {candidates.length > 0 && (
        <div className="absolute bottom-full left-3 mb-2 w-72 rounded-xl border border-zinc-800 bg-zinc-900 shadow-2xl overflow-hidden z-20">
          {candidates.map((c) => (
            <button
              key={c.handle}
              type="button"
              onClick={() => insertMention(c.handle)}
              className="w-full px-3 py-2 text-left text-xs hover:bg-zinc-800 flex items-center justify-between"
            >
              <span className="font-mono text-indigo-300">@{c.handle}</span>
              <span className="text-[11px] text-zinc-500 truncate max-w-[140px]">
                {c.desc}
              </span>
            </button>
          ))}
        </div>
      )}

      <div className="rounded-xl border border-zinc-800 bg-zinc-950 focus-within:border-indigo-500 transition-colors p-2.5 space-y-2">
        <textarea
          rows={2}
          value={text}
          onChange={(e) => setText(e.target.value)}
          onKeyDown={(e) => {
            if ((e.metaKey || e.ctrlKey) && e.key === "Enter") {
              e.preventDefault();
              void handleSend();
            }
          }}
          placeholder="Message group chat, type @codex / @claude, or send unmentioned to let Orchestrator route..."
          className="w-full bg-transparent text-sm text-zinc-100 placeholder-zinc-500 resize-none focus:outline-none"
        />

        <div className="flex flex-wrap items-center justify-between gap-2 pt-1 border-t border-zinc-900">
          <div className="flex items-center gap-3 text-[11px] text-zinc-400">
            <span>{routingPreview}</span>
            <label className="inline-flex items-center gap-1.5 cursor-pointer text-zinc-400 hover:text-zinc-200">
              <input
                type="checkbox"
                checked={activeSession.confirm_before_dispatch}
                onChange={() => void toggleConfirmFirst()}
                className="rounded border-zinc-700 bg-zinc-900"
              />
              <span>Confirm Plan First</span>
            </label>
          </div>

          <div className="flex items-center gap-2">
            {anyActive && (
              <button
                type="button"
                onClick={() => void stopAll()}
                className="inline-flex items-center gap-1 px-2.5 py-1.5 rounded-lg bg-rose-600 hover:bg-rose-500 text-xs font-medium text-white"
              >
                <Square className="w-3 h-3 fill-current" />
                <span>Stop All</span>
              </button>
            )}
            <button
              type="button"
              disabled={!text.trim() || sending}
              onClick={() => void handleSend()}
              className="inline-flex items-center gap-1.5 px-3 py-1.5 rounded-lg bg-indigo-600 hover:bg-indigo-500 disabled:opacity-40 text-xs font-medium text-white"
            >
              <Send className="w-3.5 h-3.5" />
              <span>Send</span>
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}
