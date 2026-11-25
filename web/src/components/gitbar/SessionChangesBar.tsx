import { useState } from "react";
import { ChevronDown, Eye, GitMerge, Play, Terminal, X } from "lucide-react";
import { api } from "../../api/client";
import { useAppStore } from "../../store/useAppStore";
import type { PromoteMode } from "../../types/api";
import { DiffViewer } from "../common/DiffViewer";

export function SessionChangesBar() {
  const { activeSession, sessionDiff, refreshSessionDiff, setError } = useAppStore();
  const [diffOpen, setDiffOpen] = useState(false);
  const [runOpen, setRunOpen] = useState(false);
  const [promoteOpen, setPromoteOpen] = useState(false);
  const [statusNote, setStatusNote] = useState<string | null>(null);

  if (!activeSession || activeSession.isolation_mode !== "worktree_per_member") {
    return null;
  }

  const stat = sessionDiff?.diff_stat;
  const branch = activeSession.integration_branch ?? "mh/integration";

  const handlePromote = async (mode: PromoteMode) => {
    setPromoteOpen(false);
    try {
      const res = await api.promoteSession(activeSession.id, mode);
      setStatusNote(res.result.summary);
      await refreshSessionDiff();
      window.setTimeout(() => setStatusNote(null), 5000);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  return (
    <div className="px-4 py-2 border-b border-line bg-panel/50 flex flex-wrap items-center justify-between gap-2 text-xs">
      <div className="flex items-center gap-2.5 min-w-0">
        <span className="inline-flex items-center gap-1.5 px-2.5 py-0.5 rounded-lg bg-elevated border border-line font-mono text-[11px] text-ink">
          <GitMerge className="w-3.5 h-3.5 text-pos" />
          <span>{branch}</span>
        </span>
        {stat && stat.files_changed > 0 ? (
          <span className="text-ink-soft font-mono text-[11px]">
            {sessionDiff?.ahead_commits ?? 0} commits · {stat.files_changed} files (
            <span className="text-pos">+{stat.insertions}</span> /{" "}
            <span className="text-neg">-{stat.deletions}</span>)
          </span>
        ) : (
          <span className="text-ink-muted text-[11px]">Clean integration worktree</span>
        )}
        {statusNote && (
          <span className="text-[11px] text-pos bg-pos-soft border border-pos/30 px-2 py-0.5 rounded-md">
            {statusNote}
          </span>
        )}
      </div>

      <div className="flex items-center gap-2 relative">
        <button
          type="button"
          onClick={() => {
            void refreshSessionDiff();
            setDiffOpen(true);
          }}
          className="inline-flex items-center gap-1.5 px-2.5 py-1 rounded-lg bg-elevated hover:bg-hover border border-line text-ink-soft hover:text-ink transition-colors"
        >
          <Eye className="w-3.5 h-3.5" />
          <span>Diff</span>
        </button>

        <button
          type="button"
          onClick={() => setRunOpen(true)}
          className="inline-flex items-center gap-1.5 px-2.5 py-1 rounded-lg bg-elevated hover:bg-hover border border-line text-ink-soft hover:text-ink transition-colors"
        >
          <Terminal className="w-3.5 h-3.5" />
          <span>Preview / Shell</span>
        </button>

        <div className="relative">
          <button
            type="button"
            onClick={() => setPromoteOpen((o) => !o)}
            className="inline-flex items-center gap-1.5 px-3 py-1 rounded-lg bg-emerald-600 hover:bg-emerald-500 text-white font-medium transition-colors"
          >
            <span>Promote to {activeSession.base_branch ?? "main"}</span>
            <ChevronDown className="w-3.5 h-3.5" />
          </button>

          {promoteOpen && (
            <>
              <div
                className="fixed inset-0 z-20"
                onClick={() => setPromoteOpen(false)}
              />
              <div className="absolute right-0 mt-1.5 w-60 rounded-xl border border-line-strong bg-panel shadow-2xl p-1 z-30 space-y-0.5">
                <button
                  type="button"
                  onClick={() => void handlePromote("merge")}
                  className="w-full px-3 py-2 rounded-lg text-left hover:bg-hover transition-colors"
                >
                  <div className="font-medium text-ink">Fast-Forward / Merge</div>
                  <div className="text-[11px] text-ink-muted">
                    Preserve session branch history
                  </div>
                </button>
                <button
                  type="button"
                  onClick={() => void handlePromote("squash")}
                  className="w-full px-3 py-2 rounded-lg text-left hover:bg-hover transition-colors"
                >
                  <div className="font-medium text-ink">Squash Merge</div>
                  <div className="text-[11px] text-ink-muted">
                    Single clean commit on base branch
                  </div>
                </button>
                <button
                  type="button"
                  onClick={() => void handlePromote("patch")}
                  className="w-full px-3 py-2 rounded-lg text-left hover:bg-hover transition-colors"
                >
                  <div className="font-medium text-ink">Apply Uncommitted Patch</div>
                  <div className="text-[11px] text-ink-muted">
                    Stage/review changes without committing
                  </div>
                </button>
              </div>
            </>
          )}
        </div>
      </div>

      {diffOpen && (
        <div className="fixed inset-0 z-50 bg-black/75 backdrop-blur-xs flex items-center justify-center p-4">
          <div className="w-full max-w-4xl max-h-[85vh] rounded-2xl border border-line-strong bg-panel shadow-2xl flex flex-col overflow-hidden">
            <div className="px-5 py-3 border-b border-line bg-elevated/60 flex items-center justify-between">
              <div className="flex items-center gap-2">
                <GitMerge className="w-4 h-4 text-pos" />
                <span className="font-semibold text-sm text-ink">
                  Cumulative Session Diff ({branch})
                </span>
              </div>
              <button
                type="button"
                onClick={() => setDiffOpen(false)}
                className="p-1 rounded-lg text-ink-muted hover:text-ink hover:bg-hover"
              >
                <X className="w-4 h-4" />
              </button>
            </div>
            <div className="p-4 flex-1 overflow-auto bg-app">
              <DiffViewer diffText={sessionDiff?.unified_diff ?? ""} />
            </div>
          </div>
        </div>
      )}

      {runOpen && (
        <PreviewRunnerModal
          sessionId={activeSession.id}
          cwd={activeSession.integration_wt_path ?? ""}
          onClose={() => setRunOpen(false)}
        />
      )}
    </div>
  );
}

function PreviewRunnerModal({
  sessionId,
  cwd,
  onClose,
}: {
  sessionId: string;
  cwd: string;
  onClose: () => void;
}) {
  const [command, setCommand] = useState("git status");
  const [output, setOutput] = useState<string>("");
  const [running, setRunning] = useState(false);

  const runCmd = async () => {
    setRunning(true);
    try {
      const res = await api.runSessionCommand(sessionId, command);
      setOutput(
        `$ ${command} (exit ${res.exit_code})\n${res.stdout}${
          res.stderr ? `\n[stderr]\n${res.stderr}` : ""
        }`
      );
    } catch (e) {
      setOutput(e instanceof Error ? e.message : String(e));
    } finally {
      setRunning(false);
    }
  };

  return (
    <div className="fixed inset-0 z-50 bg-black/75 backdrop-blur-xs flex items-center justify-center p-4">
      <div className="w-full max-w-2xl rounded-2xl border border-line-strong bg-panel shadow-2xl p-5 space-y-3">
        <div className="flex items-center justify-between">
          <div>
            <h3 className="text-sm font-semibold text-ink">
              Run / Preview in Session Worktree
            </h3>
            <p className="text-[11px] font-mono text-ink-muted truncate">{cwd}</p>
          </div>
          <button
            type="button"
            onClick={onClose}
            className="p-1 rounded-lg text-ink-muted hover:text-ink hover:bg-hover"
          >
            <X className="w-4 h-4" />
          </button>
        </div>

        <div className="flex gap-2">
          <input
            type="text"
            value={command}
            onChange={(e) => setCommand(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") {
                e.preventDefault();
                void runCmd();
              }
            }}
            placeholder="cargo test / npm test / pytest"
            className="flex-1 rounded-xl bg-inset border border-line focus:border-brand px-3 py-2 font-mono text-xs text-ink focus:outline-none"
          />
          <button
            type="button"
            disabled={running}
            onClick={() => void runCmd()}
            className="inline-flex items-center gap-1.5 px-3.5 py-2 rounded-xl bg-brand hover:bg-brand-hover text-xs font-medium text-white"
          >
            <Play className="w-3.5 h-3.5" />
            <span>{running ? "Running..." : "Run"}</span>
          </button>
          <button
            type="button"
            onClick={() => void api.openSessionTerminal(sessionId)}
            className="inline-flex items-center gap-1.5 px-3 py-2 rounded-xl bg-elevated hover:bg-hover border border-line text-xs text-ink"
          >
            <Terminal className="w-3.5 h-3.5" />
            <span>OS Shell</span>
          </button>
        </div>

        <pre className="h-64 overflow-auto rounded-xl bg-inset border border-line p-3 font-mono text-xs text-ink-soft whitespace-pre-wrap">
          {output ||
            "Enter a test/build command to verify integrated changes before promoting."}
        </pre>
      </div>
    </div>
  );
}
