import { memo } from "react";
import { ChevronRight, FileCode, ListChecks, Terminal, Wrench } from "lucide-react";
import { DiffViewer } from "../common/DiffViewer";
import { MarkdownBlock } from "../common/MarkdownBlock";
import { asRecord, type TrajectoryEntry } from "./trajectory";

export const TrajectoryRow = memo(function TrajectoryRow({ entry, sessionId }: { entry: TrajectoryEntry; sessionId?: string }) {
  if (entry.type === "tool") {
    const terminal = entry.category === "execute" || entry.exitCode != null || !!entry.output;
    const Icon = terminal ? Terminal : entry.edits.length ? FileCode : Wrench;
    const status = entry.exitCode != null ? `Exit ${entry.exitCode}` : entry.status?.replaceAll("_", " ");
    const failed = entry.status === "failed" || (entry.exitCode != null && entry.exitCode !== 0);
    const command = asRecord(entry.input).command;
    const title = entry.title ?? entry.name ?? (terminal ? "Terminal command" : "Tool call");
    const showCommand = typeof command === "string" && command !== title;
    const output = entry.output || entry.content.join("\n") || (typeof entry.result === "string" ? entry.result : "");
    return (
      <details className="group rounded-xl border border-line bg-elevated/50 text-xs">
        <summary className="flex list-none items-center gap-2 rounded-xl px-3 py-2.5 hover:bg-hover [&::-webkit-details-marker]:hidden">
          <Icon size={15} className="shrink-0 text-brand-ink" />
          <span className={`min-w-0 flex-1 truncate ${terminal ? "font-mono" : "font-medium"}`} title={title}>{title}</span>
          {status && <span className={`shrink-0 capitalize ${failed ? "text-neg" : "text-ink-muted"}`}>{status}</span>}
          <ChevronRight size={13} className="shrink-0 text-ink-muted group-open:rotate-90" />
        </summary>
        <div className="space-y-2 border-t border-line p-3">
          {entry.cwd && <p className="break-all font-mono text-[11px] text-ink-muted">{entry.cwd}</p>}
          {showCommand && <pre className="whitespace-pre-wrap break-words rounded-lg bg-inset p-2 font-mono text-ink-soft">{command}</pre>}
          {!showCommand && entry.input !== undefined && !terminal && (
            <details><summary className="text-ink-muted">Input</summary><pre className="overflow-x-auto p-1">{JSON.stringify(entry.input, null, 2)}</pre></details>
          )}
          {output && (
            <div>
              <h4 className="mb-1 text-ink-muted">{entry.outputTruncated ? "Output (last 24,000 characters)" : "Output"}</h4>
              {terminal ? (
                <pre className="max-h-64 overflow-auto whitespace-pre-wrap break-words rounded-lg bg-inset p-2 leading-relaxed text-ink-soft">{output}</pre>
              ) : <MarkdownBlock content={output.trim()} sessionId={sessionId} />}
            </div>
          )}
          {!output && entry.result != null && !(entry.exitCode != null && Object.keys(asRecord(entry.result)).length === 1) && (
            <details><summary className="text-ink-muted">Result</summary><pre className="overflow-x-auto p-1">{JSON.stringify(entry.result, null, 2)}</pre></details>
          )}
          {entry.edits.map((edit, index) => edit.diff ? (
            <DiffViewer key={`${edit.path}-${index}`} diffText={edit.diff} />
          ) : (
            <details key={`${edit.path}-${index}`} className="rounded-lg bg-inset p-2">
              <summary className="break-all font-mono">{edit.path}</summary>
              {edit.before !== undefined && <pre className="max-h-48 overflow-auto whitespace-pre-wrap text-neg">{edit.before}</pre>}
              {edit.after !== undefined && <pre className="max-h-48 overflow-auto whitespace-pre-wrap text-pos">{edit.after}</pre>}
            </details>
          ))}
        </div>
      </details>
    );
  }

  const { event } = entry;
  const { payload } = event;
  if (event.event_kind === "diff" && typeof payload.diff === "string") return <DiffViewer diffText={payload.diff} />;
  const plan = event.event_kind === "plan" && Array.isArray(payload.entries) ? payload.entries.map(asRecord) : null;
  const label = payload.kind === "narration" ? "Progress note" : event.event_kind.replaceAll("_", " ");
  return (
    <div className="space-y-1.5 rounded-xl border border-line bg-elevated/40 px-3 py-2 text-xs">
      <div className="flex items-center justify-between gap-2 text-ink-muted">
        <span className="font-medium capitalize">{label}</span>
        <time className="text-[10px]">{new Date(event.created_at).toLocaleTimeString()}</time>
      </div>
      {plan ? (
        <ul className="space-y-1.5">
          {plan.map((item, index) => <li key={index} className="flex items-start gap-2"><ListChecks size={13} className="shrink-0 text-brand-ink" /><div className="min-w-0 flex-1"><MarkdownBlock content={String(item.content ?? "").trim()} sessionId={sessionId} /></div><span className="shrink-0 text-ink-muted">{String(item.status ?? "").replaceAll("_", " ")}</span></li>)}
        </ul>
      ) : typeof payload.text === "string" ? (
        <MarkdownBlock content={payload.text.trim()} sessionId={sessionId} />
      ) : (
        <pre className="overflow-x-auto whitespace-pre-wrap font-mono text-ink-soft">{JSON.stringify(payload, null, 2)}</pre>
      )}
    </div>
  );
});
