import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  ArrowUp,
  ChevronRight,
  FolderGit2,
  FolderOpen,
  Loader2,
  Search,
  X,
} from "lucide-react";
import { notifyInApp, reportInAppError } from "../../inAppNotifications";
import { api } from "../../api/client";
import { useAppStore } from "../../store/useAppStore";
import type { FsEntry } from "../../types/api";

function stripOuterQuotes(raw: string): string {
  const s = raw.trim();
  if (
    s.length >= 2 &&
    ((s.startsWith('"') && s.endsWith('"')) || (s.startsWith("'") && s.endsWith("'")))
  ) {
    return s.slice(1, -1).trim();
  }
  return s;
}

function splitBreadcrumbs(pathStr: string): Array<{ label: string; path: string }> {
  if (!pathStr) return [];
  const isWin = pathStr.includes("\\") || /^[a-zA-Z]:/.test(pathStr);
  if (isWin) {
    const normalized = pathStr.replace(/\//g, "\\");
    const driveMatch = normalized.match(/^([a-zA-Z]:\\)(.*)$/);
    if (!driveMatch) return [{ label: pathStr, path: pathStr }];
    const drive = driveMatch[1];
    const rest = driveMatch[2].split("\\").filter(Boolean);
    const crumbs: Array<{ label: string; path: string }> = [
      { label: drive.slice(0, 2), path: drive },
    ];
    let acc = drive;
    for (const part of rest) {
      acc = acc.endsWith("\\") ? `${acc}${part}` : `${acc}\\${part}`;
      crumbs.push({ label: part, path: acc });
    }
    return crumbs;
  }

  const parts = pathStr.split("/").filter(Boolean);
  const crumbs: Array<{ label: string; path: string }> = [{ label: "/", path: "/" }];
  let acc = "";
  for (const part of parts) {
    acc = `${acc}/${part}`;
    crumbs.push({ label: part, path: acc });
  }
  return crumbs;
}

export function FolderPickerModal({
  onClose,
  onSelect,
}: {
  onClose: () => void;
  onSelect: (path: string, signal: AbortSignal) => Promise<void>;
}) {
  const { projects } = useAppStore();
  const [current, setCurrent] = useState("");
  const [parent, setParent] = useState<string | null>(null);
  const [entries, setEntries] = useState<FsEntry[]>([]);
  const [filterQuery, setFilterQuery] = useState("");
  const [loading, setLoading] = useState(true);
  const [submitting, setSubmitting] = useState(false);
  const [manualPath, setManualPath] = useState("");
  const [selectedPath, setSelectedPath] = useState("");
  const browseRequest = useRef<AbortController | null>(null);
  const openRequest = useRef<AbortController | null>(null);

  const load = useCallback(async (rawPath?: string) => {
    const target = rawPath ? stripOuterQuotes(rawPath) : undefined;
    browseRequest.current?.abort();
    const request = new AbortController();
    browseRequest.current = request;
    const timeout = AbortSignal.timeout(10_000);
    const signal = AbortSignal.any([request.signal, timeout]);
    setLoading(true);
    setFilterQuery("");
    try {
      const res = await api.listFs(target, signal);
      if (signal.aborted) return;
      setCurrent(res.current);
      setParent(res.parent);
      setEntries(res.entries);
      if (res.current) {
        setManualPath(res.current);
        setSelectedPath(res.current);
      } else {
        setSelectedPath("");
      }
    } catch (e) {
      if (!request.signal.aborted) {
        const message = e instanceof Error ? e.message : String(e);
        notifyInApp(timeout.aborted ? "Reading folders timed out. Try entering the folder path directly." : message, "error");
      }
    } finally {
      if (browseRequest.current === request) {
        browseRequest.current = null;
        if (!request.signal.aborted) setLoading(false);
      }
    }
  }, []);

  useEffect(() => {
    void load();
    return () => {
      browseRequest.current?.abort();
      openRequest.current?.abort();
    };
  }, [load]);

  const submitPath = async (raw: string) => {
    const cleaned = stripOuterQuotes(raw);
    if (!cleaned || openRequest.current) return;
    const request = new AbortController();
    openRequest.current = request;
    setSubmitting(true);
    try {
      await onSelect(cleaned, request.signal);
    } catch (e) {
      if (!request.signal.aborted) reportInAppError(e);
    } finally {
      openRequest.current = null;
      if (!request.signal.aborted) setSubmitting(false);
    }
  };

  const breadcrumbs = useMemo(() => splitBreadcrumbs(current), [current]);
  const visibleEntries = useMemo(() => {
    const q = filterQuery.trim().toLowerCase();
    return q
      ? entries.filter((item) => item.name.toLowerCase().includes(q))
      : entries;
  }, [entries, filterQuery]);
  const effectiveTarget = stripOuterQuotes(manualPath || selectedPath || current);

  return (
    <div
      className="fixed inset-0 z-50 bg-black/70 backdrop-blur-xs flex items-center justify-center p-4"
      onClick={onClose}
    >
      <div
        className="w-full max-w-2xl rounded-2xl border border-line-strong bg-panel shadow-2xl overflow-hidden flex flex-col max-h-[88vh]"
        onClick={(e) => e.stopPropagation()}
      >
        {/* Header */}
        <div className="px-5 py-3.5 border-b border-line flex items-center justify-between bg-elevated/60">
          <div className="flex items-center gap-2.5">
            <div className="w-7 h-7 rounded-lg bg-brand-soft border border-brand/30 flex items-center justify-center text-brand-ink">
              <FolderOpen className="w-4 h-4" />
            </div>
            <div>
              <h2 className="text-sm font-semibold text-ink">Open Project Folder</h2>
              <p className="text-[11px] text-ink-muted">
                Browse directories or paste a local repository path
              </p>
            </div>
          </div>
          <button
            type="button"
            aria-label="Close folder picker"
            onClick={onClose}
            className="p-1.5 rounded-lg text-ink-muted hover:text-ink hover:bg-hover transition-colors"
          >
            <X className="w-4 h-4" />
          </button>
        </div>

        <div className="p-4 space-y-3 overflow-y-auto">
          {/* Path Input + Go */}
          <div className="flex gap-2">
            <input
              type="text"
              value={manualPath}
              onChange={(e) => setManualPath(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") {
                  e.preventDefault();
                  void load(manualPath);
                }
              }}
              placeholder="Paste path or type ~/projects/app..."
              className="flex-1 rounded-xl bg-inset border border-line focus:border-brand px-3 py-2 text-xs font-mono text-ink placeholder-ink-muted focus:outline-none transition-colors"
            />
            <button
              type="button"
              onClick={() => void load(manualPath)}
              className="px-3.5 py-2 rounded-xl bg-elevated hover:bg-hover border border-line text-xs font-medium text-ink transition-colors"
            >
              Go
            </button>
          </div>

          {/* Quick Breadcrumbs & Subdirectory Filter */}
          <div className="flex flex-wrap items-center justify-between gap-2 px-1">
            <div className="flex items-center gap-1 flex-wrap text-[11px] text-ink-muted">
              <button
                type="button"
                onClick={() => void load("")}
                className="px-1.5 py-0.5 rounded hover:bg-hover hover:text-ink transition-colors font-medium"
              >
                Locations
              </button>
              {breadcrumbs.map((b, idx) => (
                <span key={b.path} className="inline-flex items-center gap-1">
                  <ChevronRight className="w-3 h-3 opacity-60" />
                  <button
                    type="button"
                    onClick={() => void load(b.path)}
                    className={`px-1.5 py-0.5 rounded font-mono transition-colors ${
                      idx === breadcrumbs.length - 1
                        ? "text-ink bg-elevated font-medium"
                        : "hover:bg-hover hover:text-ink"
                    }`}
                  >
                    {b.label}
                  </button>
                </span>
              ))}
            </div>

            {entries.length > 5 && (
              <div className="relative w-44">
                <Search className="w-3 h-3 text-ink-muted absolute left-2.5 top-1/2 -translate-y-1/2" />
                <input
                  type="text"
                  value={filterQuery}
                  onChange={(e) => setFilterQuery(e.target.value)}
                  placeholder="Filter folders…"
                  className="w-full rounded-lg bg-inset border border-line focus:border-brand pl-7 pr-2 py-1 text-[11px] text-ink focus:outline-none"
                />
              </div>
            )}
          </div>

          {/* Recent Projects */}
          {projects.length > 0 && (
            <div className="flex items-center gap-1.5 flex-wrap">
              <span className="text-[10px] uppercase tracking-wider font-semibold text-ink-muted mr-1">
                Recent:
              </span>
              {projects.slice(0, 4).map((p) => (
                <button
                  key={p.id}
                  type="button"
                  onClick={() => void submitPath(p.root_path)}
                  title={p.root_path}
                  className="inline-flex items-center gap-1 px-2 py-1 rounded-lg bg-elevated hover:bg-hover border border-line text-[11px] text-ink-soft hover:text-ink transition-colors"
                >
                  <FolderGit2 className="w-3 h-3 text-brand-ink" />
                  <span className="truncate max-w-[140px]">{p.name}</span>
                </button>
              ))}
            </div>
          )}

          {/* Directory Browser */}
          <div className="h-64 overflow-y-auto rounded-xl border border-line bg-inset divide-y divide-line">
            {current && (
              <button
                type="button"
                onClick={() => void load(parent ?? "")}
                className="w-full px-3.5 py-2 text-left text-xs text-ink-soft hover:bg-hover flex items-center gap-2 transition-colors"
              >
                <ArrowUp className="w-3.5 h-3.5 text-ink-muted" />
                <span>.. (Up to parent)</span>
              </button>
            )}

            {loading ? (
              <div className="h-48 flex items-center justify-center gap-2 text-xs text-ink-muted">
                <Loader2 className="w-4 h-4 animate-spin" />
                <span>Reading directories...</span>
              </div>
            ) : visibleEntries.length === 0 ? (
              <div className="h-48 flex items-center justify-center text-xs text-ink-muted px-4 text-center">
                {entries.length === 0
                  ? 'No subdirectories here — click "Open Project" below to use this folder.'
                  : "No folders match your filter."}
              </div>
            ) : (
              visibleEntries.map((entry) => {
                const isSelected = selectedPath === entry.path;
                return (
                  <div
                    key={entry.path}
                    onClick={() => {
                      setSelectedPath(entry.path);
                      setManualPath(entry.path);
                    }}
                    onDoubleClick={() => void load(entry.path)}
                    className={`group w-full px-3.5 py-2 text-xs flex items-center justify-between gap-2 cursor-pointer transition-colors ${
                      isSelected ? "bg-brand-soft" : "hover:bg-hover"
                    }`}
                  >
                    <div className="flex items-center gap-2.5 min-w-0 flex-1">
                      {entry.is_git_repo ? (
                        <FolderGit2 className="w-4 h-4 text-pos shrink-0" />
                      ) : (
                        <FolderOpen className="w-4 h-4 text-ink-muted shrink-0" />
                      )}
                      <span className="truncate font-medium text-ink">{entry.name}</span>
                      {entry.is_git_repo && (
                        <span className="text-[10px] px-1.5 py-0.5 rounded bg-pos-soft text-pos font-mono shrink-0">
                          git
                        </span>
                      )}
                    </div>

                    <div className="flex items-center gap-1 shrink-0">
                      <button
                        type="button"
                        onClick={(e) => {
                          e.stopPropagation();
                          void submitPath(entry.path);
                        }}
                        className="opacity-0 group-hover:opacity-100 focus:opacity-100 px-2 py-0.5 rounded-md bg-brand hover:bg-brand-hover text-[11px] text-white font-medium transition-opacity"
                      >
                        Open
                      </button>
                      <button
                        type="button"
                        title="Open folder contents"
                        onClick={(e) => {
                          e.stopPropagation();
                          void load(entry.path);
                        }}
                        className="p-1 rounded-md hover:bg-elevated text-ink-muted hover:text-ink transition-colors"
                      >
                        <ChevronRight className="w-3.5 h-3.5" />
                      </button>
                    </div>
                  </div>
                );
              })
            )}
          </div>
        </div>

        {/* Footer */}
        <div className="px-5 py-3 border-t border-line bg-elevated/50 flex items-center justify-between gap-3">
          <span className="text-[11px] font-mono text-ink-muted truncate">
            {effectiveTarget ? `Target: ${effectiveTarget}` : "Select a folder above or paste path"}
          </span>
          <div className="flex items-center gap-2 shrink-0">
            <button
              type="button"
              onClick={onClose}
              className="px-3.5 py-1.5 rounded-lg text-xs text-ink-soft hover:text-ink hover:bg-hover transition-colors"
            >
              Cancel
            </button>
            <button
              type="button"
              disabled={!effectiveTarget || submitting}
              onClick={() => void submitPath(effectiveTarget)}
              className="inline-flex items-center gap-1.5 px-4 py-1.5 rounded-lg bg-brand hover:bg-brand-hover disabled:opacity-40 text-xs font-medium text-white shadow-sm transition-colors"
            >
              {submitting && <Loader2 className="w-3.5 h-3.5 animate-spin" />}
              <span>{submitting ? "Opening..." : "Open Project"}</span>
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}
