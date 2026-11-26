import { useEffect, useRef, useState } from "react";
import { Check, Palette, RefreshCw, Terminal, X } from "lucide-react";
import { api } from "../../api/client";
import { useAppStore } from "../../store/useAppStore";
import { HarnessAvatar, QuotaSnapshotSummary, THEMES } from "../common/Badges";

const PLANS = ["Plus", "Pro", "Max 5x", "Max 20x"];

export function SettingsDialog({ onClose }: { onClose: () => void }) {
  const dialog = useRef<HTMLDialogElement>(null);
  const [section, setSection] = useState<"appearance" | "harnesses">("harnesses");
  const [saving, setSaving] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const { theme, setTheme, probes, quotas, quotasRefreshing, refreshQuotasLive, errorBanner } = useAppStore();
  const orderedProbes = [...probes].sort((a, b) => Number(b.installed) - Number(a.installed));

  useEffect(() => {
    const element = dialog.current;
    if (element && !element.open) element.showModal();
  }, []);

  const saveQuota = async (adapterId: string, preferences: Parameters<typeof api.updateQuota>[1]) => {
    setSaving(adapterId);
    setError(null);
    try {
      const { quota } = await api.updateQuota(adapterId, preferences);
      useAppStore.getState().handleWsEvent({ type: "quota.updated", snapshot: quota });
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setSaving(null);
    }
  };

  return (
    <dialog ref={dialog} onClose={onClose} aria-labelledby="settings-title"
      className="m-auto max-h-[85dvh] w-[calc(100%_-_2rem)] max-w-2xl rounded-2xl border border-line-strong bg-panel p-0 text-ink shadow-2xl"
      onClick={(event) => { if (event.target === dialog.current) dialog.current?.close(); }}>
      <div className="flex max-h-[85dvh] flex-col">
        <header className="flex items-start justify-between gap-4 px-5 py-4 sm:px-6">
          <div>
            <h1 id="settings-title" className="text-lg font-semibold tracking-tight">Settings</h1>
            <p className="mt-1 text-sm text-ink-muted">Applies across your projects and sessions.</p>
          </div>
          <button type="button" aria-label="Close settings" onClick={() => dialog.current?.close()}
            className="rounded-lg p-2 text-ink-muted hover:bg-hover hover:text-ink">
            <X size={17} />
          </button>
        </header>
        <nav aria-label="Settings sections" className="flex gap-2 border-b border-line px-5 pb-3 sm:px-6">
          {([ ["harnesses", "Harnesses", Terminal], ["appearance", "Appearance", Palette] ] as const).map(([id, label, Icon]) => (
            <button key={id} type="button" aria-pressed={section === id} onClick={() => setSection(id)}
              className={`inline-flex items-center gap-2 rounded-lg px-3 py-2 text-sm font-medium ${section === id ? "bg-brand-soft text-brand-ink" : "text-ink-soft hover:bg-hover"}`}>
              <Icon size={15} />{label}
            </button>
          ))}
        </nav>
        <div className="min-h-0 overflow-y-auto p-5 sm:p-6">
          {section === "appearance" ? (
            <div className="grid gap-3 sm:grid-cols-2" aria-label="Color theme">
              {THEMES.map((choice) => (
                <button key={choice.id} type="button" aria-pressed={theme === choice.id} onClick={() => setTheme(choice.id)}
                  className={`flex items-center gap-3 rounded-xl border p-3 text-left ${theme === choice.id ? "border-brand/55 bg-brand-soft" : "border-line bg-elevated hover:bg-hover"}`}>
                  <span aria-hidden="true" className="flex h-9 w-12 shrink-0 overflow-hidden rounded-md border border-line">
                    {choice.swatch.map((color) => <span key={color} className="flex-1" style={{ background: color }} />)}
                  </span>
                  <span className="flex-1 text-sm font-medium">{choice.label}</span>
                  {theme === choice.id && <Check size={15} className="text-brand-ink" />}
                </button>
              ))}
            </div>
          ) : (
            <div className="space-y-3">
              <div className="flex items-center justify-between gap-2">
                <h2 className="text-sm font-semibold">Availability and quotas</h2>
                <button type="button" disabled={quotasRefreshing || saving !== null} onClick={() => void refreshQuotasLive()}
                  className="inline-flex items-center gap-2 rounded-lg border border-line bg-elevated px-3 py-1.5 text-xs hover:bg-hover disabled:opacity-60">
                  <RefreshCw size={13} className={quotasRefreshing ? "animate-spin" : ""} />
                  {quotasRefreshing ? "Refreshing…" : "Refresh"}
                </button>
              </div>
              {(error || errorBanner) && <p role="alert" className="rounded-lg bg-neg-soft px-3 py-2 text-sm text-neg">{error ?? errorBanner}</p>}
              {orderedProbes.map((probe) => {
                const quota = quotas[probe.adapter_id];
                return (
                  <section key={probe.adapter_id} className="space-y-3 rounded-xl border border-line bg-elevated/55 p-3.5">
                    <div className="flex items-start gap-3">
                      <HarnessAvatar handle={probe.adapter_id} accentColor={probe.accent_color} size="sm" />
                      <div className="min-w-0 flex-1">
                        <h3 className="text-sm font-semibold">{probe.display_name}</h3>
                        <p className="mt-0.5 text-xs text-ink-muted">{probe.auth_hint}</p>
                        {probe.resolved_command && <p className="mt-1 truncate font-mono text-[11px] text-ink-muted" title={probe.resolved_command}>{probe.resolved_command}</p>}
                      </div>
                      <span className={`rounded-full px-2 py-0.5 text-[11px] ${probe.installed ? "bg-pos-soft text-pos" : "bg-inset text-ink-muted"}`}>
                        {probe.installed ? "Available" : "Unavailable"}
                      </span>
                    </div>
                    {quota && probe.installed && (
                      <div className="space-y-2 border-t border-line pt-3">
                        <QuotaSnapshotSummary quota={quota} />
                        <div className="flex flex-wrap items-center justify-between gap-3 text-xs">
                          <label className="flex items-center gap-2 text-ink-soft">
                            Plan
                            <select aria-label={`${probe.display_name} plan`} value={quota.plan_label ?? ""}
                              disabled={saving !== null} onChange={(event) => void saveQuota(probe.adapter_id, { plan_label: event.target.value })}
                              className="rounded-lg border border-line bg-inset px-2 py-1.5 text-ink">
                              {!quota.plan_label && <option value="" disabled>Automatic</option>}
                              {quota.plan_label && !PLANS.includes(quota.plan_label) && <option>{quota.plan_label}</option>}
                              {PLANS.map((plan) => <option key={plan}>{plan}</option>)}
                            </select>
                          </label>
                          <label className="inline-flex cursor-pointer items-center gap-2 text-ink-soft">
                            <input type="checkbox" checked={quota.exact_probe_opt_in} disabled={saving !== null}
                              onChange={(event) => void saveQuota(probe.adapter_id, { exact_probe_opt_in: event.target.checked })} />
                            Live provider quota
                          </label>
                        </div>
                      </div>
                    )}
                  </section>
                );
              })}
              {!probes.length && <p className="text-sm text-ink-muted">No harnesses are configured.</p>}
            </div>
          )}
        </div>
      </div>
    </dialog>
  );
}
