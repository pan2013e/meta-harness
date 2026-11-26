import { memo, useState } from "react";
import {
  Activity,
  ChevronDown,
  ChevronRight,
  Crown,
  GitCommitHorizontal,
  ShieldAlert,
  Sparkles,
} from "lucide-react";
import { api } from "../../api/client";
import { useAppStore } from "../../store/useAppStore";
import type { ChatMessage, DispatchTaskSpec, PendingPermissionView } from "../../types/api";
import { HarnessAvatar } from "../common/Badges";
import { MarkdownBlock } from "../common/MarkdownBlock";
import { TurnActivityTimeline, WaitingForTurn } from "./TurnActivityTimeline";

interface Props {
  message: ChatMessage;
  consultReplies: ChatMessage[];
}

export const MessageRow = memo(function MessageRow({ message, consultReplies }: Props) {
  const accent = useAppStore((state) => {
    const member = state.members.find((m) => m.handle === message.sender_id);
    return (member && state.adapters[member.adapter_id]?.accent_color) || "#6366f1";
  });
  const [consultExpanded, setConsultExpanded] = useState(false);
  const streaming = message.metadata.streaming === true;
  const waitingForApproval = useAppStore((state) =>
    streaming && state.pendingPermissions.some((prompt) =>
      prompt.session_id === message.session_id &&
      prompt.member_handle === message.sender_id &&
      (prompt.turn_id === null || prompt.turn_id === message.turn_id)
    )
  );

  if (message.kind === "activity_pill") {
    return (
      <div className="px-6 py-1 flex items-center justify-center">
        <div className="inline-flex items-center gap-1.5 px-3 py-1 rounded-full bg-elevated border border-line text-[11px] text-ink-muted">
          <Activity className="w-3 h-3 text-brand-ink" />
          <span>{message.content}</span>
        </div>
      </div>
    );
  }

  if (message.kind === "merge_notice" || message.kind === "system_notice") {
    return (
      <div className="px-6 py-1 flex items-center justify-center">
        <div
          className={`inline-flex items-center gap-1.5 rounded-lg border px-3 py-1 text-xs ${
            message.metadata.error
              ? "border-neg/25 bg-neg-soft text-neg"
              : "border-line bg-elevated text-ink-soft"
          }`}
        >
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
        <div className="mx-auto max-w-[50rem] space-y-2 rounded-xl border border-line bg-elevated/60 p-3.5 text-xs">
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
                @{d.target.replace(/^@+/, "")} · {d.model ?? "Auto"} / {d.effort ?? "auto"}
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
        <div className="mx-auto max-w-[50rem] rounded-xl border border-line-strong bg-elevated/70 p-3 text-xs space-y-2">
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
                <MarkdownBlock content={r.content} sessionId={message.session_id} />
              </div>
            ))}
        </div>
      </div>
    );
  }

  const isUser = message.sender_kind === "user";
  const model = typeof message.metadata.model === "string" ? message.metadata.model : null;
  const effort = typeof message.metadata.effort === "string" ? message.metadata.effort : null;
  const intermediateNotes = Array.isArray(message.metadata.intermediate_notes)
    ? message.metadata.intermediate_notes.filter((x): x is string => typeof x === "string")
    : [];
  const time = new Date(message.created_at).toLocaleTimeString([], {
    hour: "2-digit",
    minute: "2-digit",
  });

  if (isUser) {
    return (
      <article
        aria-label="Your message"
        className="mx-auto flex max-w-[50rem] justify-end px-4 py-3 sm:px-6"
      >
        <div className="min-w-0 max-w-[88%] sm:max-w-[78%]">
          <div className="mb-1.5 flex items-center justify-end gap-2 px-1 text-xs">
            <span className="text-ink-muted">{time}</span>
            <span className="font-medium text-ink-soft">You</span>
          </div>
          <div className="rounded-2xl rounded-tr-md border border-brand/25 bg-brand-soft px-4 py-2.5">
            <MarkdownBlock content={message.content} sessionId={message.session_id} />
          </div>
        </div>
      </article>
    );
  }

  return (
    <article className="mx-auto max-w-[50rem] px-4 py-3.5 sm:px-6">
      <div className="flex items-start gap-3">
        <HarnessAvatar handle={message.sender_id} accentColor={accent} size="sm" />

        <div className="flex-1 min-w-0 space-y-2">
          <div className="flex items-center gap-2">
            <div className="flex items-center gap-2 flex-wrap">
              <span className="text-[13px] font-semibold text-ink">
                @{message.sender_id}
              </span>
              {message.sender_kind === "orchestrator" && (
                <span className="inline-flex items-center gap-1 text-[10px] px-1.5 py-0.5 rounded-md bg-elevated text-ink-soft font-medium">
                  <Sparkles className="w-2.5 h-2.5" />
                  <span>Orchestrator</span>
                </span>
              )}
              {model && (
                <span className="text-[11px] text-ink-muted font-mono">
                  {model}
                  {effort ? ` · ${effort}` : ""}
                </span>
              )}
            </div>
            <span className="shrink-0 text-[11px] text-ink-muted ml-auto">{time}</span>
          </div>

          {message.turn_id && (
            <TurnActivityTimeline
              turnId={message.turn_id}
              sessionId={message.session_id}
              streaming={streaming}
              waitingForApproval={waitingForApproval}
              intermediateNotes={intermediateNotes}
            />
          )}

          {!message.turn_id && streaming && !message.content.trim() && (
            <WaitingForTurn waitingForApproval={waitingForApproval} />
          )}

          {message.content.trim() && (
            <MarkdownBlock
              content={message.content}
              sessionId={message.session_id}
            />
          )}
        </div>
      </div>
    </article>
  );
});

