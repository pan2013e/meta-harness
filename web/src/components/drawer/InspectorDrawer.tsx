import { useState } from "react";
import {
  Check,
  ChevronDown,
  Pin,
  Plus,
  RefreshCw,
  Search,
  Trash2,
  X,
} from "lucide-react";
import { api } from "../../api/client";
import { useAppStore } from "../../store/useAppStore";
import type { PermissionPolicy } from "../../types/api";
import { HarnessAvatar, QuotaSnapshotSummary } from "../common/Badges";
import { DiffViewer } from "../common/DiffViewer";

const PLAN_OPTIONS = [
  { value: "plus", label: "Plus" },
  { value: "pro", label: "Pro" },
  { value: "max_5x", label: "Max 5x" },
  { value: "max_20x", label: "Max 20x" },
];

const PERMISSION_OPTIONS: Array<{
  value: PermissionPolicy;
  label: string;
  desc: string;
}> = [
  {
    value: "worktree_smart",
    label: "Worktree-Smart",
    desc: "Auto-allow reads/writes inside worktree; prompt shell commands",
  },
  {
    value: "strict",
    label: "Strict",
    desc: "Prompt all file edits and shell executions",
  },
  {
    value: "autonomous_sandbox",
    label: "Autonomous Sandbox",
    desc: "Auto-allow local worktree & non-destructive tools",
  },
];

