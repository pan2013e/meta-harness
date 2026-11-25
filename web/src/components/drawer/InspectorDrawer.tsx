import { useState } from "react";
import { Pin, Plus, Search, Trash2, X } from "lucide-react";
import { api } from "../../api/client";
import { useAppStore } from "../../store/useAppStore";
import type { PermissionPolicy } from "../../types/api";
import { QuotaMeter } from "../common/Badges";

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
    permissionRules,
    refreshQuotas,
    selectSession,
  } = useAppStore();

  const [memQuery, setMemQuery] = useState("");
  const [newFact, setNewFact] = useState("");
  const [factCategory, setFactCategory] = useState("architecture");

  if (!drawerOpen) return null;

  return (
    <aside className="w-[420px] shrink-0 border-l border-zinc-800 bg-zinc-900/70 flex flex-col h-full">
      <div className="px-3 py-2 border-b border-zinc-800 flex items-center justify-between">
        <div className="flex gap-1 text-xs">
          {(["trajectory", "memory", "settings"] as const).map((t) => (
            <button
              key={t}
              type="button"
              onClick={() => openDrawerTab(t)}
              className={`px-2.5 py-1 rounded-md capitalize transition-colors ${
                drawerTab === t
                  ? "bg-indigo-600 text-white font-medium"
                  : "text-zinc-400 hover:text-zinc-200"
              }`}
            >
              {t === "settings" ? "Switch & Quotas" : t}
            </button>
          ))}
        </div>
        <button
          type="button"
          onClick={closeDrawer}
          className="text-zinc-400 hover:text-zinc-200 p-1"
        >
          <X className="w-4 h-4" />
        </button>
      </div>

      {drawerTab === "trajectory" && (
        <div className="flex-1 flex flex-col min-h-0 p-3 space-y-3">
          <select
            value={inspectedTurnId ?? ""}
            onChange={(e) => {
              if (e.target.value) void inspectTurn(e.target.value);
            }}
            className="w-full rounded bg-zinc-950 border border-zinc-800 px-2.5 py-1.5 text-xs text-zinc-200"
          >
            <option value="">Select a turn trajectory...</option>
            {trajectories.map((t) => (
              <option key={t.turn_id} value={t.turn_id}>
                @{t.member_handle} · {t.model ?? "default"} · {t.status} (
                {new Date(t.started_at).toLocaleTimeString()})
              </option>
            ))}
          </select>

          {inspectedTrajectory && (
            <div className="rounded-lg border border-zinc-800 bg-zinc-950 p-2.5 text-xs space-y-1">
              <div className="flex items-center justify-between">
                <span className="font-semibold text-indigo-300">
                  @{inspectedTrajectory.member_handle}
                </span>
                <span className="font-mono text-[11px] text-zinc-400">
                  {inspectedTrajectory.worktree_branch ?? "shared"}
                </span>
              </div>
              <div className="text-zinc-400">
                {inspectedTrajectory.model ?? "auto"} · {inspectedTrajectory.effort ?? "auto"} ·{" "}
                {inspectedTrajectory.status}
              </div>
              {inspectedTrajectory.touched_files.length > 0 && (
                <div className="text-[11px] text-emerald-400 font-mono truncate">
                  Files: {inspectedTrajectory.touched_files.join(", ")}
                </div>
              )}
            </div>
          )}

          <div className="flex-1 overflow-y-auto space-y-2 pr-1">
            {inspectedEvents.map((ev) => (
              <div
                key={ev.id}
                className="rounded-lg border border-zinc-800 bg-zinc-950/80 p-2.5 text-xs space-y-1"
              >
                <div className="flex items-center justify-between text-[10px] uppercase tracking-wide text-zinc-500">
                  <span>{ev.event_kind}</span>
                  <span>{new Date(ev.created_at).toLocaleTimeString()}</span>
                </div>
                {ev.event_kind === "diff" && typeof ev.payload.diff === "string" ? (
                  <pre className="font-mono text-[11px] text-emerald-300 overflow-x-auto whitespace-pre">
                    {ev.payload.diff}
                  </pre>
                ) : ev.event_kind === "thought" && typeof ev.payload.text === "string" ? (
                  <p className="text-zinc-400 italic whitespace-pre-wrap">
                    {ev.payload.text}
                  </p>
                ) : (
                  <pre className="font-mono text-[11px] text-zinc-300 overflow-x-auto whitespace-pre-wrap">
                    {JSON.stringify(ev.payload, null, 2)}
                  </pre>
                )}
              </div>
            ))}
            {inspectedEvents.length === 0 && (
              <p className="text-xs text-zinc-500 text-center py-8">
                Select a turn above to inspect thoughts, tool calls, and diffs.
              </p>
            )}
          </div>
        </div>
      )}

      {drawerTab === "memory" && (
        <div className="flex-1 flex flex-col min-h-0 p-3 space-y-3 overflow-y-auto">
          <div className="flex gap-2">
            <div className="relative flex-1">
              <Search className="w-3.5 h-3.5 text-zinc-500 absolute left-2.5 top-2" />
              <input
                type="text"
                value={memQuery}
                onChange={(e) => {
                  setMemQuery(e.target.value);
                  void refreshMemory(e.target.value);
                }}
                placeholder="Search project jobs & modules (FTS5)..."
                className="w-full rounded bg-zinc-950 border border-zinc-800 pl-8 pr-2.5 py-1.5 text-xs"
              />
            </div>
          </div>

          {/* Project Facts */}
          <div className="space-y-2">
            <h3 className="text-[11px] uppercase tracking-wider text-zinc-400 font-semibold">
              Pinned Architecture & Notes
            </h3>
            {activeProjectId && (
              <div className="flex gap-1.5 text-xs">
                <select
                  value={factCategory}
                  onChange={(e) => setFactCategory(e.target.value)}
                  className="rounded bg-zinc-950 border border-zinc-800 px-2 py-1"
                >
                  <option value="architecture">Arch</option>
                  <option value="ownership">Owner</option>
                  <option value="harness_affinity">Harness</option>
                  <option value="convention">Rule</option>
                </select>
                <input
                  type="text"
                  value={newFact}
                  onChange={(e) => setNewFact(e.target.value)}
                  placeholder="Add high-level project fact..."
                  className="flex-1 rounded bg-zinc-950 border border-zinc-800 px-2 py-1"
                />
                <button
                  type="button"
                  onClick={async () => {
                    if (!newFact.trim()) return;
                    await api.createFact(activeProjectId, factCategory, newFact.trim());
                    setNewFact("");
                    await refreshMemory(memQuery);
                  }}
                  className="px-2 py-1 rounded bg-indigo-600 text-white"
                >
                  <Plus className="w-3.5 h-3.5" />
                </button>
              </div>
            )}

            {memoryFacts.map((f) => (
              <div
                key={f.id}
                className="rounded border border-zinc-800 bg-zinc-950 p-2 text-xs flex items-start justify-between gap-2"
              >
                <div>
                  <span className="text-[10px] uppercase px-1.5 py-0.5 rounded bg-indigo-500/15 text-indigo-300 mr-1.5">
                    {f.category}
                  </span>
                  <span className="text-zinc-200">{f.summary}</span>
                </div>
                {activeProjectId && (
                  <button
                    type="button"
                    onClick={async () => {
                      await api.deleteFact(activeProjectId, f.id);
                      await refreshMemory(memQuery);
                    }}
                    className="text-zinc-500 hover:text-rose-400"
                  >
                    <Trash2 className="w-3.5 h-3.5" />
                  </button>
                )}
              </div>
            ))}
          </div>

          {/* Curated Episodic Jobs */}
          <div className="space-y-2 pt-2 border-t border-zinc-800">
            <h3 className="text-[11px] uppercase tracking-wider text-zinc-400 font-semibold">
              Curated Episodic Jobs ("Who Did What")
            </h3>
            {memoryJobs.map((j) => (
              <div
                key={j.id}
                className="rounded-lg border border-zinc-800 bg-zinc-950 p-2.5 text-xs space-y-1"
              >
                <div className="flex items-center justify-between">
                  <span className="font-semibold text-indigo-300">
                    @{j.member_handle}{" "}
                    <span className="text-zinc-500 font-normal">
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
                        j.pinned ? "text-amber-400" : "text-zinc-500 hover:text-zinc-300"
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
                      className="text-zinc-500 hover:text-rose-400"
                    >
                      <Trash2 className="w-3.5 h-3.5" />
                    </button>
                  </div>
                </div>
                <p className="text-zinc-200">{j.curated_summary}</p>
                {j.touched_modules.length > 0 && (
                  <div className="text-[11px] font-mono text-zinc-500 truncate">
                    {j.touched_modules.join(", ")}
                  </div>
                )}
              </div>
            ))}
          </div>
        </div>
      )}

      {drawerTab === "settings" && (
        <div className="flex-1 overflow-y-auto p-3 space-y-4 text-xs">
          {/* Adapter Doctor & Quota Calibration */}
          <div className="space-y-2">
            <h3 className="text-[11px] uppercase tracking-wider text-zinc-400 font-semibold">
              Harnesses & Subscription Quotas
            </h3>
            {probes.map((p) => {
              const q = quotas[p.adapter_id];
              return (
                <div
                  key={p.adapter_id}
                  className="rounded-lg border border-zinc-800 bg-zinc-950 p-2.5 space-y-2"
                >
                  <div className="flex items-center justify-between">
                    <span className="font-semibold text-zinc-100">{p.display_name}</span>
                    <span
                      className={`text-[10px] px-1.5 py-0.5 rounded ${
                        p.installed
                          ? "bg-emerald-500/15 text-emerald-300"
                          : "bg-zinc-800 text-zinc-400"
                      }`}
                    >
                      {p.installed ? "CLI Found" : "Not on PATH"}
                    </span>
                  </div>
                  <p className="text-[11px] text-zinc-500">{p.auth_hint}</p>
                  {q && (
                    <div className="space-y-1.5">
                      <QuotaMeter
                        label="5h Window"
                        pct={q.window_5h_remaining_pct}
                        rateLimited={q.rate_limited_until !== null}
                      />
                      <QuotaMeter label="Weekly" pct={q.weekly_remaining_pct} />

                      <div className="flex items-center justify-between gap-2 pt-1">
                        <select
                          value={q.plan_label ?? "pro"}
                          onChange={async (e) => {
                            await api.updateQuota(p.adapter_id, {
                              plan_label: e.target.value,
                            });
                            await refreshQuotas();
                          }}
                          className="rounded bg-zinc-900 border border-zinc-800 px-2 py-1 text-[11px]"
                        >
                          <option value="plus">Plus / Standard</option>
                          <option value="pro">Pro</option>
                          <option value="max_5x">Max 5x</option>
                          <option value="max_20x">Max 20x / Ultra</option>
                        </select>

                        <label className="inline-flex items-center gap-1 text-[11px] text-zinc-400">
                          <input
                            type="checkbox"
                            checked={q.exact_probe_opt_in}
                            onChange={async (e) => {
                              await api.updateQuota(p.adapter_id, {
                                exact_probe_opt_in: e.target.checked,
                              });
                              await refreshQuotas();
                            }}
                          />
                          <span>Exact Probe</span>
                        </label>
                      </div>
                    </div>
                  )}
                </div>
              );
            })}
          </div>

          {activeSession && (
            <div className="space-y-2 pt-3 border-t border-zinc-800">
              <h3 className="text-[11px] uppercase tracking-wider text-zinc-400 font-semibold">
                Session Switch Policy
              </h3>
              <div className="space-y-2 rounded-lg border border-zinc-800 bg-zinc-950 p-2.5">
                <label className="block text-zinc-400">Permission Mode</label>
                <select
                  value={activeSession.permission_policy}
                  onChange={async (e) => {
                    await api.patchSession(activeSession.id, {
                      permission_policy: e.target.value as PermissionPolicy,
                    });
                    await selectSession(activeSession.id);
                  }}
                  className="w-full rounded bg-zinc-900 border border-zinc-800 px-2 py-1.5"
                >
                  <option value="worktree_smart">
                    Worktree-Smart (Auto-allow inside worktree, prompt shell)
                  </option>
                  <option value="strict">Strict (Prompt all writes & shell)</option>
                  <option value="autonomous_sandbox">
                    Autonomous Sandbox (Auto-allow non-destructive local tools)
                  </option>
                </select>

                <label className="block text-zinc-400 pt-1">
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
                  className="w-full"
                />
              </div>

              {permissionRules.length > 0 && (
                <div className="space-y-1 pt-2">
                  <div className="text-[11px] text-zinc-400">
                    Session Allowed Patterns
                  </div>
                  {permissionRules.map((r) => (
                    <div
                      key={r.id}
                      className="flex items-center justify-between px-2 py-1 rounded bg-zinc-950 border border-zinc-800 font-mono text-[11px]"
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
                        className="text-zinc-500 hover:text-rose-400"
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