function PermissionInlineCard({
  sessionId,
  prompt,
}: {
  sessionId: string;
  prompt: PendingPermissionView;
}) {
  const isPending = useAppStore((state) =>
    state.pendingPermissions.some((p) => p.request_id === prompt.request_id)
  );
  const setError = useAppStore((state) => state.setError);
  const allow = prompt.options.find((option) => option.kind === "allow_once");
  const deny = prompt.options.find((option) => option.kind === "reject_once");
  const patternOption =
    allow ?? prompt.options.find((option) => option.kind === "allow_always");
  const resolve = (optionId: string, pattern?: string) => {
    void api
      .resolvePermission(sessionId, prompt.request_id, optionId, pattern)
      .catch((error: unknown) =>
        setError(error instanceof Error ? error.message : String(error))
      );
  };

  if (!prompt?.request_id) return null;

  return (
    <div className="px-5 py-1.5">
      <div className="mx-auto max-w-[50rem] rounded-xl border border-neg/40 bg-neg-soft p-3.5 space-y-2 text-xs">
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
            {allow && (
              <button
                type="button"
                onClick={() => resolve(allow.option_id)}
                className="px-3 py-1.5 rounded-lg bg-emerald-600 hover:bg-emerald-500 text-white font-medium"
              >
                Allow Once (Alt+A)
              </button>
            )}
            {patternOption && prompt.suggested_pattern && (
              <button
                type="button"
                onClick={() =>
                  resolve(patternOption.option_id, prompt.suggested_pattern ?? undefined)
                }
                className="px-3 py-1.5 rounded-lg bg-brand hover:bg-brand-hover text-white font-medium"
              >
                Allow &ldquo;{prompt.suggested_pattern}&rdquo; for Session
              </button>
            )}
            {deny && (
              <button
                type="button"
                onClick={() => resolve(deny.option_id)}
                className="px-3 py-1.5 rounded-lg bg-elevated hover:bg-hover border border-line text-ink-soft"
              >
                Deny (Alt+R)
              </button>
            )}
          </div>
        ) : (
          <div className="text-[11px] text-ink-muted">Permission resolved.</div>
        )}
      </div>
    </div>
  );
}
