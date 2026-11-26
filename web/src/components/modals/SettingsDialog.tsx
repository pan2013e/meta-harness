import { useEffect, useRef, useState } from "react";
import {
  Archive,
  Check,
  ChevronRight,
  Crown,
  HardDrive,
  Info,
  Palette,
  Plus,
  RefreshCw,
  Sliders,
  Terminal,
  Trash2,
  UserPlus,
  X,
} from "lucide-react";
import { api } from "../../api/client";
import { useAppStore } from "../../store/useAppStore";
import { HarnessAvatar, QuotaSnapshotSummary, THEMES } from "../common/Badges";
import { APPROVAL_MODES } from "../common/approvalModes";
import { HarnessMaintenance } from "./HarnessMaintenance";
import { PersonaModelFields, type PersonaModel } from "./PersonaModelFields";
import { GlobalPersonas } from "./GlobalPersonas";
import { SettingsBackup } from "./SettingsBackup";
import { version } from "../../../package.json";

function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return "0 B";
  const units = ["B", "KB", "MB", "GB", "TB"];
  let idx = 0;
  let val = bytes;
  while (val >= 1024 && idx < units.length - 1) {
    val /= 1024;
    idx++;
  }
  return `${idx === 0 ? val.toFixed(0) : val.toFixed(val >= 10 ? 1 : 2)} ${units[idx]}`;
}