export function InspectorDrawer() {
  const {
    drawerOpen,
    drawerTab,
    openDrawerTab,
    closeDrawer,
    trajectories,
    inspectedTurnId,
    inspectedTrajectory,
    inspectedEvents,
    inspectTurn,
    activeProjectId,
    activeSession,
    memoryJobs,
    memoryFacts,
    refreshMemory,
    probes,
    quotas,
    quotasRefreshing,
    refreshQuotas,
    refreshQuotasLive,
    permissionRules,
    selectSession,
  } = useAppStore();

  const [memQuery, setMemQuery] = useState("");
  const [newFact, setNewFact] = useState("");
  const [factCategory, setFactCategory] = useState("architecture");
  const [turnMenuOpen, setTurnMenuOpen] = useState(false);

  if (!drawerOpen) return null;

  const selectedTurn = trajectories.find((t) => t.turn_id === inspectedTurnId) ?? null;

  return (
    <aside className="w-[430px] shrink-0 border-l border-line bg-panel flex flex-col h-full">
      {/* Tabs Header */}
      <div className="px-3 py-2.5 border-b border-line flex items-center justify-between bg-elevated/40">
        <div className="flex gap-1 text-xs">
          {(["trajectory", "memory", "settings"] as const).map((t) => (
            <button
              key={t}
              type="button"
              onClick={() => openDrawerTab(t)}
              className={`px-3 py-1.5 rounded-lg capitalize transition-colors ${
                drawerTab === t
                  ? "bg-brand text-white font-medium shadow-xs"
                  : "text-ink-soft hover:text-ink hover:bg-hover"
              }`}
            >
              {t === "settings" ? "Switch & Quotas" : t}
            </button>
          ))}
        </div>
        <button
          type="button"
          onClick={closeDrawer}
          className="text-ink-muted hover:text-ink p-1.5 rounded-lg hover:bg-hover transition-colors"
        >
          <X className="w-4 h-4" />
        </button>
      </div>

      {/* Trajectories Tab */}
      {drawerTab === "trajectory" && (
        <div className="flex-1 flex flex-col min-h-0 p-3 space-y-3">
          {/* Custom Turn Picker */}
          <div className="relative">
            <button
              type="button"
              onClick={() => setTurnMenuOpen((o) => !o)}
              className="w-full flex items-center justify-between gap-2 rounded-xl bg-inset hover:bg-elevated border border-line px-3 py-2 text-xs text-ink transition-colors"
            >
              <span className="truncate font-mono">
                {selectedTurn
                  ? `@${selectedTurn.member_handle} · ${selectedTurn.model ?? "default"} · ${selectedTurn.status}`
                  : "Select a turn trajectory..."}
              </span>
              <ChevronDown className="w-3.5 h-3.5 text-ink-muted shrink-0" />
            </button>

            {turnMenuOpen && (
              <>
                <div
                  className="fixed inset-0 z-20"
                  onClick={() => setTurnMenuOpen(false)}
                />
                <div className="absolute left-0 right-0 mt-1 rounded-xl border border-line-strong bg-panel shadow-2xl p-1 z-30 max-h-60 overflow-y-auto space-y-0.5">
                  {trajectories.length === 0 ? (
                    <div className="px-3 py-2 text-xs text-ink-muted">
                      No recorded turns yet.
                    </div>
                  ) : (
                    trajectories.map((t) => {
                      const active = t.turn_id === inspectedTurnId;
                      return (
                        <button
                          key={t.turn_id}
                          type="button"
                          onClick={() => {
                            setTurnMenuOpen(false);
                            void inspectTurn(t.turn_id);
                          }}
                          className={`w-full flex items-center justify-between px-2.5 py-2 rounded-lg text-left text-xs transition-colors ${
                            active ? "bg-brand-soft text-ink" : "hover:bg-hover text-ink-soft"
                          }`}
                        >
                          <span className="font-mono truncate">
                            @{t.member_handle} · {t.model ?? "default"} ({t.status})
                          </span>
                          <span className="text-[10px] text-ink-muted shrink-0">
                            {new Date(t.started_at).toLocaleTimeString()}
                          </span>
                        </button>
                      );
                    })
                  )}
                </div>
              </>
            )}
          </div>

          {inspectedTrajectory && (
            <div className="rounded-xl border border-line bg-inset p-3 text-xs space-y-1.5">
              <div className="flex items-center justify-between">
                <span className="font-semibold text-brand-ink">
                  @{inspectedTrajectory.member_handle}
                </span>
                <span className="font-mono text-[10.5px] px-2 py-0.5 rounded bg-elevated border border-line text-ink-muted">
                  {inspectedTrajectory.worktree_branch ?? "shared"}
                </span>
              </div>
              <div className="text-ink-soft font-mono text-[11px]">
                {inspectedTrajectory.model ?? "auto"} · {inspectedTrajectory.effort ?? "auto"} ·{" "}
                {inspectedTrajectory.status}
              </div>
              {inspectedTrajectory.touched_files.length > 0 && (
                <div className="text-[11px] text-pos font-mono truncate">
                  Files: {inspectedTrajectory.touched_files.join(", ")}
                </div>
              )}
            </div>
          )}

          <div className="flex-1 overflow-y-auto space-y-2 pr-1">
            {inspectedEvents.map((ev) => (
              <div
                key={ev.id}
                className="rounded-xl border border-line bg-inset p-3 text-xs space-y-1.5"
              >
                <div className="flex items-center justify-between text-[10px] uppercase tracking-wider font-semibold text-ink-muted">
                  <span>{ev.event_kind}</span>
                  <span className="font-mono">
                    {new Date(ev.created_at).toLocaleTimeString()}
                  </span>
                </div>
                {ev.event_kind === "diff" && typeof ev.payload.diff === "string" ? (
                  <DiffViewer diffText={ev.payload.diff} />
                ) : ev.event_kind === "thought" && typeof ev.payload.text === "string" ? (
                  <p className="text-ink-soft italic whitespace-pre-wrap leading-relaxed">
                    {ev.payload.text}
                  </p>
                ) : (
                  <pre className="font-mono text-[11px] text-ink-soft overflow-x-auto whitespace-pre-wrap">
                    {JSON.stringify(ev.payload, null, 2)}
                  </pre>
                )}
              </div>
            ))}
            {inspectedEvents.length === 0 && (
              <p className="text-xs text-ink-muted text-center py-8">
                Select a turn above to inspect thoughts, tool calls, and diffs.
              </p>
            )}
          </div>
        </div>
      )}

      {/* Memory Tab */}
      {drawerTab === "memory" && (
        <div className="flex-1 flex flex-col min-h-0 p-3 space-y-4 overflow-y-auto">
          <div className="relative">
            <Search className="w-3.5 h-3.5 text-ink-muted absolute left-3 top-2.5" />
            <input
              type="text"
              value={memQuery}
              onChange={(e) => {
                setMemQuery(e.target.value);
                void refreshMemory(e.target.value);
              }}
              placeholder="Search project jobs & modules (FTS5)..."
              className="w-full rounded-xl bg-inset border border-line focus:border-brand pl-9 pr-3 py-2 text-xs text-ink placeholder-ink-muted focus:outline-none"
            />
          </div>

          {/* Pinned Architecture & Notes */}
          <div className="space-y-2">
            <h3 className="text-[10.5px] uppercase tracking-wider text-ink-muted font-semibold">
              Pinned Architecture & Notes
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
                    placeholder="Add high-level project fact..."
                    className="flex-1 rounded-xl bg-inset border border-line focus:border-brand px-3 py-1.5 text-ink focus:outline-none"
                  />
                  <button
                    type="button"
                    onClick={async () => {
                      if (!newFact.trim()) return;
                      await api.createFact(activeProjectId, factCategory, newFact.trim());
                      setNewFact("");
                      await refreshMemory(memQuery);
                    }}
                    className="px-2.5 py-1.5 rounded-xl bg-brand hover:bg-brand-hover text-white"
                  >
                    <Plus className="w-3.5 h-3.5" />
                  </button>
                </div>
              </div>
            )}

            {memoryFacts.map((f) => (
              <div
                key={f.id}
                className="rounded-xl border border-line bg-inset p-2.5 text-xs flex items-start justify-between gap-2"
              >
                <div className="space-y-1">
                  <span className="text-[10px] uppercase px-1.5 py-0.5 rounded bg-brand-soft text-brand-ink font-mono mr-1.5">
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
                    className="text-ink-muted hover:text-neg"
                  >
                    <Trash2 className="w-3.5 h-3.5" />
                  </button>
                )}
              </div>
            ))}
          </div>

          {/* Curated Episodic Jobs */}
          <div className="space-y-2 pt-2 border-t border-line">
            <h3 className="text-[10.5px] uppercase tracking-wider text-ink-muted font-semibold">
              Curated Episodic Jobs (&ldquo;Who Did What&rdquo;)
            </h3>
            {memoryJobs.map((j) => (
              <div
                key={j.id}
                className="rounded-xl border border-line bg-inset p-3 text-xs space-y-1.5"
              >
                <div className="flex items-center justify-between">
                  <span className="font-semibold text-brand-ink">
                    @{j.member_handle}{" "}
                    <span className="text-ink-muted font-mono font-normal">
                      ({j.model ?? "default"})
                    </span>
                  </span>
                  <div className="flex items-center gap-1.5">
                    <button
                      type="button"
                      onClick={async () => {
                        await api.pinJob(j.id, !j.pinned);
                        await refreshMemory(memQuery);
                      }}
                      className={
                        j.pinned ? "text-warn" : "text-ink-muted hover:text-ink"
                      }
                    >
                      <Pin className="w-3.5 h-3.5" />
                    </button>
                    <button
                      type="button"
                      onClick={async () => {
                        await api.deleteJob(j.id);
                        await refreshMemory(memQuery);
                      }}
                      className="text-ink-muted hover:text-neg"
                    >
                      <Trash2 className="w-3.5 h-3.5" />
                    </button>
                  </div>
                </div>
                <p className="text-ink">{j.curated_summary}</p>
                {j.touched_modules.length > 0 && (
                  <div className="text-[10.5px] font-mono text-ink-muted truncate">
                    {j.touched_modules.join(", ")}
                  </div>
                )}
              </div>
            ))}
          </div>
        </div>
      )}

      {/* Switch & Quotas Tab */}
      {drawerTab === "settings" && (
        <div className="flex-1 overflow-y-auto p-3.5 space-y-5 text-xs">
          <div className="space-y-2.5">
            <div className="flex items-center justify-between">
              <h3 className="text-[10.5px] uppercase tracking-wider text-ink-muted font-semibold">
                Harnesses & Live Subscription Quotas
              </h3>
              <button
                type="button"
                disabled={quotasRefreshing}
                onClick={() => void refreshQuotasLive()}
                className="inline-flex items-center gap-1 px-2.5 py-1 rounded-lg bg-brand-soft hover:bg-brand/25 text-[11px] font-medium text-brand-ink transition-colors disabled:opacity-50"
              >
                <RefreshCw
                  className={`w-3 h-3 ${quotasRefreshing ? "animate-spin" : ""}`}
                />
                <span>Refresh APIs</span>
              </button>
            </div>

            {probes.map((p) => {
              const q = quotas[p.adapter_id];
              return (
                <div
                  key={p.adapter_id}
                  className="rounded-xl border border-line bg-inset p-3 space-y-2.5"
                >
                  <div className="flex items-center justify-between gap-2">
                    <div className="flex items-center gap-2 min-w-0">
                      <HarnessAvatar
                        handle={p.adapter_id}
                        accentColor={p.accent_color}
                        size="sm"
                      />
                      <div className="min-w-0">
                        <div className="font-semibold text-ink truncate">
                          {p.display_name}
                        </div>
                        <div className="text-[10px] font-mono text-ink-muted truncate">
                          {p.resolved_command ?? "CLI not detected"}
                        </div>
                      </div>
                    </div>
                    <span
                      className={`text-[10px] px-2 py-0.5 rounded-md font-mono shrink-0 ${
                        p.installed
                          ? "bg-pos-soft text-pos"
                          : "bg-elevated text-ink-muted"
                      }`}
                    >
                      {p.installed ? "CLI Found" : "Missing"}
                    </span>
                  </div>

                  <p className="text-[11px] text-ink-muted">{p.auth_hint}</p>

                  {q && (
                    <div className="space-y-2 pt-1 border-t border-line">
                      <QuotaSnapshotSummary quota={q} />

                      <div className="flex flex-wrap items-center justify-between gap-2 pt-1">
                        <div className="flex items-center gap-1">
                          {PLAN_OPTIONS.map((opt) => (
                            <button
                              key={opt.value}
                              type="button"
                              onClick={async () => {
                                await api.updateQuota(p.adapter_id, {
                                  plan_label: opt.label,
                                });
                                await refreshQuotas();
                              }}
                              className={`px-1.5 py-0.5 rounded border text-[10px] transition-colors ${
                                q.plan_label?.toLowerCase() === opt.label.toLowerCase()
                                  ? "bg-brand-soft border-brand text-brand-ink font-medium"
                                  : "bg-elevated border-line text-ink-muted hover:text-ink"
                              }`}
                            >
                              {opt.label}
                            </button>
                          ))}
                        </div>

                        <button
                          type="button"
                          onClick={async () => {
                            await api.updateQuota(p.adapter_id, {
                              exact_probe_opt_in: !q.exact_probe_opt_in,
                            });
                            await refreshQuotas();
                          }}
                          className={`inline-flex items-center gap-1 px-2 py-0.5 rounded-md border text-[10.5px] transition-colors ${
                            q.exact_probe_opt_in
                              ? "bg-pos-soft border-pos/30 text-pos font-medium"
                              : "bg-elevated border-line text-ink-muted"
                          }`}
                        >
                          {q.exact_probe_opt_in && <Check className="w-2.5 h-2.5" />}
                          <span>Live API</span>
                        </button>
                      </div>
                    </div>
                  )}
                </div>
              );
            })}
          </div>

          {activeSession && (
            <div className="space-y-2.5 pt-3 border-t border-line">
              <h3 className="text-[10.5px] uppercase tracking-wider text-ink-muted font-semibold">
                Session Switch Policy
              </h3>
              <div className="space-y-3 rounded-xl border border-line bg-inset p-3">
                <div className="space-y-1.5">
                  <div className="text-ink-soft font-medium">Permission Mode</div>
                  <div className="space-y-1">
                    {PERMISSION_OPTIONS.map((opt) => {
                      const active = activeSession.permission_policy === opt.value;
                      return (
                        <button
                          key={opt.value}
                          type="button"
                          onClick={async () => {
                            await api.patchSession(activeSession.id, {
                              permission_policy: opt.value,
                            });
                            await selectSession(activeSession.id);
                          }}
                          className={`w-full text-left px-2.5 py-2 rounded-lg border transition-colors ${
                            active
                              ? "bg-brand-soft border-brand text-ink"
                              : "bg-panel border-line text-ink-soft hover:bg-hover"
                          }`}
                        >
                          <div className="font-medium text-xs text-ink">{opt.label}</div>
                          <div className="text-[10.5px] text-ink-muted">{opt.desc}</div>
                        </button>
                      );
                    })}
                  </div>
                </div>

                <div>
                  <label className="block text-ink-soft font-medium mb-1">
                    Max Agent Hops Per User Turn ({activeSession.max_hops})
                  </label>
                  <input
                    type="range"
                    min={2}
                    max={16}
                    value={activeSession.max_hops}
                    onChange={async (e) => {
                      await api.patchSession(activeSession.id, {
                        max_hops: Number(e.target.value),
                      });
                      await selectSession(activeSession.id);
                    }}
                    className="w-full accent-indigo-500"
                  />
                </div>
              </div>

              {permissionRules.length > 0 && (
                <div className="space-y-1.5 pt-2">
                  <div className="text-[11px] text-ink-muted font-medium">
                    Session Allowed Patterns
                  </div>
                  {permissionRules.map((r) => (
                    <div
                      key={r.id}
                      className="flex items-center justify-between px-2.5 py-1.5 rounded-lg bg-inset border border-line font-mono text-[11px] text-ink-soft"
                    >
                      <span>
                        {r.tool_kind}: {r.pattern}
                      </span>
                      <button
                        type="button"
                        onClick={async () => {
                          await api.deletePermissionRule(activeSession.id, r.id);
                          await selectSession(activeSession.id);
                        }}
                        className="text-ink-muted hover:text-neg"
                      >
                        <Trash2 className="w-3 h-3" />
                      </button>
                    </div>
                  ))}
                </div>
              )}
            </div>
          )}
        </div>
      )}
    </aside>
  );
}
