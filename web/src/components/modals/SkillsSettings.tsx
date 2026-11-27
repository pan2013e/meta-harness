import { notifyInApp, reportInAppError } from "../../inAppNotifications";
import { useRef, useState } from "react";
import {
  ChevronDown,
  ChevronRight,
  FolderOpen,
  GitBranch,
  Loader2,
  Plus,
  RefreshCw,
  RotateCcw,
  Save,
  Sparkles,
  Trash2,
} from "lucide-react";
import { api } from "../../api/client";
import { useAppStore } from "../../store/useAppStore";
import type { SkillDetail, SkillSource, SkillSummary } from "../../types/api";
import { FolderPickerModal } from "./FolderPickerModal";

const fieldClass =
  "w-full min-w-0 rounded-lg border border-line bg-inset px-2.5 py-1.5 text-ink focus:outline-none focus:border-brand";
const actionClass = "settings-button";

function sourceLabel(source: SkillSource): { text: string; cls: string } {
  switch (source) {
    case "builtin":
      return { text: "Built-in", cls: "bg-brand-soft text-brand-ink border-brand/30" };
    case "git":
      return { text: "Git", cls: "bg-pos-soft text-pos border-pos/30" };
    case "project":
      return { text: "Repo .agents/skills", cls: "bg-warn-soft text-warn border-warn/30" };
    case "custom":
      return { text: "Custom", cls: "bg-skill-soft text-skill border-skill/30" };
    default:
      return { text: "Local", cls: "bg-elevated text-ink-soft border-line" };
  }
}

