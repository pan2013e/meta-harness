import { memo, useState } from "react";
import {
  Activity,
  ChevronDown,
  ChevronRight,
  Crown,
  GitCommitHorizontal,
  SearchCode,
  ShieldAlert,
  Sparkles,
} from "lucide-react";
import { api } from "../../api/client";
import { useAppStore } from "../../store/useAppStore";
import type { ChatMessage, DispatchTaskSpec, PendingPermissionView } from "../../types/api";
import { MarkdownBlock } from "../common/MarkdownBlock";

interface Props {
  message: ChatMessage;
  consultReplies: ChatMessage[];
}

export const MessageRow = memo(function MessageRow({ message, consultReplies }: Props) {
  const { inspectTurn, turnCounters, adapters, members } = useAppStore();
  const [consultExpanded, setConsultExpanded] = useState(false);

  const member = members.find((m) => m.handle === message.sender_id);
  const accent = member
    ? (adapters[member.adapter_id]?.accent_color ?? "#6366f1")
    : "#6366f1";

  if (message.kind === "activity_pill") {
    return (
      <div className="flex justify-center py-1">
        <button
          type="button"
          onClick={() => {
            if (message.turn_id) void inspectTurn(message.turn_id);
          }}
          className="inline-flex items-center gap-1.5 px-3 py-1 rounded-full bg-zinc-900 border border-zinc-800 text-[11px] text-zinc-400 hover:text-zinc-200 hover:border-zinc-700 transition-colors"
        >
          <Activity className="w-3 h-3 text-indigo-400" />
          <span>{message.content}</span>
        </button>
      </div>
    );
  }

  if (message.kind === "merge_notice" || message.kind === "system_notice") {
    return (
      <div className="flex justify-center py-1">
        <div className="inline-flex items-center gap-1.5 px-3 py-1 rounded-md bg-emerald-500/10 border border-emerald-500/20 text-[11px] text-emerald-300">
          <GitCommitHorizontal className="w-3.5 h-3.5" />
          <span>{message.content}</span>
        </div>
      </div>
    );
  }

  if (message.kind === "dispatch_card") {
    const meta = message.metadata;
    const status = String(meta.status ?? "dispatched");
    const dispatchId = String(meta.dispatch_id ?? "");
    const dispatches = Array.isArray(meta.dispatches)
      ? (meta.dispatches as DispatchTaskSpec[])
      : [];

    return (
      <div className="mx-2 my-1 rounded-xl border border-amber-500/30 bg-amber-500/5 p-3 space-y-2 text-xs">
        <div className="flex items-center justify-between">
          <div className="flex items-center gap-2 font-medium text-amber-200">
            <Crown className="w-3.5 h-3.5 text-amber-400" />
            <span>Orchestrator Dispatch (@{message.sender_id})</span>
          </div>
          <span className="px-2 py-0.5 rounded-full text-[10px] uppercase bg-zinc-900 text-zinc-300 border border-zinc-700">
            {status.replace("_", " ")}
          </span>
        </div>
        <p className="text-zinc-300">{message.content}</p>
        <div className="flex flex-wrap gap-1.5">
          {dispatches.map((d, idx) => (
            <span
              key={idx}
              className="px-2 py-0.5 rounded bg-zinc-900 border border-zinc-800 font-mono text-[11px] text-indigo-300"
            >
              @{d.target} [{d.model ?? "auto"}:{d.effort ?? "auto"}]
            </span>
          ))}
        </div>
        {status === "awaiting_confirmation" && dispatchId && (
          <div className="flex gap-2 pt-1">
            <button
              type="button"
              onClick={() => void api.confirmDispatch(message.session_id, dispatchId)}
              className="px-3 py-1 rounded bg-amber-500 hover:bg-amber-400 text-zinc-950 font-medium"
            >
              Confirm & Run
            </button>
            <button
              type="button"
              onClick={() => void api.cancelDispatch(message.session_id, dispatchId)}
              className="px-3 py-1 rounded bg-zinc-800 hover:bg-zinc-700 text-zinc-300"
            >
              Cancel
            </button>
          </div>
        )}
      </div>
    );
  }

  if (message.kind === "permission_card") {
    const prompt = message.metadata as unknown as PendingPermissionView;
    return <PermissionInlineCard sessionId={message.session_id} prompt={prompt} />;
  }

  if (message.kind === "consult_thread") {
    if (message.parent_msg_id) {
      return null; // Rendered nested inside parent consult question
    }
    const target = String(message.metadata.target ?? "peer");
    return (
      <div className="mx-2 my-1 rounded-lg border border-violet-500/30 bg-violet-500/5 p-2.5 text-xs space-y-1.5">
        <button
          type="button"
          onClick={() => setConsultExpanded((v) => !v)}
          className="w-full flex items-center justify-between text-left text-violet-200 font-medium"
        >
          <span className="flex items-center gap-1.5">
            {consultExpanded ? (
              <ChevronDown className="w-3.5 h-3.5" />
            ) : (
              <ChevronRight className="w-3.5 h-3.5" />
            )}
            <span>
              Sync Consult: @{message.sender_id} → @{target}
            </span>
          </span>
          <span className="text-[10px] text-violet-300/70">
            {consultReplies.length > 0 ? "Answered" : "Waiting..."}
          </span>
        </button>
        <div className="text-zinc-300 pl-5">{message.content}</div>
        {consultExpanded &&
          consultReplies.map((r) => (
            <div
              key={r.id}
              className="mt-2 ml-5 pl-3 border-l-2 border-violet-500/40 space-y-1"
            >
              <div className="text-[11px] font-medium text-violet-300">
                @{r.sender_id} answered:
              </div>
              <MarkdownBlock content={r.content} />
            </div>
          ))}
      </div>
    );
  }

  // Standard text bubble
  const isUser = message.sender_kind === "user";
  const model = typeof message.metadata.model === "string" ? message.metadata.model : null;
  const effort = typeof message.metadata.effort === "string" ? message.metadata.effort : null;
  const streaming = message.metadata.streaming === true;
  const counters = message.turn_id ? turnCounters[message.turn_id] : undefined;

  return (
    <div className={`px-4 py-2 flex ${isUser ? "justify-end" : "justify-start"}`}>
      <div
        className={`max-w-[85%] rounded-2xl px-4 py-3 space-y-1.5 border ${
          isUser
            ? "bg-indigo-600/20 border-indigo-500/30 text-zinc-100"
            : "bg-zinc-900/90 border-zinc-800 text-zinc-200"
        }`}
      >
        <div className="flex items-center justify-between gap-4 text-[11px]">
          <div className="flex items-center gap-2">
            {!isUser && (
              <span
                className="h-2 w-2 rounded-full"
                style={{ backgroundColor: accent }}
              />
            )}
            <span className="font-semibold text-zinc-200">
              {isUser ? "You" : `@${message.sender_id}`}
            </span>
            {message.sender_kind === "orchestrator" && (
              <span className="inline-flex items-center gap-0.5 text-[10px] px-1.5 py-0.2 rounded bg-amber-500/15 text-amber-300">
                <Sparkles className="w-2.5 h-2.5" />
                <span>Orchestrator</span>
              </span>
            )}
            {model && (
              <span className="font-mono text-[10px] text-zinc-400">
                {model}
                {effort ? ` · ${effort}` : ""}
              </span>
            )}
          </div>
          <span className="text-[10px] text-zinc-500">
            {new Date(message.created_at).toLocaleTimeString([], {
              hour: "2-digit",
              minute: "2-digit",
            })}
          </span>
        </div>

        <MarkdownBlock
          content={message.content || (streaming ? "Thinking..." : "")}
        />

        {message.turn_id && (
          <div className="pt-1 flex items-center justify-between gap-3 border-t border-zinc-800/60 text-[11px]">
            <div className="text-zinc-500">
              {counters
                ? `${counters.tool_calls} tools · ${counters.file_edits} edits`
                : streaming
                  ? "Streaming..."
                  : "Trajectory recorded"}
            </div>
            <button
              type="button"
              onClick={() => {
                if (message.turn_id) void inspectTurn(message.turn_id);
              }}
              className="inline-flex items-center gap-1 text-indigo-400 hover:text-indigo-300 font-medium"
            >
              <SearchCode className="w-3.5 h-3.5" />
              <span>Inspect Trajectory</span>
            </button>
          </div>
        )}
      </div>
    </div>
  );
});

