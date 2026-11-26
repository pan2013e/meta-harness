import { useState } from "react";
import { Brain, Pin, Plus, Search, Trash2, X } from "lucide-react";
import { api } from "../../api/client";
import { useAppStore } from "../../store/useAppStore";

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

  const activeProject = projects.find((p) => p.id === activeProjectId) ?? null;

  return (
    <div
      className="fixed inset-0 z-50 bg-black/75 backdrop-blur-xs flex items-center justify-center p-4"
      onClick={onClose}
    >
      <div
        className="w-full max-w-3xl max-h-[85dvh] rounded-2xl border border-line-strong bg-panel shadow-2xl flex flex-col overflow-hidden"
        onClick={(e) => e.stopPropagation()}
      >
        <header className="px-5 py-4 border-b border-line bg-elevated/60 flex items-center justify-between gap-3">
          <div className="flex items-center gap-2.5">
            <div className="w-8 h-8 rounded-xl bg-brand-soft border border-brand/30 flex items-center justify-center text-brand-ink">
              <Brain className="w-4 h-4" />
            </div>
            <div>
              <h2 className="text-sm font-semibold text-ink">
                Project Memory{activeProject ? ` · ${activeProject.name}` : ""}
              </h2>
              <p className="text-[11px] text-ink-muted">
                Episodic ledger of who did what, plus pinned project rules shared across sessions
              </p>
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

        <div className="flex-1 overflow-y-auto p-5 space-y-5 text-xs">
          <div className="relative">
            <Search className="w-3.5 h-3.5 text-ink-muted absolute left-3 top-2.5" />
            <input
              type="text"
              value={memQuery}
              onChange={(e) => {
                setMemQuery(e.target.value);
                void refreshMemory(e.target.value);
              }}
              placeholder="Search completed jobs, harnesses, or touched files..."
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
                          onClick={async () => {
                            await api.pinJob(j.id, !j.pinned);
                            await refreshMemory(memQuery);
                          }}
                          className={`p-1 rounded hover:bg-hover ${
                            j.pinned ? "text-warn" : "text-ink-muted hover:text-ink"
                          }`}
                        >
                          <Pin className="w-3.5 h-3.5" />
                        </button>
                        <button
                          type="button"
                          title="Delete job entry"
                          onClick={async () => {
                            await api.deleteJob(j.id);
                            await refreshMemory(memQuery);
                          }}
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
                <div className="flex gap-1.5 text-xs">
                  <input
                    type="text"
                    value={newFact}
                    onChange={(e) => setNewFact(e.target.value)}
                    onKeyDown={(e) => {
                      if (e.key === "Enter" && newFact.trim()) {
                        e.preventDefault();
                        void api
                          .createFact(activeProjectId, factCategory, newFact.trim())
                          .then(() => {
                            setNewFact("");
                            return refreshMemory(memQuery);
                          });
                      }
                    }}
                    placeholder="Add a project convention, owner rule, or architectural note..."
                    className="flex-1 rounded-xl bg-inset border border-line focus:border-brand px-3 py-2 text-ink focus:outline-none"
                  />
                  <button
                    type="button"
                    onClick={async () => {
                      if (!newFact.trim()) return;
                      await api.createFact(activeProjectId, factCategory, newFact.trim());
                      setNewFact("");
                      await refreshMemory(memQuery);
                    }}
                    className="inline-flex items-center gap-1 px-3 py-2 rounded-xl bg-brand hover:bg-brand-hover text-white font-medium"
                  >
                    <Plus className="w-3.5 h-3.5" />
                    <span>Pin</span>
                  </button>
                </div>
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
                    <button
                      type="button"
                      onClick={async () => {
                        await api.deleteFact(activeProjectId, f.id);
                        await refreshMemory(memQuery);
                      }}
                      className="text-ink-muted hover:text-neg p-1"
                    >
                      <Trash2 className="w-3.5 h-3.5" />
                    </button>
                  )}
                </div>
              ))}
            </div>
          </section>
        </div>
      </div>
    </div>
  );
}