export function SkillsSettings() {
  const { activeProjectId, skills, skillsDir, skillsLoading, refreshSkills, toggleSkill } =
    useAppStore();

  const [busy, setBusy] = useState(false);

  const [folderPickerOpen, setFolderPickerOpen] = useState(false);
  const [installMode, setInstallMode] = useState<"none" | "git" | "custom">("none");
  const [gitUrl, setGitUrl] = useState("");
  const [gitSubpath, setGitSubpath] = useState("");

  const [customName, setCustomName] = useState("");
  const [customId, setCustomId] = useState("");
  const [customDesc, setCustomDesc] = useState("");
  const [customBody, setCustomBody] = useState(
    "# Instructions\n\nDescribe when harnesses should apply this skill and key conventions to follow.\n"
  );

  const [expandedId, setExpandedId] = useState<string | null>(null);
  const [detail, setDetail] = useState<SkillDetail | null>(null);
  const [detailLoading, setDetailLoading] = useState(false);
  const [draftContent, setDraftContent] = useState("");
  const [confirmUninstallId, setConfirmUninstallId] = useState<string | null>(null);
  const detailRequest = useRef(0);

  const runAction = async (fn: () => Promise<void>) => {
    setBusy(true);
    try {
      await fn();
    } catch (cause) {
      reportInAppError(cause);
    } finally {
      setBusy(false);
    }
  };

  const openDetailFile = async (skill: SkillSummary, file?: string) => {
    const request = ++detailRequest.current;
    setExpandedId(skill.id);
    setDetail(null);
    setDetailLoading(true);
    try {
      const res = await api.getSkillDetail(skill.id, {
        projectId: activeProjectId,
        file: file ?? "SKILL.md",
      });
      if (request !== detailRequest.current) return;
      setDetail(res.detail);
      setDraftContent(res.detail.content);
    } catch (cause) {
      if (request === detailRequest.current) reportInAppError(cause);
    } finally {
      if (request === detailRequest.current) setDetailLoading(false);
    }
  };

  const toggleExpand = (skill: SkillSummary) => {
    if (expandedId === skill.id) {
      detailRequest.current++;
      setExpandedId(null);
      setDetail(null);
      setDetailLoading(false);
      return;
    }
    void openDetailFile(skill, "SKILL.md");
  };

  const handleInstallFolder = async (folderPath: string) => {
    setBusy(true);
    try {
      const res = await api.installSkillFromFolder(folderPath);
      await refreshSkills();
      setFolderPickerOpen(false);
      notifyInApp(
        `Installed ${res.installed.length} skill${res.installed.length === 1 ? "" : "s"} (${res.installed
          .map((s) => `/${s.id}`)
          .join(", ")}).`, "success",
      );
    } finally {
      setBusy(false);
    }
  };

  const handleInstallGit = async () => {
    if (!gitUrl.trim()) return;
    await runAction(async () => {
      const res = await api.installSkillFromGit(gitUrl.trim(), gitSubpath.trim() || undefined);
      await refreshSkills();
      setGitUrl("");
      setGitSubpath("");
      setInstallMode("none");
      notifyInApp(
        `Installed ${res.installed.length} skill${res.installed.length === 1 ? "" : "s"} (${res.installed
          .map((s) => `/${s.id}`)
          .join(", ")}).`, "success",
      );
    });
  };

  const handleCreateCustom = async () => {
    if (!customName.trim() || !customDesc.trim()) return;
    await runAction(async () => {
      const res = await api.createCustomSkill({
        id: customId.trim() || undefined,
        name: customName.trim(),
        description: customDesc.trim(),
        content: customBody,
      });
      await refreshSkills();
      setCustomName("");
      setCustomId("");
      setCustomDesc("");
      setInstallMode("none");
      notifyInApp(`Created custom skill /${res.skill.id}.`, "success");
    });
  };

  const handleSaveContent = async () => {
    if (!detail) return;
    await runAction(async () => {
      const res = await api.updateSkillContent(detail.skill.id, {
        file: detail.file_path,
        content: draftContent,
      });
      await refreshSkills();
      setDetail(res.detail);
      setDraftContent(res.detail.content);
      notifyInApp(`Saved ${res.detail.file_path} for /${res.detail.skill.id}.`, "success");
    });
  };

  const handleResetBuiltin = async (id: string) => {
    await runAction(async () => {
      const res = await api.resetSkill(id);
      await refreshSkills();
      if (expandedId === id) {
        const refreshed = await api.getSkillDetail(id, { projectId: activeProjectId });
        setDetail(refreshed.detail);
        setDraftContent(refreshed.detail.content);
      }
      notifyInApp(`Restored /${res.skill.id} to built-in version.`, "success");
    });
  };

  const handleUninstall = async (id: string) => {
    await runAction(async () => {
      await api.deleteSkill(id);
      setConfirmUninstallId(null);
      if (expandedId === id) {
        setExpandedId(null);
        setDetail(null);
      }
      await refreshSkills();
      notifyInApp(`Uninstalled /${id}.`, "success");
    });
  };

  return (
    <section className="space-y-4 text-xs">
      <div className="flex flex-wrap items-start justify-between gap-2">
        <div className="min-w-0">
          <p className="mt-1 text-ink-muted">
            Installed skills are available to every harness and through{" "}
            <code className="font-mono text-skill">/skill</code> composer commands.
          </p>
          {skillsDir && (
            <p className="mt-1 truncate font-mono text-[10.5px] text-ink-muted" title={skillsDir}>
              Global directory: {skillsDir}
            </p>
          )}
        </div>
        <button
          type="button"
          disabled={busy || skillsLoading}
          onClick={() => void runAction(() => refreshSkills())}
          className={actionClass}
        >
          <RefreshCw size={13} className={skillsLoading ? "animate-spin" : ""} />
          <span>Refresh</span>
        </button>
      </div>

      {/* Install Controls */}
      <div className="flex flex-wrap items-center gap-2">
        <button
          type="button"
          disabled={busy}
          onClick={() => setFolderPickerOpen(true)}
          className={actionClass}
        >
          <FolderOpen className="h-3.5 w-3.5 text-brand-ink" />
          <span>Install Folder…</span>
        </button>
        <button
          type="button"
          disabled={busy}
          aria-pressed={installMode === "git"}
          onClick={() => setInstallMode((m) => (m === "git" ? "none" : "git"))}
          className={actionClass}
        >
          <GitBranch className="h-3.5 w-3.5 text-pos" />
          <span>Install from Git URL</span>
        </button>
        <button
          type="button"
          disabled={busy}
          aria-pressed={installMode === "custom"}
          onClick={() => setInstallMode((m) => (m === "custom" ? "none" : "custom"))}
          className={actionClass}
        >
          <Plus className="h-3.5 w-3.5 text-skill" />
          <span>New Custom Skill</span>
        </button>
      </div>


      {/* Install from Git URL Form */}
      {installMode === "git" && (
        <form
          onSubmit={(e) => {
            e.preventDefault();
            void handleInstallGit();
          }}
          className="settings-card space-y-2.5"
        >
          <div className="font-medium text-ink">Install from Git / GitHub Repository</div>
          <div className="grid grid-cols-1 sm:grid-cols-3 gap-2">
            <label className="sm:col-span-2 space-y-1">
              <span className="text-[11px] text-ink-soft">Repository URL or owner/repo</span>
              <input
                type="text"
                required
                disabled={busy}
                value={gitUrl}
                onChange={(e) => setGitUrl(e.target.value)}
                placeholder="owner/repo or https://github.com/owner/repo/tree/main/skills/name"
                className={fieldClass}
              />
            </label>
            <label className="space-y-1">
              <span className="text-[11px] text-ink-soft">Subpath (optional)</span>
              <input
                type="text"
                disabled={busy}
                value={gitSubpath}
                onChange={(e) => setGitSubpath(e.target.value)}
                placeholder="skills/my-skill"
                className={fieldClass}
              />
            </label>
          </div>
          <div className="flex justify-end gap-2">
            <button
              type="button"
              disabled={busy}
              onClick={() => setInstallMode("none")}
              className={actionClass}
            >
              Cancel
            </button>
            <button
              type="submit"
              disabled={busy || !gitUrl.trim()}
              className="inline-flex items-center gap-1.5 rounded-lg bg-brand hover:bg-brand-hover px-3 py-1.5 font-medium text-white disabled:opacity-50"
            >
              {busy && <Loader2 className="h-3.5 w-3.5 animate-spin" />}
              <span>Clone &amp; Install</span>
            </button>
          </div>
        </form>
      )}

      {/* New Custom Skill Form */}
      {installMode === "custom" && (
        <form
          onSubmit={(e) => {
            e.preventDefault();
            void handleCreateCustom();
          }}
          className="settings-card space-y-2.5"
        >
          <div className="font-medium text-ink">Create Custom Skill</div>
          <div className="grid grid-cols-1 sm:grid-cols-2 gap-2">
            <label className="space-y-1">
              <span className="text-[11px] text-ink-soft">Display Name</span>
              <input
                type="text"
                required
                disabled={busy}
                value={customName}
                onChange={(e) => setCustomName(e.target.value)}
                placeholder="Rust Security Reviewer"
                className={fieldClass}
              />
            </label>
            <label className="space-y-1">
              <span className="text-[11px] text-ink-soft">Slash Command Slug (optional)</span>
              <input
                type="text"
                disabled={busy}
                value={customId}
                onChange={(e) => setCustomId(e.target.value)}
                placeholder="rust-security"
                className={`${fieldClass} font-mono`}
              />
            </label>
          </div>
          <label className="block space-y-1">
            <span className="text-[11px] text-ink-soft">
              Description (guides agents on when to activate)
            </span>
            <input
              type="text"
              required
              disabled={busy}
              value={customDesc}
              onChange={(e) => setCustomDesc(e.target.value)}
              placeholder="Audit async Rust code for path traversal, lock ordering, and panics."
              className={fieldClass}
            />
          </label>
          <label className="block space-y-1">
            <span className="text-[11px] text-ink-soft">SKILL.md Instructions</span>
            <textarea
              rows={6}
              disabled={busy}
              value={customBody}
              onChange={(e) => setCustomBody(e.target.value)}
              className={`${fieldClass} font-mono text-[11px]`}
            />
          </label>
          <div className="flex justify-end gap-2">
            <button
              type="button"
              disabled={busy}
              onClick={() => setInstallMode("none")}
              className={actionClass}
            >
              Cancel
            </button>
            <button
              type="submit"
              disabled={busy || !customName.trim() || !customDesc.trim()}
              className="inline-flex items-center gap-1.5 rounded-lg bg-brand hover:bg-brand-hover px-3 py-1.5 font-medium text-white disabled:opacity-50"
            >
              <Sparkles className="h-3.5 w-3.5" />
              <span>Create Skill</span>
            </button>
          </div>
        </form>
      )}

      {/* Skill List */}
      <div className="space-y-2.5">
        {skills.map((skill) => {
          const badge = sourceLabel(skill.source);
          const isExpanded = expandedId === skill.id;
          const isProject = skill.source === "project";
          const canDelete = !isProject && skill.source !== "builtin";
          return (
            <article
              key={`${skill.source}:${skill.id}`}
              className="settings-card space-y-2"
            >
              <div className="flex items-start justify-between gap-3">
                <div className="min-w-0 flex-1 space-y-1">
                  <div className="flex flex-wrap items-center gap-2">
                    <h3 className="font-semibold text-sm text-ink">{skill.name}</h3>
                    <span className="rounded-md bg-skill-soft border border-skill/35 px-1.5 py-0.2 font-mono text-[11px] font-semibold text-skill">
                      /{skill.id}
                    </span>
                    <span className={`rounded-full border px-2 py-0.2 text-[10px] font-medium ${badge.cls}`}>
                      {badge.text}
                    </span>
                    {skill.builtin_modified && (
                      <span className="rounded-full border border-warn/30 bg-warn-soft px-2 py-0.2 text-[10px] font-medium text-warn">
                        Modified
                      </span>
                    )}
                  </div>
                  <p className="text-ink-soft leading-relaxed">{skill.description}</p>
                  <div className="flex flex-wrap items-center gap-x-3 gap-y-1 text-[10.5px] text-ink-muted">
                    <span>
                      {skill.files.length} file{skill.files.length === 1 ? "" : "s"}
                    </span>
                    {skill.origin && skill.origin !== "embedded" && (
                      <span className="truncate max-w-sm font-mono" title={skill.origin}>
                        {skill.origin}
                      </span>
                    )}
                  </div>
                </div>

                <div className="flex items-center gap-1.5 shrink-0">
                  <button
                    type="button"
                    role="switch"
                    aria-checked={skill.enabled}
                    aria-label={`Enable skill ${skill.name}`}
                    disabled={busy}
                    onClick={() => void runAction(() => toggleSkill(skill.id, !skill.enabled))}
                    className={`relative inline-flex h-5 w-9 shrink-0 items-center rounded-full border transition-colors ${
                      skill.enabled
                        ? "bg-brand border-brand"
                        : "bg-inset border-line-strong"
                    }`}
                  >
                    <span
                      className={`inline-block h-3.5 w-3.5 transform rounded-full bg-white transition-transform ${
                        skill.enabled ? "translate-x-4" : "translate-x-0.5"
                      }`}
                    />
                  </button>
                </div>
              </div>

              <div className="flex flex-wrap items-center justify-between gap-2 pt-1 border-t border-line/70">
                <button
                  type="button"
                  disabled={busy}
                  onClick={() => toggleExpand(skill)}
                  className="inline-flex items-center gap-1 text-[11px] font-medium text-brand-ink hover:underline"
                >
                  {isExpanded ? (
                    <ChevronDown className="h-3.5 w-3.5" />
                  ) : (
                    <ChevronRight className="h-3.5 w-3.5" />
                  )}
                  <span>{isExpanded ? "Hide Files" : "Inspect / Edit Files"}</span>
                </button>

                <div className="flex items-center gap-2">
                  {skill.source === "builtin" && skill.builtin_modified && (
                    <button
                      type="button"
                      disabled={busy}
                      onClick={() => void handleResetBuiltin(skill.id)}
                      className="inline-flex items-center gap-1 rounded-lg border border-line bg-inset px-2 py-1 text-[11px] text-ink-soft hover:bg-hover hover:text-ink"
                    >
                      <RotateCcw className="h-3 w-3" />
                      <span>Reset to Built-in</span>
                    </button>
                  )}

                  {canDelete &&
                    (confirmUninstallId === skill.id ? (
                      <div className="inline-flex items-center gap-1.5">
                        <button
                          type="button"
                          onClick={() => setConfirmUninstallId(null)}
                          className="rounded px-2 py-0.5 text-[11px] text-ink-muted hover:bg-hover"
                        >
                          Cancel
                        </button>
                        <button
                          type="button"
                          disabled={busy}
                          onClick={() => void handleUninstall(skill.id)}
                          className="rounded bg-neg px-2 py-0.5 text-[11px] font-medium text-white"
                        >
                          Confirm Uninstall
                        </button>
                      </div>
                    ) : (
                      <button
                        type="button"
                        aria-label={`Uninstall skill ${skill.name}`}
                        disabled={busy}
                        onClick={() => setConfirmUninstallId(skill.id)}
                        className="inline-flex items-center gap-1 rounded-lg p-1 text-ink-muted hover:bg-neg-soft hover:text-neg"
                      >
                        <Trash2 className="h-3.5 w-3.5" />
                      </button>
                    ))}
                </div>
              </div>

              {isExpanded && (
                <div className="settings-card space-y-3">
                  <div className="flex flex-wrap items-center gap-1.5 border-b border-line pb-2">
                    {skill.files.map((f) => (
                      <button
                        key={f}
                        type="button"
                        disabled={busy}
                        onClick={() => void openDetailFile(skill, f)}
                        className={`rounded-md px-2 py-1 font-mono text-[10.5px] transition-colors ${
                          detail?.file_path === f
                            ? "bg-brand-soft text-brand-ink font-semibold"
                            : "text-ink-muted hover:bg-hover hover:text-ink"
                        }`}
                      >
                        {f}
                      </button>
                    ))}
                  </div>

                  {detailLoading ? (
                    <div className="py-6 text-center text-ink-muted">Loading file…</div>
                  ) : detail && detail.skill.id === skill.id ? (
                    <>
                      <textarea
                        rows={11}
                        readOnly={isProject || busy}
                        value={draftContent}
                        onChange={(e) => setDraftContent(e.target.value)}
                        className="w-full rounded-lg border border-line bg-app p-2.5 font-mono text-[11px] text-ink leading-relaxed focus:outline-none focus:border-brand"
                      />
                      {!isProject && (
                        <div className="flex items-center justify-between gap-2">
                          <span className="truncate font-mono text-[10px] text-ink-muted">
                            {detail.skill.skill_dir}/{detail.file_path}
                          </span>
                          <button
                            type="button"
                            disabled={busy || draftContent === detail.content}
                            onClick={() => void handleSaveContent()}
                            className="inline-flex items-center gap-1 rounded-lg bg-brand hover:bg-brand-hover px-3 py-1 font-medium text-white disabled:opacity-40"
                          >
                            <Save className="h-3 w-3" />
                            <span>Save</span>
                          </button>
                        </div>
                      )}
                    </>
                  ) : null}
                </div>
              )}
            </article>
          );
        })}

        {!skills.length && !skillsLoading && (
          <p className="rounded-xl border border-line bg-inset p-4 text-center text-ink-muted">
            No skills found.
          </p>
        )}
      </div>

      {folderPickerOpen && (
        <FolderPickerModal
          onClose={() => setFolderPickerOpen(false)}
          onSelect={handleInstallFolder}
        />
      )}
    </section>
  );
}