function PermissionInlineCard({
  sessionId,
  prompt,
}: {
  sessionId: string;
  prompt: PendingPermissionView;
}) {
  const { pendingPermissions } = useAppStore();
  const isPending = pendingPermissions.some(
    (p) => p.request_id === prompt.request_id
  );

  if (!prompt?.request_id) return null;

  return (
    <div className="mx-4 my-2 rounded-xl border border-rose-500/40 bg-rose-500/5 p-3 space-y-2 text-xs">
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-2 font-semibold text-rose-200">
          <ShieldAlert className="w-4 h-4 text-rose-400" />
          <span>
            Permission Required (@{prompt.member_handle})
          </span>
        </div>
        <span className="text-[10px] font-mono text-zinc-400">
          {prompt.tool_kind}
        </span>
      </div>
      <div className="text-zinc-200 font-medium">{prompt.title}</div>
      {prompt.detail && (
        <pre className="p-2 rounded bg-zinc-950 border border-zinc-800 font-mono text-[11px] text-zinc-300 overflow-x-auto max-h-28">
          {prompt.detail}
        </pre>
      )}
      {isPending ? (
        <div className="flex flex-wrap gap-2 pt-1">
          <button
            type="button"
            onClick={() =>
              void api.resolvePermission(sessionId, prompt.request_id, "allow_once")
            }
            className="px-3 py-1 rounded bg-emerald-600 hover:bg-emerald-500 text-white font-medium"
          >
            Allow Once (Alt+A)
          </button>
          {prompt.suggested_pattern && (
            <button
              type="button"
              onClick={() =>
                void api.resolvePermission(
                  sessionId,
                  prompt.request_id,
                  "allow_always",
                  prompt.suggested_pattern ?? undefined
                )
              }
              className="px-3 py-1 rounded bg-indigo-600 hover:bg-indigo-500 text-white font-medium"
            >
              Allow "{prompt.suggested_pattern}" for Session
            </button>
          )}
          <button
            type="button"
            onClick={() =>
              void api.resolvePermission(sessionId, prompt.request_id, "reject_once")
            }
            className="px-3 py-1 rounded bg-zinc-800 hover:bg-zinc-700 text-rose-300"
          >
            Reject (Alt+R)
          </button>
        </div>
      ) : (
        <div className="text-[11px] text-zinc-500">Resolved</div>
      )}
    </div>
  );
}
