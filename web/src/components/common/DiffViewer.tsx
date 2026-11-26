import { useMemo, useState } from "react";
import { ChevronDown, ChevronRight, FileCode2 } from "lucide-react";

interface DiffSection {
  filename: string;
  additions: number;
  deletions: number;
  lines: string[];
}

function parseUnifiedDiff(diffText: string): DiffSection[] {
  const rawLines = diffText.split(/\r?\n/);
  const sections: DiffSection[] = [];
  let current: DiffSection | null = null;
  let inHunk = false;

  for (const line of rawLines) {
    if (line.startsWith("diff --git ")) {
      if (current) sections.push(current);
      inHunk = false;
      const m = line.match(/^diff --git a\/.+ b\/(.+)$/);
      current = {
        filename: m ? m[1] : line.slice(11),
        additions: 0,
        deletions: 0,
        lines: [],
      };
      continue;
    }

    if (!current) {
      if (!line.trim()) continue;
      current = {
        filename: "Patch",
        additions: 0,
        deletions: 0,
        lines: [],
      };
    }

    if (line.startsWith("@@")) inHunk = true;

    if (!inHunk && line.startsWith("+++ b/")) {
      current.filename = line.slice(6);
      continue;
    }
    if (
      !inHunk && (line.startsWith("index ") ||
      line.startsWith("--- ") || line.startsWith("+++ ") ||
      line.startsWith("new file mode") || line.startsWith("deleted file mode"))
    ) {
      continue;
    }

    if (inHunk && line.startsWith("+")) {
      current.additions += 1;
    } else if (inHunk && line.startsWith("-")) {
      current.deletions += 1;
    }
    current.lines.push(line);
  }

  if (current) sections.push(current);
  return sections;
}

export function DiffViewer({ diffText }: { diffText: string }) {
  const sections = useMemo(() => parseUnifiedDiff(diffText), [diffText]);
  const [collapsed, setCollapsed] = useState<Record<number, boolean>>({});

  if (!diffText.trim() || sections.length === 0) {
    return (
      <div className="p-8 text-center text-xs text-ink-muted">
        No changes yet on integration branch.
      </div>
    );
  }

  return (
    <div className="space-y-3">
      {sections.map((sec, idx) => {
        const isClosed = Boolean(collapsed[idx]);
        return (
          <div
            key={`${sec.filename}-${idx}`}
            className="rounded-xl border border-line bg-inset overflow-hidden"
          >
            <button
              type="button"
              onClick={() =>
                setCollapsed((prev) => ({ ...prev, [idx]: !prev[idx] }))
              }
              className="w-full px-3 py-2 bg-elevated hover:bg-hover border-b border-line flex items-center justify-between text-xs transition-colors"
            >
              <span className="flex items-center gap-2 font-mono text-ink truncate">
                {isClosed ? (
                  <ChevronRight className="w-3.5 h-3.5 text-ink-muted shrink-0" />
                ) : (
                  <ChevronDown className="w-3.5 h-3.5 text-ink-muted shrink-0" />
                )}
                <FileCode2 className="w-3.5 h-3.5 text-brand-ink shrink-0" />
                <span className="truncate">{sec.filename}</span>
              </span>
              <span className="flex items-center gap-2 font-mono text-[11px] shrink-0">
                <span className="text-pos">+{sec.additions}</span>
                <span className="text-neg">-{sec.deletions}</span>
              </span>
            </button>

            {!isClosed && (
              <div className="overflow-x-auto font-mono text-[11.5px] leading-5 py-1">
                {sec.lines.map((line, li) => {
                  const isHunk = line.startsWith("@@");
                  const isAdd = line.startsWith("+");
                  const isDel = line.startsWith("-");
                  const cls = isHunk
                    ? "bg-brand-soft text-brand-ink font-semibold"
                    : isAdd
                      ? "bg-pos-soft text-pos"
                      : isDel
                        ? "bg-neg-soft text-neg"
                        : "text-ink-soft";
                  return (
                    <div key={li} className={`px-3 whitespace-pre ${cls}`}>
                      {line || " "}
                    </div>
                  );
                })}
              </div>
            )}
          </div>
        );
      })}
    </div>
  );
}
