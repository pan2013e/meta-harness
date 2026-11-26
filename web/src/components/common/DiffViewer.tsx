import { useMemo, useState } from "react";
import { ChevronDown, ChevronRight, FileCode2, FileImage } from "lucide-react";

export interface DiffSection {
  filename: string;
  additions: number;
  deletions: number;
  binary: boolean;
  lines: string[];
}

export function parseUnifiedDiff(diffText: string): DiffSection[] {
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
        binary: false,
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
        binary: false,
        lines: [],
      };
    }

    if (line.startsWith("Binary files ") && line.endsWith(" differ")) {
      current.binary = true;
      current.lines.push(line);
      continue;
    }

    if (line.startsWith("@@")) inHunk = true;

    if (!inHunk && line.startsWith("+++ b/")) {
      current.filename = line.slice(6);
      continue;
    }
    if (
      !inHunk &&
      (line.startsWith("index ") ||
        line.startsWith("--- ") ||
        line.startsWith("+++ ") ||
        line.startsWith("new file mode") ||
        line.startsWith("deleted file mode"))
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

export function DiffViewer({
  diffText,
  selectedFile,
}: {
  diffText: string;
  selectedFile?: string | null;
}) {
  const sections = useMemo(() => {
    const parsed = parseUnifiedDiff(diffText);
    return selectedFile ? parsed.filter((s) => s.filename === selectedFile) : parsed;
  }, [diffText, selectedFile]);
  const [collapsed, setCollapsed] = useState<Record<string, boolean>>({});

  if (!diffText.trim() || sections.length === 0) {
    return (
      <div className="p-8 text-center text-xs text-ink-muted">
        No changes yet on integration branch.
      </div>
    );
  }

  return (
    <div className="space-y-3 min-w-0">
      {sections.map((sec, idx) => {
        const key = `${sec.filename}-${idx}`;
        const isClosed = Boolean(collapsed[key]);
        return (
          <div
            key={key}
            className="rounded-xl border border-line bg-inset overflow-hidden min-w-0"
          >
            <button
              type="button"
              onClick={() =>
                setCollapsed((prev) => ({ ...prev, [key]: !prev[key] }))
              }
              className="w-full px-3 py-2 bg-elevated hover:bg-hover border-b border-line flex items-center justify-between gap-2 text-xs transition-colors"
            >
              <span className="flex items-center gap-2 font-mono text-ink min-w-0">
                {isClosed ? (
                  <ChevronRight className="w-3.5 h-3.5 text-ink-muted shrink-0" />
                ) : (
                  <ChevronDown className="w-3.5 h-3.5 text-ink-muted shrink-0" />
                )}
                {sec.binary ? (
                  <FileImage className="w-3.5 h-3.5 text-brand-ink shrink-0" />
                ) : (
                  <FileCode2 className="w-3.5 h-3.5 text-brand-ink shrink-0" />
                )}
                <span className="truncate">{sec.filename}</span>
              </span>
              <span className="flex items-center gap-2 font-mono text-[11px] shrink-0">
                {sec.binary ? (
                  <span className="px-1.5 py-0.2 rounded bg-brand-soft text-brand-ink text-[10px]">
                    binary
                  </span>
                ) : (
                  <>
                    <span className="text-pos">+{sec.additions}</span>
                    <span className="text-neg">-{sec.deletions}</span>
                  </>
                )}
              </span>
            </button>

            {!isClosed && (
              <div className="overflow-x-auto font-mono text-[11.5px] leading-5 py-1 max-w-full">
                {sec.lines.map((line, li) => {
                  const isHunk = line.startsWith("@@");
                  const isAdd = !sec.binary && line.startsWith("+");
                  const isDel = !sec.binary && line.startsWith("-");
                  const cls = isHunk
                    ? "bg-brand-soft text-brand-ink font-semibold"
                    : isAdd
                      ? "bg-pos-soft text-pos"
                      : isDel
                        ? "bg-neg-soft text-neg"
                        : "text-ink-soft";
                  return (
                    <div key={li} className={`px-3 whitespace-pre min-w-max ${cls}`}>
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
