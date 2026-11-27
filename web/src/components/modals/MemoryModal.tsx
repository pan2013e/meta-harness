import { reportInAppError } from "../../inAppNotifications";
import { useEffect, useState } from "react";
import { Brain, Pencil, Pin, Plus, Search, Trash2, X } from "lucide-react";
import { api } from "../../api/client";
import { useAppStore } from "../../store/useAppStore";
import type { ProjectFact } from "../../types/api";

export function MemoryModal({ onClose }: { onClose: () => void }) {
  const {
    projects,
    activeProjectId,
    memoryJobs,
    memoryFacts,
    refreshMemory,
  } = useAppStore();

  const [memQuery, setMemQuery] = useState("");
  const [newFact, setNewFact] = useState("");
  const [factCategory, setFactCategory] = useState("architecture");
  const [editingFact, setEditingFact] = useState<ProjectFact | null>(null);
  const [savingFact, setSavingFact] = useState(false);

  useEffect(() => {
    setEditingFact(null);
    setNewFact("");
  }, [activeProjectId]);

  const saveFact = async () => {
    const summary = newFact.trim();
    if (!activeProjectId || !summary || savingFact || (editingFact && editingFact.project_id !== activeProjectId)) return;
    setSavingFact(true);
    try {
      if (editingFact) await api.reviseFact(editingFact, factCategory, summary);
      else await api.createFact(activeProjectId, factCategory, summary);
      if (activeProjectId === useAppStore.getState().activeProjectId) {
        setNewFact("");
        setEditingFact(null);
        await refreshMemory(memQuery);
      }
    } catch (error) {
      reportInAppError(error);
    } finally {
      setSavingFact(false);
    }
  };

  const activeProject = projects.find((p) => p.id === activeProjectId) ?? null;

  return (
    <div
      className="fixed inset-0 z-50 bg-black/75 backdrop-blur-xs flex items-center justify-center p-4"
      onClick={onClose}
    >
      <div
        role="dialog"
        aria-modal="true"
        aria-labelledby="project-memory-title"
        className="w-full max-w-3xl max-h-[85dvh] rounded-2xl border border-line-strong bg-panel shadow-2xl flex flex-col overflow-hidden"
        onClick={(e) => e.stopPropagation()}
      >
        <header className="px-5 py-4 border-b border-line bg-elevated/60 flex items-center justify-between gap-3">
          <div className="flex items-center gap-2.5">
            <div className="w-8 h-8 rounded-xl bg-brand-soft border border-brand/30 flex items-center justify-center text-brand-ink">
              <Brain className="w-4 h-4" />
            </div>
            <div>
              <h2 id="project-memory-title" className="text-sm font-semibold text-ink">
                Project Memory{activeProject ? ` · ${activeProject.name}` : ""}
              </h2>
              {activeProjectId && <p className="text-[11px] text-ink-muted">
                Episodic ledger of who did what, plus pinned project rules shared across sessions
              </p>}
            </div>
          </div>
          <button
            type="button"
            aria-label="Close memory"
            onClick={onClose}
            className="p-2 rounded-lg text-ink-muted hover:text-ink hover:bg-hover transition-colors"
          >
            <X className="w-4 h-4" />
          </button>
        </header>

        {activeProjectId ? <div className="flex-1 overflow-y-auto p-5 space-y-5 text-xs">
          <div className="relative">
            <Search className="w-3.5 h-3.5 text-ink-muted absolute left-3 top-2.5" />
            <input
              type="text"
              value={memQuery}
              onChange={(e) => {
                setMemQuery(e.target.value);
                void refreshMemory(e.target.value);
              }}
              placeholder="Search jobs and project facts"
              className="w-full rounded-xl bg-inset border border-line focus:border-brand pl-9 pr-3 py-2 text-xs text-ink placeholder-ink-muted focus:outline-none"
            />
          </div>

          {/* Completed Episodic Jobs */}
          <section className="space-y-2.5">
            <div className="flex items-center justify-between">
              <h3 className="text-[11px] uppercase tracking-wider text-ink-muted font-semibold">
                Recent Jobs (&ldquo;Who Did What&rdquo;)
              </h3>
              <span className="text-[11px] text-ink-muted font-mono">
                {memoryJobs.length} recorded
              </span>
            </div>

            {memoryJobs.length === 0 ? (
              <div className="rounded-xl border border-line bg-inset/60 p-4 text-center text-ink-muted">
                No curated jobs yet. Complete a coding task with any harness to record its outcome.
              </div>
            ) : (
              <div className="space-y-2">
                {memoryJobs.map((j) => (
                  <div
                    key={j.id}
                    className="rounded-xl border border-line bg-inset p-3.5 space-y-2"
                  >
                    <div className="flex items-center justify-between gap-2">
                      <div className="flex items-center gap-2 flex-wrap">
                        <span className="font-semibold text-brand-ink">
                          @{j.member_handle}
                        </span>
                        {j.model && (
                          <span className="font-mono text-[10.5px] px-1.5 py-0.2 rounded bg-elevated border border-line text-ink-soft">
                            {j.model}
                          </span>
                        )}
                        <span className="text-[10.5px] text-ink-muted">
                          curated by {j.curated_by}
                        </span>
                      </div>
                      <div className="flex items-center gap-1.5 shrink-0">
                        <span className="text-[10px] font-mono text-ink-muted">
                          {new Date(j.created_at).toLocaleTimeString([], {
                            hour: "2-digit",
                            minute: "2-digit",
                          })}
                        </span>
                        <button
                          type="button"
                          title={j.pinned ? "Unpin job" : "Pin job"}
                          onClick={() => void api.pinJob(j.id, !j.pinned)
                            .then(() => refreshMemory(memQuery)).catch(reportInAppError)}
                          className={`p-1 rounded hover:bg-hover ${
                            j.pinned ? "text-warn" : "text-ink-muted hover:text-ink"
                          }`}
                        >
                          <Pin className="w-3.5 h-3.5" />
                        </button>
                        <button
                          type="button"
                          title="Delete job entry"
                          onClick={() => void api.deleteJob(j.id)
                            .then(() => refreshMemory(memQuery)).catch(reportInAppError)}
                          className="p-1 rounded text-ink-muted hover:text-neg hover:bg-hover"
                        >
                          <Trash2 className="w-3.5 h-3.5" />
                        </button>
                      </div>
                    </div>

                    <p className="text-ink leading-relaxed">{j.curated_summary}</p>

                    {j.touched_modules.length > 0 && (
                      <div className="flex flex-wrap gap-1">
                        {j.touched_modules.map((mod) => (
                          <span
                            key={mod}
                            className="px-1.5 py-0.5 rounded bg-elevated border border-line font-mono text-[10.5px] text-ink-soft"
                          >
                            {mod}
                          </span>
                        ))}
                      </div>
                    )}
                  </div>
                ))}
              </div>
            )}
          </section>

          {/* Pinned Facts */}
          <section className="space-y-2.5 pt-3 border-t border-line">
            <h3 className="text-[11px] uppercase tracking-wider text-ink-muted font-semibold">
              Pinned Architecture & Rules
            </h3>

            {activeProjectId && (
              <div className="space-y-2">
                <div className="flex flex-wrap gap-1">
                  {[
                    ["architecture", "Arch"],
                    ["ownership", "Owner"],
                    ["harness_affinity", "Harness"],
                    ["convention", "Rule"],
                  ].map(([val, lbl]) => (
                    <button
                      key={val}
                      type="button"
                      onClick={() => setFactCategory(val)}
                      className={`px-2 py-0.5 rounded-md border text-[11px] transition-colors ${
                        factCategory === val
                          ? "bg-brand-soft border-brand text-brand-ink font-medium"
                          : "bg-inset border-line text-ink-muted hover:text-ink"
                      }`}
                    >
                      {lbl}
                    </button>
                  ))}
                </div>
                <form className="flex gap-1.5 text-xs" onSubmit={(event) => { event.preventDefault(); void saveFact(); }}>
                  <input
                    type="text"
                    aria-label={editingFact ? "Edit project fact" : "New project fact"}
                    value={newFact}
                    disabled={savingFact}
                    onChange={(e) => setNewFact(e.target.value)}
                    placeholder="Add a project convention, owner rule, or architectural note..."
                    className="flex-1 rounded-xl bg-inset border border-line focus:border-brand px-3 py-2 text-ink focus:outline-none"
                  />
                  <button
                    type="submit"
                    disabled={savingFact || !newFact.trim()}
                    className="inline-flex items-center gap-1 px-3 py-2 rounded-xl bg-brand hover:bg-brand-hover text-white font-medium"
                  >
                    {!editingFact && <Plus className="w-3.5 h-3.5" />}
                    <span>{editingFact ? "Save" : "Pin"}</span>
                  </button>
                  {editingFact && <button type="button" disabled={savingFact}
                    onClick={() => { setEditingFact(null); setNewFact(""); }}
                    className="rounded-lg border border-line px-2">Cancel</button>}
                </form>
              </div>
            )}

            <div className="space-y-1.5">
              {memoryFacts.map((f) => (
                <div
                  key={f.id}
                  className="rounded-xl border border-line bg-inset p-3 flex items-start justify-between gap-2"
                >
                  <div className="space-y-1">
                    <span className="text-[10px] uppercase px-1.5 py-0.5 rounded bg-brand-soft text-brand-ink font-mono mr-2">
                      {f.category}
                    </span>
                    <span className="text-ink">{f.summary}</span>
                  </div>
                  {activeProjectId && (
                    <div className="flex shrink-0 gap-1">
                    <button type="button" aria-label="Edit fact" disabled={savingFact}
                      onClick={() => { setEditingFact(f); setFactCategory(f.category); setNewFact(f.summary); }}
                      className="p-1 text-ink-muted hover:text-ink"><Pencil className="h-3.5 w-3.5" /></button>
                    <button
                      type="button"
                      aria-label="Delete fact"
                      disabled={savingFact}
                      onClick={async () => {
                        try {
                          await api.deleteFact(activeProjectId, f.id);
                          if (editingFact?.id === f.id) { setEditingFact(null); setNewFact(""); }
                          await refreshMemory(memQuery);
                        } catch (error) {
                          reportInAppError(error);
                        }
                      }}
                      className="text-ink-muted hover:text-neg p-1"
                    >
                      <Trash2 className="w-3.5 h-3.5" />
                    </button>
                    </div>
                  )}
                </div>
              ))}
            </div>
          </section>
        </div> : (
          <p className="p-5 text-sm text-ink-muted">
            Open a project to view its saved jobs and pinned notes.
          </p>
        )}
      </div>
    </div>
  );
}
