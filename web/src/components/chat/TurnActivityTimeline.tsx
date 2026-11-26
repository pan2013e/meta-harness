import { useEffect, useMemo, useState } from "react";
import { ChevronDown, ChevronRight, FileDiff, Loader2, Sparkles, Terminal } from "lucide-react";
import { useAppStore } from "../../store/useAppStore";
import { MarkdownBlock } from "../common/MarkdownBlock";
import { TrajectoryRow } from "../drawer/TrajectoryRow";
import { cleanWorktreePath, groupTrajectoryEvents } from "../drawer/trajectory";

export function TurnActivityTimeline({
  turnId,
  sessionId,
  streaming,
  intermediateNotes,
}: {
  turnId: string;
  sessionId: string;
  streaming: boolean;
  intermediateNotes: string[];
}) {
  const [expanded, setExpanded] = useState(false);
  const events = useAppStore((s) => s.turnEvents[turnId]);
  const counters = useAppStore((s) => s.turnCounters[turnId]);
  const snapshotReady = useAppStore((s) => s.turnEventSnapshots[turnId]);
  const loadTurnEvents = useAppStore((s) => s.loadTurnEvents);

  useEffect(() => {
    if (expanded && !snapshotReady) {
      void loadTurnEvents(turnId);
    }
  }, [expanded, snapshotReady, turnId, loadTurnEvents]);

  const entries = useMemo(() => groupTrajectoryEvents(events ?? []), [events]);

  const hasNarrationInEvents = useMemo(
    () =>
      (events ?? []).some(
        (ev) => ev.event_kind === "thought" && ev.payload.kind === "narration"
      ),
    [events]
  );

  const fallbackNotes = hasNarrationInEvents ? [] : intermediateNotes;

  const totalSteps =
    entries.length > 0
      ? entries.length + fallbackNotes.length
      : (counters?.tool_calls ?? 0) + (counters?.thoughts ?? 0) + fallbackNotes.length;

  const editsCount =
    counters?.file_edits ??
    entries.reduce((sum, e) => sum + (e.type === "tool" ? e.edits.length : e.event.event_kind === "diff" ? 1 : 0), 0);

  const latestTitle = useMemo(() => {
    const last = entries.at(-1);
    if (!last) {
      return fallbackNotes.at(-1)?.slice(0, 72) ?? null;
    }
    if (last.type === "tool") {
      return cleanWorktreePath(last.title ?? last.name ?? "Running tool");
    }
    if (last.event.event_kind === "diff") {
      const p = typeof last.event.payload.path === "string" ? last.event.payload.path : "file";
      return `Edited ${cleanWorktreePath(p)}`;
    }
    if (last.event.event_kind === "thought") {
      return "Thinking";
    }
    return last.event.event_kind;
  }, [entries, fallbackNotes]);

  const summaryLabel = streaming
    ? `Working${latestTitle ? ` · ${latestTitle}` : ""}`
    : [
        `Worked`,
        totalSteps > 0 ? `${totalSteps} step${totalSteps === 1 ? "" : "s"}` : null,
        editsCount > 0 ? `${editsCount} file edit${editsCount === 1 ? "" : "s"}` : null,
      ]
        .filter(Boolean)
        .join(" · ");

  return (
    <div className="my-1 rounded-xl border border-line bg-elevated/45 overflow-hidden text-xs">
      <button
        type="button"
        onClick={() => setExpanded((v) => !v)}
        aria-expanded={expanded}
        className="w-full px-3 py-1.5 flex items-center justify-between gap-2 text-left text-ink-muted hover:text-ink hover:bg-hover/60 transition-colors"
      >
        <span className="flex items-center gap-2 min-w-0">
          {streaming ? (
            <Loader2 className="w-3.5 h-3.5 animate-spin text-brand-ink shrink-0" />
          ) : expanded ? (
            <ChevronDown className="w-3.5 h-3.5 shrink-0" />
          ) : (
            <ChevronRight className="w-3.5 h-3.5 shrink-0" />
          )}
          <span className="truncate font-medium">{summaryLabel}</span>
        </span>
        <span className="flex items-center gap-2 text-[10.5px] font-mono shrink-0">
          {editsCount > 0 && (
            <span className="inline-flex items-center gap-1 text-pos">
              <FileDiff className="w-3 h-3" />
              {editsCount}
            </span>
          )}
          {!streaming && (counters?.tool_calls ?? 0) > 0 && editsCount === 0 && (
            <span className="inline-flex items-center gap-1">
              <Terminal className="w-3 h-3" />
              {counters?.tool_calls}
            </span>
          )}
          <span>{expanded ? "Hide" : "Steps"}</span>
        </span>
      </button>

      {expanded && (
        <div className="border-t border-line bg-app/50 p-2.5 space-y-2 max-h-96 overflow-y-auto">
          {fallbackNotes.map((note, idx) => (
            <div
              key={`note-${idx}`}
              className="rounded-lg border border-line bg-panel/70 px-3 py-2 text-xs text-ink-soft whitespace-pre-wrap"
            >
              <div className="flex items-center gap-1.5 text-[10px] uppercase tracking-wider text-ink-muted mb-1">
                <Sparkles className="w-3 h-3 text-brand-ink" />
                <span>Progress note</span>
              </div>
              <MarkdownBlock content={note.trim()} sessionId={sessionId} />
            </div>
          ))}

          {entries.map((entry) => (
            <TrajectoryRow key={entry.key} entry={entry} sessionId={sessionId} />
          ))}

          {entries.length === 0 && fallbackNotes.length === 0 && (
            <div className="py-3 text-center text-[11px] text-ink-muted">
              {snapshotReady ? "No steps recorded yet." : "Loading steps…"}
            </div>
          )}
        </div>
      )}
    </div>
  );
}
