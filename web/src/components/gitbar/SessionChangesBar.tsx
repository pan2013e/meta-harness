import { useState } from "react";
import { ChevronDown, Eye, GitMerge, Play, Terminal, X } from "lucide-react";
import { api } from "../../api/client";
import { useAppStore } from "../../store/useAppStore";
import type { PromoteMode } from "../../types/api";

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
    <div className="px-4 py-2 border-b border-zinc-800 bg-zinc-900/40 flex flex-wrap items-center justify-between gap-2 text-xs">
      <div className="flex items-center gap-2.5 min-w-0">
        <span className="inline-flex items-center gap-1 px-2 py-0.5 rounded bg-zinc-800 font-mono text-[11px] text-zinc-300">
          <GitMerge className="w-3 h-3 text-emerald-400" />
          <span>{branch}</span>
        </span>
        {stat && stat.files_changed > 0 ? (
          <span className="text-zinc-400 font-mono text-[11px]">
            {sessionDiff?.ahead_commits ?? 0} commits · {stat.files_changed} files (
            <span className="text-emerald-400">+{stat.insertions}</span> /{" "}
            <span className="text-rose-400">-{stat.deletions}</span>)
          </span>
        ) : (
          <span className="text-zinc-500 text-[11px]">Clean integration worktree</span>
        )}
        {statusNote && (
          <span className="text-[11px] text-emerald-400 bg-emerald-500/10 px-2 py-0.5 rounded">
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
          className="inline-flex items-center gap-1 px-2.5 py-1 rounded bg-zinc-800 hover:bg-zinc-700 text-zinc-200"
        >
          <Eye className="w-3.5 h-3.5" />
          <span>Diff</span>
        </button>

        <button
          type="button"
          onClick={() => setRunOpen(true)}
          className="inline-flex items-center gap-1 px-2.5 py-1 rounded bg-zinc-800 hover:bg-zinc-700 text-zinc-200"
        >
          <Terminal className="w-3.5 h-3.5" />
          <span>Preview / Shell</span>
        </button>

        <div className="relative">
          <button
            type="button"
            onClick={() => setPromoteOpen((o) => !o)}
            className="inline-flex items-center gap-1 px-2.5 py-1 rounded bg-emerald-600 hover:bg-emerald-500 text-white font-medium"
          >
            <span>Promote to {activeSession.base_branch ?? "main"}</span>
            <ChevronDown className="w-3.5 h-3.5" />
          </button>

          {promoteOpen && (
            <div className="absolute right-0 mt-1 w-56 rounded-lg border border-zinc-800 bg-zinc-900 shadow-xl py-1 z-30">
              <button
                type="button"
                onClick={() => void handlePromote("merge")}
                className="w-full px-3 py-2 text-left hover:bg-zinc-800"
              >
                <div className="font-medium text-zinc-100">Fast-Forward / Merge</div>
                <div className="text-[11px] text-zinc-400">
                  Preserve session branch history
                </div>
              </button>
              <button
                type="button"
                onClick={() => void handlePromote("squash")}
                className="w-full px-3 py-2 text-left hover:bg-zinc-800"
              >
                <div className="font-medium text-zinc-100">Squash Merge</div>
                <div className="text-[11px] text-zinc-400">
                  Single clean commit on base branch
                </div>
              </button>
              <button
                type="button"
                onClick={() => void handlePromote("patch")}
                className="w-full px-3 py-2 text-left hover:bg-zinc-800"
              >
                <div className="font-medium text-zinc-100">Apply Uncommitted Patch</div>
                <div className="text-[11px] text-zinc-400">
                  Stage/review changes without committing
                </div>
              </button>
            </div>
          )}
        </div>
      </div>

      {diffOpen && (
        <div className="fixed inset-0 z-50 bg-black/75 flex items-center justify-center p-4">
          <div className="w-full max-w-4xl max-h-[85vh] rounded-xl border border-zinc-800 bg-zinc-900 flex flex-col overflow-hidden">
            <div className="px-4 py-3 border-b border-zinc-800 flex items-center justify-between">
              <span className="font-semibold text-sm">
                Cumulative Session Diff ({branch})
              </span>
              <button type="button" onClick={() => setDiffOpen(false)}>
                <X className="w-4 h-4 text-zinc-400" />
              </button>
            </div>
            <pre className="p-4 flex-1 overflow-auto font-mono text-xs text-zinc-200 bg-zinc-950 whitespace-pre">
              {sessionDiff?.unified_diff || "No changes yet on integration branch."}
            </pre>
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
    <div className="fixed inset-0 z-50 bg-black/75 flex items-center justify-center p-4">
      <div className="w-full max-w-2xl rounded-xl border border-zinc-800 bg-zinc-900 p-4 space-y-3">
        <div className="flex items-center justify-between">
          <div>
            <h3 className="text-sm font-semibold">Run / Preview in Session Worktree</h3>
            <p className="text-[11px] font-mono text-zinc-500 truncate">{cwd}</p>
          </div>
          <button type="button" onClick={onClose}>
            <X className="w-4 h-4 text-zinc-400" />
          </button>
        </div>

        <div className="flex gap-2">
          <input
            type="text"
            value={command}
            onChange={(e) => setCommand(e.target.value)}
            placeholder="cargo test / npm test / pytest"
            className="flex-1 rounded bg-zinc-950 border border-zinc-800 px-3 py-1.5 font-mono text-xs"
          />
          <button
            type="button"
            disabled={running}
            onClick={() => void runCmd()}
            className="inline-flex items-center gap-1 px-3 py-1.5 rounded bg-indigo-600 hover:bg-indigo-500 text-xs text-white"
          >
            <Play className="w-3.5 h-3.5" />
            <span>{running ? "Running..." : "Run"}</span>
          </button>
          <button
            type="button"
            onClick={() => void api.openSessionTerminal(sessionId)}
            className="inline-flex items-center gap-1 px-3 py-1.5 rounded bg-zinc-800 hover:bg-zinc-700 text-xs"
          >
            <Terminal className="w-3.5 h-3.5" />
            <span>Open OS Shell</span>
          </button>
        </div>

        <pre className="h-64 overflow-auto rounded bg-zinc-950 border border-zinc-800 p-3 font-mono text-xs text-zinc-200 whitespace-pre-wrap">
          {output || "Enter a test/build command to verify integrated changes before promoting."}
        </pre>
      </div>
    </div>
  );
}
