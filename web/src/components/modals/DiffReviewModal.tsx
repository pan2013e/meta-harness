import { useMemo, useState } from "react";
import {
  ChevronDown,
  ExternalLink,
  FileCode2,
  FileImage,
  GitMerge,
  Loader2,
  X,
} from "lucide-react";
import { api } from "../../api/client";
import { SessionFileLink } from "../common/SessionFileLink";
import { useAppStore } from "../../store/useAppStore";
import type { PromoteMode } from "../../types/api";
import { DiffViewer, parseUnifiedDiff } from "../common/DiffViewer";

export function DiffReviewModal({ onClose }: { onClose: () => void }) {
  const {
    activeSession,
    sessionDiff,
    refreshSessionDiff,
    setPromoteBanner,
    setError,
  } = useAppStore();

  const [selectedFile, setSelectedFile] = useState<string | null>(null);
  const [promoteOpen, setPromoteOpen] = useState(false);
  const [promoting, setPromoting] = useState(false);

  const sections = useMemo(
    () => parseUnifiedDiff(sessionDiff?.unified_diff ?? ""),
    [sessionDiff?.unified_diff]
  );

  if (!activeSession) return null;

  const baseBranch = activeSession.base_branch ?? "main";
  const stat = sessionDiff?.diff_stat;
  const hasChanges =
    (stat?.files_changed ?? 0) > 0 || (sessionDiff?.ahead_commits ?? 0) > 0;

  const handlePromote = async (mode: PromoteMode) => {
    setPromoteOpen(false);
    setPromoting(true);
    try {
      const res = await api.promoteSession(activeSession.id, mode);
      setPromoteBanner(res.result.summary);
      await refreshSessionDiff();
      window.setTimeout(() => {
        if (useAppStore.getState().promoteBanner === res.result.summary) {
          useAppStore.getState().setPromoteBanner(null);
        }
      }, 5000);
      onClose();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setPromoting(false);
    }
  };

  return (
    <div
      className="fixed inset-0 z-50 bg-black/75 backdrop-blur-xs flex items-center justify-center p-4"
      onClick={onClose}
    >
      <div
        className="w-full max-w-5xl h-[84dvh] rounded-2xl border border-line-strong bg-panel shadow-2xl flex flex-col overflow-hidden"
        onClick={(e) => e.stopPropagation()}
      >
        {/* Header */}
        <header className="px-5 py-3.5 border-b border-line bg-elevated/60 flex flex-wrap items-center justify-between gap-3">
          <div className="flex items-center gap-3 min-w-0">
            <div className="w-8 h-8 rounded-xl bg-pos-soft border border-pos/30 flex items-center justify-center text-pos shrink-0">
              <GitMerge className="w-4 h-4" />
            </div>
            <div className="min-w-0">
              <h2 className="text-sm font-semibold text-ink truncate">
                Review Session Changes
              </h2>
              <div className="flex items-center gap-2 text-[11px] font-mono text-ink-muted truncate">
                <span>{sessionDiff?.integration_branch ?? activeSession.integration_branch}</span>
                {activeSession.project_id ? (
                  <>
                    <span>→</span>
                    <span className="text-ink">{baseBranch}</span>
                  </>
                ) : (
                  <span className="text-ink-muted">(standalone workspace)</span>
                )}
                {stat && stat.files_changed > 0 && (
                  <span>
                    · {stat.files_changed} files (
                    <span className="text-pos">+{stat.insertions}</span> /{" "}
                    <span className="text-neg">-{stat.deletions}</span>)
                  </span>
                )}
              </div>
            </div>
          </div>

          <div className="flex items-center gap-2 relative">
            {hasChanges && activeSession.project_id !== null && (
              <div className="relative">
                <button
                  type="button"
                  disabled={promoting}
                  onClick={() => setPromoteOpen((o) => !o)}
                  className="inline-flex items-center gap-1.5 px-3.5 py-2 rounded-xl bg-emerald-600 hover:bg-emerald-500 disabled:opacity-50 text-xs font-semibold text-white transition-colors"
                >
                  {promoting && <Loader2 className="w-3.5 h-3.5 animate-spin" />}
                  <span>Promote to {baseBranch}</span>
                  <ChevronDown className="w-3.5 h-3.5" />
                </button>

                {promoteOpen && (
                  <>
                    <div
                      className="fixed inset-0 z-20"
                      onClick={() => setPromoteOpen(false)}
                    />
                    <div className="absolute right-0 mt-1.5 w-64 rounded-xl border border-line-strong bg-panel shadow-2xl p-1 z-30 space-y-0.5 text-xs">
                      <button
                        type="button"
                        onClick={() => void handlePromote("merge")}
                        className="w-full px-3 py-2 rounded-lg text-left hover:bg-hover transition-colors"
                      >
                        <div className="font-medium text-ink">Fast-Forward / Merge</div>
                        <div className="text-[11px] text-ink-muted">
                          Preserve branch history cleanly
                        </div>
                      </button>
                      <button
                        type="button"
                        onClick={() => void handlePromote("squash")}
                        className="w-full px-3 py-2 rounded-lg text-left hover:bg-hover transition-colors"
                      >
                        <div className="font-medium text-ink">Squash Merge</div>
                        <div className="text-[11px] text-ink-muted">
                          Single clean commit on {baseBranch}
                        </div>
                      </button>
                      <button
                        type="button"
                        onClick={() => void handlePromote("patch")}
                        className="w-full px-3 py-2 rounded-lg text-left hover:bg-hover transition-colors"
                      >
                        <div className="font-medium text-ink">Apply Uncommitted Patch</div>
                        <div className="text-[11px] text-ink-muted">
                          Stage changes in checkout without committing
                        </div>
                      </button>
                    </div>
                  </>
                )}
              </div>
            )}

            <button
              type="button"
              aria-label="Close diff review"
              onClick={onClose}
              className="p-2 rounded-lg text-ink-muted hover:text-ink hover:bg-hover transition-colors"
            >
              <X className="w-4 h-4" />
            </button>
          </div>
        </header>

        {/* 2-Column Body: File Sidebar + Diff Pane */}
        <div className="flex-1 flex min-h-0">
          {sections.length > 0 && (
            <nav
              aria-label="Changed files"
              className="w-64 shrink-0 border-r border-line bg-inset/60 p-2.5 overflow-y-auto space-y-1 hidden sm:block"
            >
              <button
                type="button"
                onClick={() => setSelectedFile(null)}
                className={`w-full px-2.5 py-2 rounded-lg text-left text-xs flex items-center justify-between transition-colors ${
                  selectedFile === null
                    ? "bg-brand-soft text-ink font-medium"
                    : "text-ink-soft hover:bg-hover"
                }`}
              >
                <span>All changed files</span>
                <span className="font-mono text-[10.5px] text-ink-muted">
                  {sections.length}
                </span>
              </button>

              {sections.map((sec) => {
                const active = selectedFile === sec.filename;
                return (
                  <div
                    key={sec.filename}
                    className={`group flex items-center justify-between gap-1.5 rounded-lg px-2.5 py-2 text-xs transition-colors ${
                      active ? "bg-brand-soft text-ink" : "hover:bg-hover text-ink-soft"
                    }`}
                  >
                    <button
                      type="button"
                      onClick={() => setSelectedFile(sec.filename)}
                      className="flex items-center gap-2 min-w-0 flex-1 text-left"
                    >
                      {sec.binary ? (
                        <FileImage className="w-3.5 h-3.5 text-brand-ink shrink-0" />
                      ) : (
                        <FileCode2 className="w-3.5 h-3.5 text-brand-ink shrink-0" />
                      )}
                      <span className="truncate font-mono text-[11px]">
                        {sec.filename}
                      </span>
                    </button>

                    <div className="flex items-center gap-1.5 shrink-0 font-mono text-[10px]">
                      {sec.binary ? (
                        <span className="text-ink-muted">bin</span>
                      ) : (
                        <>
                          <span className="text-pos">+{sec.additions}</span>
                          <span className="text-neg">-{sec.deletions}</span>
                        </>
                      )}
                      <SessionFileLink
                        sessionId={activeSession.id}
                        path={sec.filename}
                        title={`Open ${sec.filename}`}
                        className="opacity-0 group-hover:opacity-100 p-0.5 text-ink-muted hover:text-brand-ink"
                      >
                        <ExternalLink className="w-3 h-3" />
                      </SessionFileLink>
                    </div>
                  </div>
                );
              })}
            </nav>
          )}

          <div className="flex-1 min-w-0 overflow-auto p-4 bg-app">
            <DiffViewer
              diffText={sessionDiff?.unified_diff ?? ""}
              selectedFile={selectedFile}
            />
          </div>
        </div>
      </div>
    </div>
  );
}