export function SettingsDialog({ onClose }: { onClose: () => void }) {
  const dialog = useRef<HTMLDialogElement>(null);
  const [section, setSection] = useState<
    "harnesses" | "session" | "personas" | "storage" | "appearance" | "about"
  >("harnesses");
  const [saving, setSaving] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [cleanupNotice, setCleanupNotice] = useState<string | null>(null);
  const [confirmArchivedDelete, setConfirmArchivedDelete] = useState(false);
  const [showUninstalled, setShowUninstalled] = useState(false);
  const [buildInfo, setBuildInfo] = useState<Awaited<ReturnType<typeof api.buildInfo>> | null>(null);

  const [personaHandle, setPersonaHandle] = useState("codex-reviewer");
  const [personaAdapter, setPersonaAdapter] = useState("codex");
  const [personaOptions, setPersonaOptions] = useState<PersonaModel>({ active_model: null, active_effort: null });
  const [personaName, setPersonaName] = useState("Codex Reviewer");
  const [personaPrompt, setPersonaPrompt] = useState(
    "Focus on architecture, race conditions, and thorough test coverage."
  );

  const {
    theme,
    setTheme,
    probes,
    adapters,
    quotas,
    adapterCatalogs,
    adapterDefaults,
    savedProfiles,
    quotasRefreshing,
    refreshQuotasLive,
    updateAdapterDefaults,
    updateMemberConfig,
    errorBanner,
    activeSession,
    members,
    permissionRules,
    selectSession,
    storageSummary,
    storageLoading,
    loadStorage,
    cleanupStorage,
  } = useAppStore();

  const installedProbes = probes.filter((p) => p.installed);
  const uninstalledProbes = probes.filter((p) => !p.installed);
  const installedAdapters = installedProbes.map((p) => p.adapter_id);
  const visibleMembers = members.filter((m) =>
    installedAdapters.includes(m.adapter_id)
  );

  useEffect(() => {
    const element = dialog.current;
    if (element && !element.open) element.showModal();
  }, []);

  useEffect(() => {
    if (section === "storage") {
      void loadStorage();
    }
  }, [section, loadStorage]);

  useEffect(() => {
    if (section !== "about" || buildInfo) return;
    const controller = new AbortController();
    void api.buildInfo(controller.signal).then((info) => {
      if (!controller.signal.aborted) setBuildInfo(info);
    }).catch(() => undefined);
    return () => controller.abort();
  }, [section, buildInfo]);

  useEffect(() => {
    if (installedAdapters.length > 0 && !installedAdapters.includes(personaAdapter)) {
      setPersonaAdapter(installedAdapters[0]);
    }
  }, [installedAdapters, personaAdapter]);

  const handleDefaultChange = async (
    adapterId: string,
    payload: { default_model?: string | null; default_effort?: string | null }
  ) => {
    setSaving(adapterId);
    setError(null);
    try {
      await updateAdapterDefaults(adapterId, payload);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setSaving(null);
    }
  };

  const handleCleanup = async (action: "clear_caches" | "delete_archived") => {
    setError(null);
    setCleanupNotice(null);
    try {
      const res = await cleanupStorage(action);
      if (action === "clear_caches") {
        setCleanupNotice(
          `Cleared ${res.removed_dirs} orphan/cache directories (freed ${formatBytes(res.freed_bytes)}).`
        );
      } else {
        setCleanupNotice(
          `Deleted ${res.removed_sessions} archived sessions & ${res.removed_dirs} worktree dirs (freed ${formatBytes(res.freed_bytes)}).`
        );
        if (res.harness_warnings?.length) setError(res.harness_warnings.join(" "));
      }
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    }
  };

  const addPersona = async () => {
    if (!activeSession || !personaHandle.trim()) return;
    setSaving("persona");
    setError(null);
    try {
      await api.upsertMember(activeSession.id, {
        handle: personaHandle.trim(),
        adapter_id: personaAdapter,
        display_name: personaName.trim() || personaHandle.trim(),
        persona_prompt: personaPrompt.trim(),
        ...personaOptions,
      });
      if (useAppStore.getState().activeSession?.id === activeSession.id) await selectSession(activeSession.id);
      setPersonaHandle("");
      setPersonaOptions({ active_model: null, active_effort: null });
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setSaving(null);
    }
  };

  const changePersona = async (handle: string, values: Partial<PersonaModel>) => {
    setSaving(handle);
    setError(null);
    try {
      await updateMemberConfig(handle, values);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setSaving(null);
    }
  };

  const removePersona = async (handle: string) => {
    if (!activeSession) return;
    const sid = activeSession.id;
    setSaving(handle);
    setError(null);
    try {
      await api.deleteMember(sid, handle);
      if (useAppStore.getState().activeSession?.id === sid) {
        await selectSession(sid);
      }
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setSaving(null);
    }
  };

  return (
    <dialog
      ref={dialog}
      onClose={onClose}
      aria-labelledby="settings-title"
      className="m-auto h-[min(82dvh,680px)] w-[calc(100%_-_2rem)] max-w-4xl rounded-2xl border border-line-strong bg-panel p-0 text-ink shadow-2xl overflow-hidden"
      onClick={(event) => {
        if (event.target === dialog.current) dialog.current?.close();
      }}
    >
      <div className="flex h-full flex-col">
        {/* Top Title Strip */}
        <header className="flex items-center justify-between gap-4 border-b border-line px-5 py-3.5 sm:px-6 shrink-0 bg-elevated/40">
          <div>
            <h1 id="settings-title" className="text-base font-semibold tracking-tight">
              Settings
            </h1>
          </div>
          <button
            type="button"
            aria-label="Close settings"
            onClick={() => dialog.current?.close()}
            className="rounded-lg p-2 text-ink-muted hover:bg-hover hover:text-ink"
          >
            <X size={17} />
          </button>
        </header>

        {/* 2-Column Body */}
        <div className="flex flex-1 min-h-0 flex-col sm:flex-row">
          <nav
            aria-label="Settings sections"
            className="w-full sm:w-52 shrink-0 border-b sm:border-b-0 sm:border-r border-line bg-inset/45 p-3 flex sm:flex-col gap-1 overflow-x-auto"
          >
            {(
              [
                ["appearance", "Appearance", Palette],
                ["harnesses", "Harnesses & Models", Terminal],
                ["personas", "Global Personas", UserPlus],
                ["session", "Session", Sliders],
                ["storage", "Storage & Data", HardDrive],
                ["about", "About", Info],
              ] as const
            ).map(([id, label, Icon]) => (
              <button
                key={id}
                type="button"
                aria-pressed={section === id}
                onClick={() => setSection(id)}
                className={`inline-flex items-center gap-2.5 rounded-xl px-3 py-2 text-xs font-medium shrink-0 text-left transition-colors ${
                  section === id
                    ? "bg-brand-soft text-brand-ink border border-brand/30"
                    : "text-ink-soft hover:bg-hover hover:text-ink border border-transparent"
                }`}
              >
                <Icon size={14} className="shrink-0" />
                <span>{label}</span>
              </button>
            ))}
          </nav>

          <div className="flex-1 min-w-0 overflow-y-auto p-5 sm:p-6">
            {(error || errorBanner) && (
              <p
                role="alert"
                className="mb-3 rounded-lg bg-neg-soft px-3 py-2 text-xs text-neg"
              >
                {error ?? errorBanner}
              </p>
            )}

            {section === "about" && (
              <section className="space-y-5 text-sm">
                <div className="flex items-center gap-3">
                  <img src="/meta-harness.svg" alt="" className="h-12 w-12" />
                  <div>
                    <h2 className="text-base font-semibold">Meta-Harness</h2>
                    <p className="flex items-center gap-2 text-xs text-ink-muted">
                      Version {buildInfo?.version ?? version}
                      {buildInfo?.commit && <code aria-label={"Commit " + buildInfo.commit}>{buildInfo.commit}</code>}
                    </p>
                  </div>
                </div>
                <div className="space-y-1">
                  <p>Created by Zhiyuan Pan</p>
                  <p className="text-ink-muted">© 2026 Zhiyuan Pan</p>
                  <div className="flex flex-wrap gap-x-4 gap-y-1 text-brand-ink">
                    <a className="underline" href="https://github.com/pan2013e/meta-harness" target="_blank" rel="noreferrer">GitHub repository</a>
                    <a className="underline" href="https://github.com/pan2013e/meta-harness/blob/main/LICENSE" target="_blank" rel="noreferrer">MIT License</a>
                  </div>
                </div>
                <div className="space-y-2 border-t border-line pt-4">
                  <h3 className="font-medium">Open-source software</h3>
                  <ul className="space-y-1 text-xs text-ink-soft">
                    <li>Rust, Tokio, Axum and SQLite</li>
                    <li>React, Tailwind CSS and xterm.js</li>
                    <li>Monaco Editor and PDF.js</li>
                    <li>Electron and Chromium in the desktop app</li>
                  </ul>
                  <a className="inline-block text-xs text-brand-ink underline" href="/third-party-licenses.txt" download>Third-party license notices</a>
                </div>
              </section>
            )}

            {section === "storage" && (
              <div className="space-y-4 text-xs">
                <SettingsBackup />
                <div className="flex items-center justify-between gap-2">
                  <h2 className="text-[11px] uppercase tracking-wider font-semibold text-ink-muted">
                    Disk Usage &amp; Cleanup
                  </h2>
                  <button
                    type="button"
                    disabled={storageLoading}
                    onClick={() => void loadStorage()}
                    className="inline-flex items-center gap-1.5 rounded-lg border border-line bg-elevated px-3 py-1.5 text-xs hover:bg-hover disabled:opacity-60"
                  >
                    <RefreshCw
                      size={13}
                      className={storageLoading ? "animate-spin" : ""}
                    />
                    <span>{storageLoading ? "Scanning…" : "Recalculate"}</span>
                  </button>
                </div>

                {cleanupNotice && (
                  <p className="rounded-lg bg-pos-soft border border-pos/30 px-3 py-2 text-xs text-pos">
                    {cleanupNotice}
                  </p>
                )}

                {storageSummary ? (
                  <>
                    <div className="rounded-xl border border-line bg-elevated/55 p-4 flex items-center justify-between gap-4">
                      <div>
                        <div className="text-[11px] text-ink-muted">
                          Total Meta-Harness Storage
                        </div>
                        <div className="text-xl font-semibold font-mono text-ink mt-0.5">
                          {formatBytes(storageSummary.total_bytes)}
                        </div>
                      </div>
                      <div className="text-right font-mono text-[10.5px] text-ink-muted space-y-0.5 min-w-0">
                        <div className="truncate" title={storageSummary.data_dir}>
                          Data: {storageSummary.data_dir}
                        </div>
                        <div className="truncate" title={storageSummary.workspaces_dir}>
                          Workspaces: {storageSummary.workspaces_dir}
                        </div>
                      </div>
                    </div>

                    <div className="grid grid-cols-1 sm:grid-cols-2 gap-2.5">
                      <div className="rounded-xl border border-line bg-inset p-3">
                        <div className="text-ink-muted text-[11px]">SQLite Database &amp; WAL</div>
                        <div className="font-mono text-sm font-semibold text-ink mt-0.5">
                          {formatBytes(storageSummary.database_bytes)}
                        </div>
                      </div>
                      <div className="rounded-xl border border-line bg-inset p-3">
                        <div className="text-ink-muted text-[11px]">
                          Project Worktrees ({storageSummary.active_worktrees_count})
                        </div>
                        <div className="font-mono text-sm font-semibold text-ink mt-0.5">
                          {formatBytes(storageSummary.active_worktrees_bytes)}
                        </div>
                      </div>
                      <div className="rounded-xl border border-line bg-inset p-3">
                        <div className="text-ink-muted text-[11px]">
                          Standalone Sessions ({storageSummary.standalone_count})
                        </div>
                        <div className="font-mono text-sm font-semibold text-ink mt-0.5">
                          {formatBytes(storageSummary.standalone_workspaces_bytes)}
                        </div>
                      </div>
                      <div className="rounded-xl border border-line bg-inset p-3">
                        <div className="text-ink-muted text-[11px]">
                          Archived Sessions ({storageSummary.archived_sessions_count})
                        </div>
                        <div className="font-mono text-sm font-semibold text-ink mt-0.5">
                          {formatBytes(storageSummary.archived_bytes)}
                        </div>
                      </div>
                      <div className="rounded-xl border border-line bg-inset p-3 sm:col-span-2 flex items-center justify-between gap-2">
                        <div>
                          <div className="text-ink-muted text-[11px]">
                            Caches &amp; Orphan Directories ({storageSummary.orphan_dirs_count})
                          </div>
                          <div className="font-mono text-sm font-semibold text-ink mt-0.5">
                            {formatBytes(storageSummary.cache_and_orphans_bytes)}
                          </div>
                        </div>
                      </div>
                    </div>

                    <div className="flex flex-wrap items-center gap-2.5 pt-2 border-t border-line">
                      <button
                        type="button"
                        disabled={storageLoading}
                        onClick={() => void handleCleanup("clear_caches")}
                        className="inline-flex items-center gap-1.5 rounded-xl border border-line bg-elevated hover:bg-hover px-3.5 py-2 text-xs font-medium text-ink disabled:opacity-50 transition-colors"
                      >
                        <Trash2 className="w-3.5 h-3.5 text-brand-ink" />
                        <span>Clear Caches &amp; Orphans</span>
                      </button>

                      <button
                        type="button"
                        disabled={
                          storageLoading || storageSummary.archived_sessions_count === 0
                        }
                        onClick={() => setConfirmArchivedDelete(true)}
                        className="inline-flex items-center gap-1.5 rounded-xl bg-neg-soft hover:bg-neg/20 border border-neg/30 px-3.5 py-2 text-xs font-medium text-neg disabled:opacity-40 transition-colors"
                      >
                        <Archive className="w-3.5 h-3.5" />
                        <span>
                          Delete Archived Sessions ({storageSummary.archived_sessions_count})
                        </span>
                      </button>
                    </div>
                    {confirmArchivedDelete && (
                      <div className="flex flex-wrap items-center justify-between gap-3 rounded-xl border border-neg/30 bg-neg-soft p-3">
                        <p>Permanently delete the archived sessions and their linked conversations in supported harnesses?</p>
                        <div className="flex gap-2">
                          <button type="button" onClick={() => setConfirmArchivedDelete(false)} className="rounded-lg px-3 py-1.5 hover:bg-hover">Cancel</button>
                          <button
                            type="button"
                            disabled={storageLoading || storageSummary.archived_sessions_count === 0}
                            onClick={() => {
                              setConfirmArchivedDelete(false);
                              void handleCleanup("delete_archived");
                            }}
                            className="rounded-lg bg-neg px-3 py-1.5 font-medium text-white disabled:opacity-40"
                          >
                            Delete sessions
                          </button>
                        </div>
                      </div>
                    )}
                  </>
                ) : (
                  <div className="rounded-xl border border-line bg-inset p-6 text-center text-ink-muted">
                    {storageLoading ? "Calculating storage usage…" : "No storage summary available."}
                  </div>
                )}
              </div>
            )}

            {section === "appearance" && (
              <div className="space-y-3">
                <h2 className="text-[11px] uppercase tracking-wider font-semibold text-ink-muted">
                  Workspace Theme
                </h2>
                <div className="grid gap-3 sm:grid-cols-2" aria-label="Color theme">
                  {THEMES.map((choice) => (
                    <button
                      key={choice.id}
                      type="button"
                      aria-pressed={theme === choice.id}
                      onClick={() => setTheme(choice.id)}
                      className={`flex items-center gap-3 rounded-xl border p-3 text-left transition-colors ${
                        theme === choice.id
                          ? "border-brand/55 bg-brand-soft"
                          : "border-line bg-elevated hover:bg-hover"
                      }`}
                    >
                      <span
                        aria-hidden="true"
                        className="flex h-9 w-12 shrink-0 overflow-hidden rounded-md border border-line"
                      >
                        {choice.swatch.map((color) => (
                          <span
                            key={color}
                            className="flex-1"
                            style={{ background: color }}
                          />
                        ))}
                      </span>
                      <div className="min-w-0 flex-1">
                        <div className="text-sm font-medium">{choice.label}</div>
                        <div className="text-[11px] text-ink-muted">{choice.desc}</div>
                      </div>
                      {theme === choice.id && (
                        <Check size={15} className="text-brand-ink shrink-0" />
                      )}
                    </button>
                  ))}
                </div>
              </div>
            )}

            {section === "harnesses" && (
              <div className="space-y-4">
                <div className="flex items-center justify-between gap-2">
                  <h2 className="text-[11px] uppercase tracking-wider font-semibold text-ink-muted">
                    Available Harnesses &amp; Quotas
                  </h2>
                  <button
                    type="button"
                    disabled={quotasRefreshing || saving !== null}
                    onClick={() => void refreshQuotasLive()}
                    className="inline-flex items-center gap-1.5 rounded-lg border border-line bg-elevated px-3 py-1.5 text-xs hover:bg-hover disabled:opacity-60"
                  >
                    <RefreshCw
                      size={13}
                      className={quotasRefreshing ? "animate-spin" : ""}
                    />
                    {quotasRefreshing ? "Refreshing…" : "Refresh Models & Quotas"}
                  </button>
                </div>

                {installedProbes.length === 0 ? (
                  <div className="rounded-xl border border-line bg-inset p-4 text-xs text-ink-muted">
                    No installed CLI harnesses were detected on PATH. Expand &ldquo;Not installed&rdquo; below to see setup instructions.
                  </div>
                ) : (
                  installedProbes.map((probe) => {
                    const quota = quotas[probe.adapter_id];
                    const adapter = adapters[probe.adapter_id];
                    const isAntigravityCli = adapter?.quota_provider === "google_local"
                      && /(?:^|[\\/])(?:agy|antigravity)(?:\.(?:exe|cmd|bat|ps1))?$/i.test(probe.resolved_command ?? "");
                    const catalog = adapterCatalogs[probe.adapter_id];
                    const pref = adapterDefaults[probe.adapter_id];
                    const modelDetails =
                      catalog?.model_details && catalog.model_details.length > 0
                        ? catalog.model_details
                        : (catalog?.models ?? []).map((m) => ({
                            value: m.value,
                            label: m.label,
                            description: "",
                            is_default: false,
                            default_effort: null,
                            supported_efforts: catalog?.efforts ?? [],
                          }));
                    const activeDetail = modelDetails.find(
                      (m) => m.value === (pref?.default_model ?? "")
                    );
                    const efforts =
                      activeDetail && activeDetail.supported_efforts.length > 0
                        ? activeDetail.supported_efforts
                        : catalog?.efforts ?? [];

                    return (
                      <section
                        key={probe.adapter_id}
                        className="space-y-3 rounded-xl border border-line bg-elevated/55 p-4"
                      >
                        <div className="flex items-start gap-3">
                          <HarnessAvatar
                            handle={probe.adapter_id}
                            accentColor={probe.accent_color}
                            size="sm"
                          />
                          <div className="min-w-0 flex-1">
                            <div className="flex items-center gap-2 flex-wrap">
                              <h3 className="text-sm font-semibold">{probe.display_name}</h3>
                              {quota?.plan_label && (
                                <span className="rounded-md bg-inset border border-line px-1.5 py-0.5 text-[10px] font-mono text-ink-soft">
                                  {quota.plan_label}
                                </span>
                              )}
                            </div>
                            <p className="mt-0.5 text-xs text-ink-muted">
                              {probe.auth_hint}
                            </p>
                            {adapters[probe.adapter_id]?.profile_dir && (
                              <p className="mt-1 truncate text-[11px] text-ink-muted" title={adapters[probe.adapter_id].profile_dir ?? undefined}>
                                Separate profile · {adapters[probe.adapter_id].profile_dir}
                              </p>
                            )}
                            {probe.resolved_command && (
                              <p
                                className="mt-1 truncate font-mono text-[11px] text-ink-muted"
                                title={probe.resolved_command}
                              >
                                {probe.resolved_command}
                              </p>
                            )}
                          </div>
                          <span className="rounded-full px-2 py-0.5 text-[11px] bg-pos-soft text-pos shrink-0">
                            Available
                          </span>
                        </div>
                        <HarnessMaintenance
                          adapterId={probe.adapter_id}
                          profile={adapter?.profile_dir}
                          hasSavedProfile={savedProfiles.includes(probe.adapter_id)}
                          offersDeviceCode={adapter?.quota_provider === "codex_local"}
                          offersSignIn={["codex_local", "claude_local"].includes(adapter?.quota_provider ?? "") || isAntigravityCli}
                        />

                        {quota && (
                          <div className="border-t border-line pt-3">
                            <QuotaSnapshotSummary quota={quota} />
                          </div>
                        )}

                        <div className="grid grid-cols-1 sm:grid-cols-2 gap-3 border-t border-line pt-3 text-xs">
                          <label className="space-y-1">
                            <span className="block text-[11px] font-medium text-ink-soft">
                              Default Model
                            </span>
                            <select
                              aria-label={`${probe.display_name} default model`}
                              value={pref?.default_model ?? ""}
                              disabled={saving !== null}
                              onChange={(e) => {
                                const nextModel = e.target.value || null;
                                const nextDetail = modelDetails.find(
                                  (m) => m.value === nextModel
                                );
                                const allowed =
                                  nextDetail && nextDetail.supported_efforts.length > 0
                                    ? nextDetail.supported_efforts
                                    : catalog?.efforts ?? [];
                                const curEff = pref?.default_effort ?? null;
                                const validEff =
                                  curEff === null || allowed.some((x) => x.value === curEff);
                                void handleDefaultChange(probe.adapter_id, {
                                  default_model: nextModel,
                                  ...(validEff ? {} : { default_effort: null }),
                                });
                              }}
                              className="w-full rounded-lg border border-line bg-inset px-2.5 py-1.5 font-mono text-xs text-ink"
                            >
                              <option value="">Harness Default</option>
                              {pref?.default_model &&
                                !modelDetails.some((m) => m.value === pref.default_model) && (
                                  <option value={pref.default_model}>
                                    {pref.default_model}
                                  </option>
                                )}
                              {modelDetails.map((m) => (
                                <option key={m.value} value={m.value}>
                                  {m.label}
                                </option>
                              ))}
                            </select>
                          </label>

                          <label className="space-y-1">
                            <span className="block text-[11px] font-medium text-ink-soft">
                              Default Reasoning Effort
                            </span>
                            <select
                              aria-label={`${probe.display_name} default effort`}
                              value={pref?.default_effort ?? ""}
                              disabled={saving !== null}
                              onChange={(e) =>
                                void handleDefaultChange(probe.adapter_id, {
                                  default_effort: e.target.value || null,
                                })
                              }
                              className="w-full rounded-lg border border-line bg-inset px-2.5 py-1.5 text-xs text-ink"
                            >
                              <option value="">Harness Default</option>
                              {efforts.map((eff) => (
                                <option key={eff.value} value={eff.value}>
                                  {eff.label}
                                </option>
                              ))}
                            </select>
                          </label>
                        </div>
                      </section>
                    );
                  })
                )}

                {uninstalledProbes.length > 0 && (
                  <div className="pt-2">
                    <button
                      type="button"
                      onClick={() => setShowUninstalled((v) => !v)}
                      className="inline-flex items-center gap-1.5 text-xs text-ink-muted hover:text-ink transition-colors"
                    >
                      <ChevronRight
                        size={14}
                        className={`transition-transform ${showUninstalled ? "rotate-90" : ""}`}
                      />
                      <span>Not installed ({uninstalledProbes.length})</span>
                    </button>

                    {showUninstalled && (
                      <div className="mt-2.5 space-y-2">
                        {uninstalledProbes.map((probe) => (
                          <div
                            key={probe.adapter_id}
                            className="flex items-start justify-between gap-3 rounded-xl border border-line bg-inset/60 p-3 text-xs"
                          >
                            <div className="flex items-start gap-2.5 min-w-0">
                              <HarnessAvatar
                                handle={probe.adapter_id}
                                accentColor={probe.accent_color}
                                size="sm"
                              />
                              <div className="min-w-0">
                                <div className="font-semibold text-ink">
                                  {probe.display_name}
                                </div>
                                <p className="text-[11px] text-ink-muted mt-0.5">
                                  {probe.auth_hint}
                                </p>
                              </div>
                            </div>
                            <span className="rounded-full bg-elevated px-2 py-0.5 text-[10.5px] text-ink-muted shrink-0">
                              Not Installed
                            </span>
                          </div>
                        ))}
                      </div>
                    )}
                  </div>
                )}
              </div>
            )}

            {section === "personas" && <GlobalPersonas />}

            {section === "session" && (
              <div className="space-y-5 text-xs">
                {!activeSession ? (
                  <div className="rounded-xl border border-line bg-inset/60 p-6 text-center text-ink-muted">
                    Start or select a session to configure its Orchestrator, custom personas, and permission rules.
                  </div>
                ) : (
                  <>
                    {/* Orchestrator & Session Members (installed only) */}
                    <section className="space-y-2.5">
                      <h2 className="text-[11px] uppercase tracking-wider font-semibold text-ink-muted">
                        Session Roster &amp; Orchestrator
                      </h2>
                      <div className="space-y-1.5">
                        {visibleMembers.map((m) => {
                          const isOrch = activeSession.orchestrator_member === m.handle;
                          return (
                            <div
                              key={m.handle}
                              className="space-y-2.5 rounded-xl border border-line bg-inset px-3 py-2"
                            >
                              <div className="flex items-center justify-between gap-2">
                              <div className="min-w-0">
                                <div className="flex items-center gap-2">
                                  <span className="font-mono font-semibold text-ink">
                                    @{m.handle}
                                  </span>
                                  <span className="text-ink-muted">({m.adapter_id})</span>
                                </div>
                                {m.persona_prompt && (
                                  <p className="text-[11px] text-ink-muted truncate mt-0.5">
                                    {m.persona_prompt}
                                  </p>
                                )}
                              </div>

                              <button
                                type="button"
                                onClick={async () => {
                                  await api.patchSession(activeSession.id, {
                                    orchestrator_member: isOrch ? null : m.handle,
                                  });
                                  await selectSession(activeSession.id);
                                }}
                                className={`inline-flex items-center gap-1 px-2.5 py-1 rounded-lg border text-[11px] shrink-0 transition-colors ${
                                  isOrch
                                    ? "bg-warn-soft border-warn/40 text-warn font-medium"
                                    : "bg-elevated border-line text-ink-soft hover:text-ink"
                                }`}
                              >
                                <Crown className="w-3 h-3" />
                                <span>{isOrch ? "Orchestrator" : "Make Orchestrator"}</span>
                              </button>
                              {(m.handle !== m.adapter_id || Boolean(m.persona_prompt)) && (
                                <button type="button" aria-label={"Remove persona @" + m.handle}
                                  disabled={saving !== null} onClick={() => void removePersona(m.handle)}
                                  className="shrink-0 rounded-lg p-1.5 text-ink-muted hover:bg-neg-soft hover:text-neg">
                                  <Trash2 className="h-3.5 w-3.5" />
                                </button>
                              )}
                              </div>
                              <PersonaModelFields label={"@" + m.handle} catalog={adapterCatalogs[m.adapter_id]}
                                model={m.active_model} effort={m.active_effort} disabled={saving !== null}
                                onChange={(values) => void changePersona(m.handle, values)} />
                            </div>
                          );
                        })}
                      </div>

                      {/* Add Persona */}
                      {installedAdapters.length > 0 && (
                        <div className="rounded-xl border border-line bg-elevated/40 p-3 space-y-2.5">
                          <div className="flex items-center gap-1.5 font-medium text-ink">
                            <UserPlus className="w-3.5 h-3.5 text-brand-ink" />
                            <span>Add Custom Persona</span>
                          </div>
                          <div className="grid grid-cols-1 sm:grid-cols-3 gap-2">
                            <input
                              type="text"
                              value={personaHandle}
                              onChange={(e) => setPersonaHandle(e.target.value)}
                              placeholder="@handle"
                              className="rounded-lg bg-inset border border-line px-2.5 py-1.5 font-mono text-ink"
                            />
                            <select
                              aria-label="Persona adapter"
                              value={personaAdapter}
                              onChange={(e) => {
                                setPersonaAdapter(e.target.value);
                                setPersonaOptions({ active_model: null, active_effort: null });
                              }}
                              className="rounded-lg bg-inset border border-line px-2.5 py-1.5 text-ink"
                            >
                              {installedAdapters.map((a) => (
                                <option key={a} value={a}>
                                  {a}
                                </option>
                              ))}
                            </select>
                            <input
                              type="text"
                              value={personaName}
                              onChange={(e) => setPersonaName(e.target.value)}
                              placeholder="Display name"
                              className="rounded-lg bg-inset border border-line px-2.5 py-1.5 text-ink"
                            />
                          </div>
                          <PersonaModelFields label="New persona" catalog={adapterCatalogs[personaAdapter]}
                            model={personaOptions.active_model} effort={personaOptions.active_effort}
                            disabled={saving !== null}
                            onChange={(values) => setPersonaOptions((previous) => ({ ...previous, ...values }))} />
                          <div className="flex gap-2">
                            <input
                              type="text"
                              value={personaPrompt}
                              onChange={(e) => setPersonaPrompt(e.target.value)}
                              placeholder="Persona system guidance..."
                              className="flex-1 rounded-lg bg-inset border border-line px-2.5 py-1.5 text-ink"
                            />
                            <button
                              type="button"
                              disabled={saving !== null || !personaHandle.trim()}
                              onClick={() => void addPersona()}
                              className="inline-flex items-center gap-1 px-3 py-1.5 rounded-lg bg-brand hover:bg-brand-hover disabled:opacity-40 text-white font-medium"
                            >
                              <Plus className="w-3.5 h-3.5" />
                              <span>Add</span>
                            </button>
                          </div>
                        </div>
                      )}
                    </section>

                    {/* Permission Policy & Hops */}
                    <section className="space-y-2.5 pt-3 border-t border-line">
                      <h2 className="text-[11px] uppercase tracking-wider font-semibold text-ink-muted">
                        Permissions &amp; Loop Guard
                      </h2>
                      <div className="space-y-1.5">
                        {APPROVAL_MODES.map(({ value, label, description, Icon, color }) => {
                          const active = activeSession.permission_policy === value;
                          return (
                            <button
                              key={value}
                              type="button"
                              onClick={async () => {
                                await api.patchSession(activeSession.id, {
                                  permission_policy: value,
                                });
                                await selectSession(activeSession.id);
                              }}
                              className={`w-full text-left px-3 py-2 rounded-xl border transition-colors ${
                                active
                                  ? "bg-brand-soft border-brand text-ink"
                                  : "bg-inset border-line text-ink-soft hover:bg-hover"
                              }`}
                            >
                              <div className="flex items-center gap-1.5 font-medium text-ink">
                                <Icon aria-hidden="true" className={`h-3.5 w-3.5 ${color}`} />
                                {label}
                              </div>
                              <div className="text-[11px] text-ink-muted">{description}</div>
                            </button>
                          );
                        })}
                      </div>

                      <div className="rounded-xl border border-line bg-inset p-3">
                        <label className="block text-ink-soft font-medium mb-1.5">
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

                      {permissionRules.length > 0 && (
                        <div className="space-y-1.5">
                          <div className="text-[11px] text-ink-muted font-medium">
                            Allowed Command Patterns for Session
                          </div>
                          {permissionRules.map((r) => (
                            <div
                              key={r.id}
                              className="flex items-center justify-between px-3 py-1.5 rounded-lg bg-inset border border-line font-mono text-[11px] text-ink-soft"
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
                    </section>
                  </>
                )}
              </div>
            )}
          </div>
        </div>
      </div>
    </dialog>
  );
}
