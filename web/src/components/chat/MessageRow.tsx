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
import { HarnessAvatar } from "../common/Badges";
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
      <div className="px-6 py-1 flex items-center justify-center">
        <button
          type="button"
          onClick={() => {
            if (message.turn_id) void inspectTurn(message.turn_id);
          }}
          className="inline-flex items-center gap-1.5 px-3 py-1 rounded-full bg-elevated border border-line text-[11px] text-ink-muted hover:text-ink hover:border-line-strong transition-colors"
        >
          <Activity className="w-3 h-3 text-brand-ink" />
          <span>{message.content}</span>
        </button>
      </div>
    );
  }

  if (message.kind === "merge_notice" || message.kind === "system_notice") {
    return (
      <div className="px-6 py-1 flex items-center justify-center">
        <div className="inline-flex items-center gap-1.5 px-3 py-1 rounded-lg bg-pos-soft border border-pos/25 text-[11px] text-pos">
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
      <div className="px-5 py-1.5">
        <div className="rounded-xl border border-warn/30 bg-warn-soft/50 p-3.5 space-y-2 text-xs">
          <div className="flex items-center justify-between gap-2">
            <div className="flex items-center gap-2 font-semibold text-warn">
              <Crown className="w-3.5 h-3.5" />
              <span>Orchestrator Dispatch · @{message.sender_id}</span>
            </div>
            <span className="px-2 py-0.5 rounded-full text-[10px] uppercase tracking-wider bg-panel text-ink-soft border border-line font-mono">
              {status.replace("_", " ")}
            </span>
          </div>
          <p className="text-ink">{message.content}</p>
          <div className="flex flex-wrap gap-1.5">
            {dispatches.map((d, idx) => (
              <span
                key={idx}
                className="px-2 py-0.5 rounded-md bg-panel border border-line font-mono text-[11px] text-brand-ink"
              >
                @{d.target} [{d.model ?? "auto"} · {d.effort ?? "auto"}]
              </span>
            ))}
          </div>
          {status === "awaiting_confirmation" && dispatchId && (
            <div className="flex gap-2 pt-1">
              <button
                type="button"
                onClick={() => void api.confirmDispatch(message.session_id, dispatchId)}
                className="px-3 py-1 rounded-lg bg-amber-500 hover:bg-amber-400 text-zinc-950 font-semibold transition-colors"
              >
                Confirm & Run
              </button>
              <button
                type="button"
                onClick={() => void api.cancelDispatch(message.session_id, dispatchId)}
                className="px-3 py-1 rounded-lg bg-elevated hover:bg-hover border border-line text-ink-soft transition-colors"
              >
                Cancel
              </button>
            </div>
          )}
        </div>
      </div>
    );
  }

  if (message.kind === "permission_card") {
    const prompt = message.metadata as unknown as PendingPermissionView;
    return <PermissionInlineCard sessionId={message.session_id} prompt={prompt} />;
  }

  if (message.kind === "consult_thread") {
    if (message.parent_msg_id) {
      return null;
    }
    const target = String(message.metadata.target ?? "peer");
    return (
      <div className="px-5 py-1.5">
        <div className="rounded-xl border border-line-strong bg-elevated/70 p-3 text-xs space-y-2">
          <button
            type="button"
            onClick={() => setConsultExpanded((v) => !v)}
            className="w-full flex items-center justify-between text-left text-brand-ink font-medium"
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
            <span className="text-[10px] font-mono px-2 py-0.5 rounded-full bg-panel border border-line text-ink-muted">
              {consultReplies.length > 0 ? "Answered" : "Waiting..."}
            </span>
          </button>
          <div className="text-ink-soft pl-5">{message.content}</div>
          {consultExpanded &&
            consultReplies.map((r) => (
              <div
                key={r.id}
                className="mt-2 ml-5 pl-3 border-l-2 border-brand/50 space-y-1"
              >
                <div className="text-[11px] font-semibold text-brand-ink">
                  @{r.sender_id} replied:
                </div>
                <MarkdownBlock content={r.content} />
              </div>
            ))}
        </div>
      </div>
    );
  }

  // Developer Thread Row (Slack / Discord / Linear style)
  const isUser = message.sender_kind === "user";
  const model = typeof message.metadata.model === "string" ? message.metadata.model : null;
  const effort = typeof message.metadata.effort === "string" ? message.metadata.effort : null;
  const streaming = message.metadata.streaming === true;
  const counters = message.turn_id ? turnCounters[message.turn_id] : undefined;

  return (
    <div
      className={`px-5 py-3 border-b border-line/40 transition-colors hover:bg-hover/35 ${
        isUser ? "bg-panel/40" : ""
      }`}
    >
      <div className="max-w-4xl mx-auto flex items-start gap-3.5">
        <HarnessAvatar
          handle={isUser ? "You" : message.sender_id}
          accentColor={accent}
          isUser={isUser}
          size="md"
        />

        <div className="flex-1 min-w-0 space-y-1.5">
          {/* Sender Metadata Row */}
          <div className="flex items-center justify-between gap-3">
            <div className="flex items-center gap-2 flex-wrap">
              <span className="text-[13px] font-semibold text-ink">
                {isUser ? "You" : `@${message.sender_id}`}
              </span>
              {message.sender_kind === "orchestrator" && (
                <span className="inline-flex items-center gap-1 text-[10px] px-1.5 py-0.5 rounded-md bg-warn-soft text-warn font-medium">
                  <Sparkles className="w-2.5 h-2.5" />
                  <span>Orchestrator</span>
                </span>
              )}
              {model && (
                <span className="font-mono text-[10.5px] px-1.5 py-0.5 rounded-md bg-elevated border border-line text-ink-soft">
                  {model}
                  {effort ? ` · ${effort}` : ""}
                </span>
              )}
              {streaming && (
                <span className="inline-flex items-center gap-1 text-[10px] font-mono text-brand-ink">
                  <span className="w-1.5 h-1.5 rounded-full bg-brand animate-ping" />
                  <span>streaming</span>
                </span>
              )}
            </div>
            <span className="text-[10.5px] text-ink-muted font-mono shrink-0">
              {new Date(message.created_at).toLocaleTimeString([], {
                hour: "2-digit",
                minute: "2-digit",
              })}
            </span>
          </div>

          {/* Content */}
          <MarkdownBlock
            content={message.content || (streaming ? "_Thinking..._" : "")}
          />

          {/* Trajectory Footer Pill */}
          {message.turn_id && (
            <div className="pt-1 flex items-center gap-3 text-[11px]">
              <button
                type="button"
                onClick={() => {
                  if (message.turn_id) void inspectTurn(message.turn_id);
                }}
                className="inline-flex items-center gap-1.5 px-2.5 py-1 rounded-lg bg-elevated hover:bg-hover border border-line text-ink-soft hover:text-brand-ink transition-colors"
              >
                <SearchCode className="w-3.5 h-3.5 text-brand-ink" />
                <span>
                  {counters
                    ? `${counters.tool_calls} tools · ${counters.file_edits} edits`
                    : streaming
                      ? "Live trace"
                      : "Inspect Trajectory"}
                </span>
              </button>
            </div>
          )}
        </div>
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
    <div className="px-5 py-1.5">
      <div className="max-w-4xl mx-auto rounded-xl border border-neg/40 bg-neg-soft p-3.5 space-y-2 text-xs">
        <div className="flex items-center justify-between">
          <div className="flex items-center gap-2 font-semibold text-neg">
            <ShieldAlert className="w-4 h-4" />
            <span>Permission Required (@{prompt.member_handle})</span>
          </div>
          <span className="text-[10px] font-mono px-2 py-0.5 rounded bg-panel border border-line text-ink-muted">
            {prompt.tool_kind}
          </span>
        </div>
        <div className="text-ink font-medium">{prompt.title}</div>
        {prompt.detail && (
          <pre className="p-2.5 rounded-lg bg-inset border border-line font-mono text-[11px] text-ink-soft overflow-x-auto max-h-28">
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
              className="px-3 py-1.5 rounded-lg bg-emerald-600 hover:bg-emerald-500 text-white font-medium"
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
                className="px-3 py-1.5 rounded-lg bg-brand hover:bg-brand-hover text-white font-medium"
              >
                Allow &ldquo;{prompt.suggested_pattern}&rdquo; for Session
              </button>
            )}
            <button
              type="button"
              onClick={() =>
                void api.resolvePermission(sessionId, prompt.request_id, "reject_once")
              }
              className="px-3 py-1.5 rounded-lg bg-elevated hover:bg-hover border border-line text-ink-soft"
            >
              Deny (Alt+R)
            </button>
          </div>
        ) : (
          <div className="text-[11px] text-ink-muted">Permission resolved.</div>
        )}
      </div>
    </div>
  );
}
